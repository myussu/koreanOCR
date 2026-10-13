//! Plain completions for the MiLMMT diagnostic; production tasks still use chat.

use std::sync::LazyLock;

use futures_util::StreamExt;
use serde_json::{Value, json};

use crate::{LlmError, Result};

static HTTP: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);

#[derive(Default)]
struct Parser {
    buffer: Vec<u8>,
    stopped: bool,
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
            self.stopped = true;
            return Ok(());
        }
        let value: Value = serde_json::from_str(payload).map_err(|e| LlmError::Stream(format!("invalid completion event: {e}")))?;
        if let Some(error) = value.get("error") {
            return Err(LlmError::Api(error.to_string()));
        }
        if let Some(content) = value.get("content").and_then(Value::as_str) {
            delta(content);
        }
        self.stopped |= value.get("stop").and_then(Value::as_bool).unwrap_or(false);
        if value.get("truncated").and_then(Value::as_bool) == Some(true)
            || value.get("stop_type").and_then(Value::as_str) == Some("limit") {
            return Err(LlmError::Api("MiLMMT reached the live output cap; inspect the preserved stream".into()));
        }
        Ok(())
    }
}

fn prompt(source: &str, turns: &[(String, String)]) -> String {
    // Completed examples are an experimental extension of Xiaomi's plain prompt.
    let mut prompt = String::new();
    for (ko, en) in turns {
        prompt.push_str(&format!("Translate this from Korean to English:\nKorean: {ko}\nEnglish: {en}\n\n"));
    }
    prompt.push_str(&format!("Translate this from Korean to English:\nKorean: {source}\nEnglish:"));
    prompt
}

pub(crate) async fn stream(root: &str, source: &str, turns: &[(String, String)], delta: &mut (dyn FnMut(&str) + Send)) -> Result<String> {
    let prompt = prompt(source, turns);
    let response = HTTP.post(format!("{root}/completion")).json(&json!({
        "prompt": prompt, "stream": true, "temperature": 0.0, "top_k": 1,
        "n_predict": (source.chars().count() as u32 * 2 + 40).min(256), "cache_prompt": true,
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
        if parser.stopped { return Ok(full.trim().to_owned()); }
    }
    let rest = std::mem::take(&mut parser.buffer);
    parser.line(&rest, &mut receive)?;
    if !parser.stopped { return Err(LlmError::Stream("completion stream ended without its final event".into())); }
    Ok(full.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{MockResponse, spawn_mock};

    #[tokio::test]
    async fn the_milmmt_prompt_is_a_plain_completion_with_greedy_decoding() {
        let event = "data: {\"content\":\"Don't go. 안녕\",\"stop\":false}\n\ndata: {\"content\":\"\",\"stop\":true}\n\n".as_bytes().to_vec();
        let split = event.iter().position(|&b| b >= 128).unwrap() + 1;
        let server = spawn_mock(move |_, _| MockResponse { status: 200, chunks: vec![event[..split].to_vec(), event[split..].to_vec()], delay_ms: 0 }).await;
        let mut deltas = String::new();
        assert_eq!(stream(&server.url, "가지 마세요.", &[], &mut |s| deltas.push_str(s)).await.unwrap(), "Don't go. 안녕");
        assert_eq!(deltas, "Don't go. 안녕");
        let requests = server.requests.lock().unwrap();
        assert_eq!(requests[0].0, "/completion");
        let body: Value = serde_json::from_str(&requests[0].1).unwrap();
        assert_eq!(body["prompt"], "Translate this from Korean to English:\nKorean: 가지 마세요.\nEnglish:");
        assert_eq!(body["temperature"], 0.0);
        assert_eq!(body["top_k"], 1);
        assert_eq!(body["n_predict"], 54);
        assert!(body.get("messages").is_none());
    }

    #[tokio::test]
    async fn an_incomplete_or_capped_or_invalid_completion_is_refused() {
        for event in ["data: {\"content\":\"partial\"}\n", "data: {\"content\":\"partial\",\"stop\":true,\"stop_type\":\"limit\"}\n", "data: not-json\n", "data: {\"error\":\"failed\"}\n"] {
            let server = spawn_mock(move |_, _| MockResponse { status: 200, chunks: vec![event.as_bytes().to_vec()], delay_ms: 0 }).await;
            assert!(stream(&server.url, "네", &[], &mut |_| {}).await.is_err());
        }
    }

    #[tokio::test]
    async fn previous_translations_are_completed_examples_and_only_the_current_answer_is_generated() {
        let server = spawn_mock(|_, _| MockResponse { status: 200, chunks: vec![b"data: {\"content\":\"Up?\",\"stop\":true}\n\n".to_vec()], delay_ms: 0 }).await;
        let turns = vec![("이걸 타면\n위로 갈 수 있어요.".into(), "We can ride this up.".into()), ("네.".into(), "Yes.".into())];
        assert_eq!(stream(&server.url, "위로?", &turns, &mut |_| {}).await.unwrap(), "Up?");
        let requests = server.requests.lock().unwrap();
        let body: Value = serde_json::from_str(&requests[0].1).unwrap();
        assert_eq!(body["prompt"], "Translate this from Korean to English:\nKorean: 이걸 타면\n위로 갈 수 있어요.\nEnglish: We can ride this up.\n\nTranslate this from Korean to English:\nKorean: 네.\nEnglish: Yes.\n\nTranslate this from Korean to English:\nKorean: 위로?\nEnglish:");
        assert_eq!(body["n_predict"], 46);
        assert_eq!(body["temperature"], 0.0);
        assert_eq!(body["top_k"], 1);
        assert!(body.get("messages").is_none());
    }
}
