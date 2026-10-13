//! Native Korean-to-English translation for the optional live models.

use crate::{LiveRequest, LlmChunk, LlmClient, LlmError, Result};
use futures_util::StreamExt;
use maru_core::{event::Variant, live_model::LiveModel};
use serde_json::{Value, json};
use std::sync::LazyLock;
use std::time::Duration;

static HTTP: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);

fn request(model: LiveModel, alias: &str, source: &str, turns: &[(String, String)]) -> (&'static str, Value) {
    let cap = source.chars().count().saturating_mul(2).saturating_add(40).min(256);
    match model {
        LiveModel::Milmmt => {
            let mut prompt = String::new();
            for (ko, en) in turns {
                prompt.push_str(&format!("Translate this from Korean to English:\nKorean: {ko}\nEnglish: {en}\n\n"));
            }
            prompt.push_str(&format!("Translate this from Korean to English:\nKorean: {source}\nEnglish:"));
            (
                "/completion",
                json!({ "prompt": prompt, "stream": true, "temperature": 0.0, "top_k": 1, "n_predict": cap, "cache_prompt": true }),
            )
        }
        LiveModel::Lfm => {
            let mut messages = vec![json!({ "role": "system", "content": "Translate the following text to English." })];
            for (ko, en) in turns {
                messages.push(json!({ "role": "user", "content": ko }));
                messages.push(json!({ "role": "assistant", "content": en }));
            }
            messages.push(json!({ "role": "user", "content": source }));
            ("/v1/chat/completions", json!({ "model": alias, "messages": messages, "stream": true, "temperature": 0.3, "max_tokens": cap, "cache_prompt": true }))
        }
        _ => {
            let prompt = if turns.is_empty() {
                format!(
                    "Translate the following text into English. Note that you should only output the translated result without any additional explanation:\n{source}"
                )
            } else {
                let background = turns.iter().map(|(ko, en)| format!("Korean: {ko}\nEnglish: {en}\n")).collect::<String>();
                format!(
                    "[Background Information]\n{background}\nPlease translate the following text into English, taking the provided background information into consideration.\n[Source Text]\n{source}"
                )
            };
            (
                "/v1/chat/completions",
                json!({ "model": alias, "messages": [{ "role": "user", "content": prompt }], "stream": true,
                "temperature": 0.7, "top_p": 0.6, "top_k": 20, "repeat_penalty": 1.05, "max_tokens": cap,
                "cache_prompt": true, "chat_template_kwargs": {"enable_thinking": false} }),
            )
        }
    }
}

#[derive(Default)]
struct Parser {
    buffer: Vec<u8>,
    done: bool,
}

impl Parser {
    fn push(&mut self, bytes: &[u8], receive: &mut (dyn FnMut(&str) + Send)) -> Result<()> {
        self.buffer.extend_from_slice(bytes);
        while let Some(end) = self.buffer.iter().position(|&b| b == b'\n') {
            let line: Vec<_> = self.buffer.drain(..=end).collect();
            self.line(&line, receive)?;
        }
        Ok(())
    }
    fn line(&mut self, line: &[u8], receive: &mut (dyn FnMut(&str) + Send)) -> Result<()> {
        let line = std::str::from_utf8(line).map_err(|e| LlmError::Stream(e.to_string()))?;
        let Some(payload) = line.trim().strip_prefix("data:").map(str::trim) else {
            return Ok(());
        };
        if payload == "[DONE]" {
            self.done = true;
            return Ok(());
        }
        let value: Value = serde_json::from_str(payload).map_err(|e| LlmError::Stream(e.to_string()))?;
        if let Some(error) = value.get("error") {
            return Err(LlmError::Api(error.to_string()));
        }
        if value.get("truncated").and_then(Value::as_bool) == Some(true)
            || value.get("stop_type").and_then(Value::as_str) == Some("limit")
            || value.pointer("/choices/0/finish_reason").and_then(Value::as_str).is_some_and(|r| r != "stop")
        {
            return Err(LlmError::Api("translation stopped before finishing; try a shorter snapshot".into()));
        }
        if let Some(text) = value
            .get("content")
            .or_else(|| value.pointer("/choices/0/delta/content"))
            .and_then(Value::as_str)
        {
            receive(text);
        }
        self.done |= value.get("stop").and_then(Value::as_bool) == Some(true);
        Ok(())
    }
}

