//! Native candidate prompts and strict chat streaming for isolated live-model tests.

use std::sync::LazyLock;

use futures_util::StreamExt;
use serde_json::{Value, json};

use crate::{LlmClient, LlmError, Message, Result};

static HTTP: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);

pub(crate) fn sub_messages(source: &str, turns: &[(String, String)]) -> Vec<Message> {
    // The publisher trained these separate context blocks; no invented speaker or glossary.
    let system = "TASK: Translate Korean subtitles into English.\nSTYLE: neutral.\nTranslate only CURRENT_SOURCE. PREVIOUS_SOURCE and PREVIOUS_TRANSLATION are context only. Preserve meaning, tone, slang, profanity, uncertainty, repetitions and incomplete speech. Return only the final translation without labels or commentary.";
    let current = format!("[CURRENT_SOURCE]\n{source}");
    let user = if turns.is_empty() {
        current
    } else {
        let previous_sources = turns.iter().map(|(ko, _)| ko.as_str()).collect::<Vec<_>>().join("\n");
        let previous_translations = turns.iter().map(|(_, en)| en.as_str()).collect::<Vec<_>>().join("\n");
        format!("[PREVIOUS_SOURCE]\n{previous_sources}\n\n[PREVIOUS_TRANSLATION]\n{previous_translations}\n\n{current}")
    };
    vec![Message::system(system), Message::user(user)]
}

pub(crate) fn lmt_messages(source: &str, turns: &[(String, String)]) -> Vec<Message> {
    // The single-source instruction is native; preceding chat pairs are experimental.
    let prompt = |ko: &str| format!("Translate the following text from Korean into English:\nKorean: {ko}\nEnglish:");
    let mut messages = Vec::new();
    for (ko, en) in turns {
        messages.push(Message::user(prompt(ko)));
        messages.push(Message::assistant(en));
    }
    messages.push(Message::user(prompt(source)));
    messages
}

#[derive(Default)]
struct Parser {
    buffer: Vec<u8>,
    done: bool,
}

impl Parser {
    fn push(&mut self, chunk: &[u8], delta: &mut (dyn FnMut(&str) + Send)) -> Result<()> {
        self.buffer.extend_from_slice(chunk);
        while let Some(end) = self.buffer.iter().position(|&b| b == b'\n') {
            let line: Vec<_> = self.buffer.drain(..=end).collect();
            self.line(&line, delta)?;
        }
        Ok(())
    }

    fn line(&mut self, line: &[u8], delta: &mut (dyn FnMut(&str) + Send)) -> Result<()> {
        let line = String::from_utf8_lossy(line);
        let Some(payload) = line.trim().strip_prefix("data:").map(str::trim) else { return Ok(()) };
        if payload == "[DONE]" {
            self.done = true;
            return Ok(());
        }
        let value: Value = serde_json::from_str(payload).map_err(|e| LlmError::Stream(format!("invalid diagnostic chat event: {e}")))?;
        if let Some(error) = value.get("error") { return Err(LlmError::Api(error.to_string())); }
        if let Some(text) = value.pointer("/choices/0/delta/content").and_then(Value::as_str) { delta(text); }
        if let Some(reason) = value.pointer("/choices/0/finish_reason").and_then(Value::as_str).filter(|r| *r != "stop") {
            return Err(LlmError::Api(format!("diagnostic chat ended with {reason}; inspect the preserved stream")));
        }
        Ok(())
    }
}

