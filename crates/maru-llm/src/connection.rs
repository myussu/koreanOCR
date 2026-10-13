//! A bounded, fixed-text check of an external model's chat API.

use std::time::{Duration, Instant};

use maru_core::config::LlmConfig;
use serde::Serialize;

use crate::{ChatOpts, LlmClient, Message};

#[derive(Debug, Serialize)]
pub struct ConnectionReply {
    pub endpoint: String,
    pub reply: String,
    pub took_ms: u64,
}

pub async fn test_connection(endpoint: &str, model_name: &str, reasoning_effort: &str) -> Result<ConnectionReply, String> {
    check(endpoint, model_name, reasoning_effort, Duration::from_secs(60)).await
}

async fn check(endpoint: &str, model_name: &str, reasoning_effort: &str, deadline: Duration) -> Result<ConnectionReply, String> {
    let endpoint = crate::discovery::normalize(endpoint)?;
    let url = reqwest::Url::parse(&endpoint).map_err(|_| "Enter a complete server address, such as http://192.168.1.20:11434.".to_string())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("The server address must start with http:// or https://.".into());
    }
    if model_name.trim().is_empty() {
        return Err("Enter the model name expected by the server.".into());
    }
    let cfg = LlmConfig {
        endpoint: Some(endpoint.clone()), model_name: model_name.trim().into(),
        reasoning_effort: reasoning_effort.into(), request_timeout_secs: 60,
        ..Default::default()
    };
    // No runtime is attached: checking a draft must never start a local model.
    let client = LlmClient::from_config(&cfg);
    let messages = [Message::user("Translate this Korean sentence into English. Return only the translation:\n안녕하세요.")];
    let opts = ChatOpts { max_tokens: Some(64), temperature: Some(0.0), ..Default::default() };
    let start = Instant::now();
    let reply = tokio::time::timeout(deadline, client.stream_chat(&messages, &opts, &mut |_| {}))
        .await.map_err(|_| "The connection test timed out. The server may still be loading the model; check it and try again.".to_string())?
        .map_err(|e| e.to_string())?;
    if reply.trim().is_empty() {
        return Err("The server responded but returned no translation. Check the model name and its streaming chat API.".into());
    }
    Ok(ConnectionReply { endpoint, reply: reply.trim().chars().take(300).collect(), took_ms: start.elapsed().as_millis() as u64 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{MockResponse, spawn_mock, sse_answer};

    #[tokio::test]
    async fn a_draft_server_and_model_are_checked_with_fixed_text_and_foreign_server_options() {
        let server = spawn_mock(|_, _| MockResponse { status: 200, chunks: sse_answer(&["Hello", "."]), delay_ms: 0 }).await;
        let reply = test_connection(&format!(" {}/v1/ ", server.url), " draft-model ", "low").await.unwrap();
        assert_eq!(reply.reply, "Hello.");
        let requests = server.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].0, "/v1/chat/completions");
        let body: serde_json::Value = serde_json::from_str(&requests[0].1).unwrap();
        assert_eq!(body["model"], "draft-model");
        assert_eq!(body["reasoning_effort"], "low");
        assert!(body["messages"][0]["content"].as_str().unwrap().contains("안녕하세요."));
        assert!(body.get("cache_prompt").is_none());
    }

    #[tokio::test]
    async fn a_reachable_server_without_an_answer_does_not_pass() {
        let server = spawn_mock(|_, _| MockResponse { status: 200, chunks: sse_answer(&[]), delay_ms: 0 }).await;
        assert!(test_connection(&server.url, "maru", "").await.unwrap_err().contains("no translation"));
    }

    #[tokio::test]
    async fn a_missing_model_preserves_the_server_error() {
        let server = spawn_mock(|_, _| MockResponse { status: 404, chunks: vec![b"model not found".to_vec()], delay_ms: 0 }).await;
        assert!(test_connection(&server.url, "missing", "none").await.unwrap_err().contains("model not found"));
    }

    #[tokio::test]
    async fn a_slow_response_is_bounded_even_when_it_has_started_streaming() {
        let server = spawn_mock(|_, _| MockResponse { status: 200, chunks: sse_answer(&["Hello"]), delay_ms: 500 }).await;
        assert!(check(&server.url, "maru", "", Duration::from_millis(80)).await.unwrap_err().contains("timed out"));
    }

    #[tokio::test]
    async fn an_invalid_address_or_empty_model_is_refused_before_sending() {
        assert!(test_connection("file:///model", "maru", "").await.is_err());
        assert!(test_connection("", "maru", "").await.is_err());
        let server = spawn_mock(|_, _| MockResponse { status: 200, chunks: vec![], delay_ms: 0 }).await;
        assert!(test_connection(&server.url, " ", "").await.is_err());
        assert!(server.requests.lock().unwrap().is_empty());
    }
}