impl LlmClient {
    pub async fn translate_selected_live(&self, model: LiveModel, req: &LiveRequest<'_>, sink: &mut (dyn FnMut(LlmChunk) + Send)) -> Result<()> {
        if model == LiveModel::Main {
            return self.translate_live(req, sink).await.map(|_| ());
        }
        let _lease = self.model_lease().await?;
        let (route, body) = request(model, self.model_name(), req.text, req.turns);
        let root = self.root();
        let timeout = Duration::from_secs(60);
        let response = HTTP
            .post(format!("{root}{route}"))
            .json(&body)
            .timeout(timeout)
            .send()
            .await
            .map_err(|source| LlmError::Unreachable { url: root.into(), source })?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default().chars().take(400).collect();
            return Err(LlmError::Http { status, body });
        }
        let mut parser = Parser::default();
        let mut stream = response.bytes_stream();
        let mut full = String::new();
        let mut receive = |s: &str| {
            full.push_str(s);
            sink(LlmChunk::Text {
                variant: Variant::Primary,
                delta: s.into(),
            });
        };
        while let Some(chunk) = stream.next().await {
            parser.push(&chunk.map_err(|e| LlmError::Stream(e.to_string()))?, &mut receive)?;
            if parser.done {
                break;
            }
        }
        if !parser.done {
            let rest = std::mem::take(&mut parser.buffer);
            parser.line(&rest, &mut receive)?;
        }
        if !parser.done {
            return Err(LlmError::Stream("translation ended without its final event".into()));
        }
        if full.trim().is_empty() {
            return Err(LlmError::Api("the live model returned an empty translation".into()));
        }
        sink(LlmChunk::TextDone {
            variant: Variant::Primary,
            text: full.trim().into(),
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        PromptContext,
        testing::{MockResponse, spawn_mock},
    };
    use maru_core::config::LlmConfig;

    #[tokio::test]
    async fn a_snapshot_deadline_ends_a_stalled_quality_request() {
        let mock = spawn_mock(|_, _| MockResponse {
            status: 200,
            chunks: crate::testing::sse_answer(&["slow"]),
            delay_ms: 200,
        })
        .await;
        let client = LlmClient::new(&mock.url, &LlmConfig::default()).with_request_deadline(Duration::from_millis(25));
        let start = std::time::Instant::now();
        assert!(
            client
                .stream_chat(&[crate::Message::user("test")], &crate::ChatOpts::default(), &mut |_| {})
                .await
                .is_err()
        );
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn native_dialogue_context_uses_completed_pairs_and_the_exact_mt2_background_template() {
        let turns = vec![("네.".into(), "Yes.".into())];
        let (_, hy) = request(LiveModel::HyMt2, "hy-mt2", "왜요?", &turns);
        assert_eq!(
            hy["messages"][0]["content"],
            "[Background Information]\nKorean: 네.\nEnglish: Yes.\n\nPlease translate the following text into English, taking the provided background information into consideration.\n[Source Text]\n왜요?"
        );
        let (_, mi) = request(LiveModel::Milmmt, "milmmt", "왜요?", &turns);
        assert_eq!(
            mi["prompt"],
            "Translate this from Korean to English:\nKorean: 네.\nEnglish: Yes.\n\nTranslate this from Korean to English:\nKorean: 왜요?\nEnglish:"
        );
        assert_eq!(mi["temperature"], 0.0);
        assert_eq!(hy["top_p"], 0.6);
    }

    #[tokio::test]
    async fn native_models_stream_translation_without_tutoring_prompts() {
        for model in [LiveModel::Milmmt, LiveModel::HyMt2, LiveModel::HyMt2Small, LiveModel::Lfm] {
            let event = if model == LiveModel::Milmmt {
                "data: {\"content\":\"Yes. 안녕\",\"stop\":true}\n\n"
            } else {
                "data: {\"choices\":[{\"delta\":{\"content\":\"Yes. 안녕\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n"
            };
            let bytes = event.as_bytes().to_vec();
            let split = bytes.iter().position(|&b| b >= 128).unwrap() + 1;
            let mock = spawn_mock(move |_, _| MockResponse {
                status: 200,
                chunks: vec![bytes[..split].to_vec(), bytes[split..].to_vec()],
                delay_ms: 0,
            })
            .await;
            let client = LlmClient::new(&mock.url, &LlmConfig::default());
            let mut final_text = String::new();
            client
                .translate_selected_live(
                    model,
                    &LiveRequest {
                        text: "네.",
                        ctx: &PromptContext::default(),
                        turns: &[],
                    },
                    &mut |c| {
                        if let LlmChunk::TextDone { text, .. } = c {
                            final_text = text;
                        }
                    },
                )
                .await
                .unwrap();
            assert_eq!(final_text, "Yes. 안녕");
            let requests = mock.requests.lock().unwrap();
            let body: Value = serde_json::from_str(&requests[0].1).unwrap();
            assert_eq!(requests[0].0, if model == LiveModel::Milmmt { "/completion" } else { "/v1/chat/completions" });
            assert_eq!(body[if model == LiveModel::Milmmt { "n_predict" } else { "max_tokens" }], 44);
            assert_eq!(body["temperature"], if model == LiveModel::Milmmt { 0.0 } else if model == LiveModel::Lfm { 0.3 } else { 0.7 });
            if model == LiveModel::Lfm {
                assert_eq!(body["messages"][0]["content"], "Translate the following text to English.");
                assert_eq!(body["messages"][1]["content"], "네.");
            }
        }
    }

    #[tokio::test]
    async fn invalid_capped_empty_and_incomplete_streams_never_complete_a_translation() {
        for event in [
            "data: nope\n",
            "data: {\"content\":\"partial\"}\n",
            "data: {\"content\":\"partial\",\"stop\":true,\"stop_type\":\"limit\"}\n",
            "data: [DONE]\n",
        ] {
            let mock = spawn_mock(move |_, _| MockResponse {
                status: 200,
                chunks: vec![event.as_bytes().to_vec()],
                delay_ms: 0,
            })
            .await;
            let client = LlmClient::new(&mock.url, &LlmConfig::default());
            let mut completed = false;
            assert!(
                client
                    .translate_selected_live(
                        LiveModel::Milmmt,
                        &LiveRequest {
                            text: "네",
                            ctx: &PromptContext::default(),
                            turns: &[]
                        },
                        &mut |c| completed |= matches!(c, LlmChunk::TextDone { .. })
                    )
                    .await
                    .is_err()
            );
            assert!(!completed);
        }
    }
}