pub(crate) async fn stream(client: &LlmClient, source: &str, messages: &[Message], delta: &mut (dyn FnMut(&str) + Send)) -> Result<String> {
    let root = client.root();
    let response = HTTP.post(format!("{root}/v1/chat/completions")).json(&json!({
        "model": client.model_name(), "messages": messages, "stream": true,
        "temperature": 0.0, "top_k": 1, "max_tokens": (source.chars().count() as u32 * 2 + 40).min(256),
        "cache_prompt": true, "chat_template_kwargs": { "enable_thinking": false },
        "stream_options": { "include_usage": true },
    })).send().await.map_err(|source| LlmError::Unreachable { url: root.into(), source })?;
    if !response.status().is_success() {
        let status = response.status().as_u16();
        let body = response.text().await.unwrap_or_default().chars().take(400).collect();
        return Err(LlmError::Http { status, body });
    }
    let mut parser = Parser::default();
    let mut chunks = response.bytes_stream();
    let mut full = String::new();
    let mut receive = |text: &str| { full.push_str(text); delta(text); };
    while let Some(chunk) = chunks.next().await {
        parser.push(&chunk.map_err(|e| LlmError::Stream(e.to_string()))?, &mut receive)?;
        if parser.done { return Ok(full.trim().to_owned()); }
    }
    let rest = std::mem::take(&mut parser.buffer);
    parser.line(&rest, &mut receive)?;
    if !parser.done { return Err(LlmError::Stream("diagnostic chat ended without its final event".into())); }
    Ok(full.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use maru_core::config::LlmConfig;
    use crate::testing::{MockResponse, spawn_mock};

    #[test]
    fn subtitle_context_keeps_sources_and_answers_aligned_and_the_current_source_unanswered() {
        let turns = vec![("이걸 타면\n위로 갈 수 있어요.".into(), "We can ride this up.".into()), ("네.".into(), "Yes.".into())];
        let body = json!(sub_messages("위로?", &turns));
        assert_eq!(body[0]["role"], "system");
        assert!(body[0]["content"].as_str().unwrap().contains("STYLE: neutral."));
        assert_eq!(body[1]["content"], "[PREVIOUS_SOURCE]\n이걸 타면\n위로 갈 수 있어요.\n네.\n\n[PREVIOUS_TRANSLATION]\nWe can ride this up.\nYes.\n\n[CURRENT_SOURCE]\n위로?");
        assert_eq!(json!(sub_messages("위로?", &[]))[1]["content"], "[CURRENT_SOURCE]\n위로?");
    }

    #[test]
    fn lmt_uses_the_published_instruction_without_a_system_and_marks_context_as_completed_chat_pairs() {
        let turns = vec![("갈까요?".into(), "Shall we go?".into())];
        let body = json!(lmt_messages("가지 마세요.", &turns));
        assert_eq!(body[0]["role"], "user");
        assert_eq!(body[0]["content"], "Translate the following text from Korean into English:\nKorean: 갈까요?\nEnglish:");
        assert_eq!(body[1]["role"], "assistant");
        assert_eq!(body[1]["content"], "Shall we go?");
        assert_eq!(body[2]["content"], "Translate the following text from Korean into English:\nKorean: 가지 마세요.\nEnglish:");
        assert_eq!(lmt_messages("가지 마세요.", &[]).len(), 1);
    }

    #[tokio::test]
    async fn diagnostic_chat_is_greedy_and_preserves_split_utf8_and_unwanted_labels_for_review() {
        let event = "data: {\"choices\":[{\"delta\":{\"content\":\"English: 안녕\"},\"finish_reason\":null}]}\n\ndata: [DONE]\n\n".as_bytes().to_vec();
        let split = event.iter().position(|&b| b >= 128).unwrap() + 1;
        let mock = spawn_mock(move |_, _| MockResponse { status: 200, chunks: vec![event[..split].to_vec(), event[split..].to_vec()], delay_ms: 0 }).await;
        let client = LlmClient::new(&mock.url, &LlmConfig { model_name: "test-alias".into(), ..Default::default() });
        let mut deltas = String::new();
        assert_eq!(stream(&client, "위로?", &sub_messages("위로?", &[]), &mut |s| deltas.push_str(s)).await.unwrap(), "English: 안녕");
        assert_eq!(deltas, "English: 안녕");
        let requests = mock.requests.lock().unwrap();
        assert_eq!(requests[0].0, "/v1/chat/completions");
        let body: Value = serde_json::from_str(&requests[0].1).unwrap();
        assert_eq!(body["model"], "test-alias");
        assert_eq!(body["temperature"], 0.0);
        assert_eq!(body["top_k"], 1);
        assert_eq!(body["max_tokens"], 46);
        assert_eq!(body["chat_template_kwargs"]["enable_thinking"], false);
    }

    #[tokio::test]
    async fn capped_incomplete_and_invalid_chat_outputs_are_errors_with_partial_text_retained() {
        for event in ["data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n", "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"},\"finish_reason\":\"length\"}]}\n\ndata: [DONE]\n", "data: not-json\n", "data: {\"error\":\"failed\"}\n"] {
            let mock = spawn_mock(move |_, _| MockResponse { status: 200, chunks: vec![event.as_bytes().to_vec()], delay_ms: 0 }).await;
            let client = LlmClient::new(&mock.url, &LlmConfig::default());
            let mut deltas = String::new();
            assert!(stream(&client, "위로?", &sub_messages("위로?", &[]), &mut |s| deltas.push_str(s)).await.is_err());
            if event.contains("partial") { assert_eq!(deltas, "partial"); }
        }
    }
}
