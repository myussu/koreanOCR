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

pub async fn test_authenticated_connection(endpoint: &str, model_name: &str, reasoning_effort: &str, provider: maru_core::external_servers::ServerProvider, api_key: Option<maru_core::external_servers::ServerKey>) -> Result<ConnectionReply, String> {
    check_authenticated(endpoint, model_name, reasoning_effort, provider, api_key, Duration::from_secs(60)).await
}

async fn check(endpoint: &str, model_name: &str, reasoning_effort: &str, deadline: Duration) -> Result<ConnectionReply, String> {
    check_authenticated(endpoint, model_name, reasoning_effort, Default::default(), None, deadline).await
}

/// Why a connection check did not pass.
#[derive(Debug)]
pub enum CheckFailure {
    /// What was typed cannot be asked: the sentence says what to change.
    Input(String),
    /// The server did not answer within the time allowed.
    TimedOut,
    /// It answered with nothing.
    Empty,
    /// The request failed.
    Model(crate::LlmError),
}

impl CheckFailure {
    /// The kind of problem and one plain sentence about the connection called `name`, with no status code or
    /// server text (the settings page shows this).
    pub fn plain(&self, name: &str) -> (Option<crate::problem::Problem>, String) {
        use crate::problem::{Problem, classify, sentence};
        match self {
            CheckFailure::Input(what) => (None, what.clone()),
            CheckFailure::TimedOut => (Some(Problem::Unreachable), format!("{name} did not answer in time. It may still be loading its model: try again in a moment.")),
            CheckFailure::Empty => (Some(Problem::Failed), format!("{name} answered with nothing. Check the model name.")),
            CheckFailure::Model(error) => {
                let problem = classify(error);
                let hint = match (problem, error) {
                    (Problem::Failed, crate::LlmError::Http { status: 404, .. }) => " The server does not know that model name.",
                    (Problem::Unreachable, _) => " Check the address and that the server is running.",
                    _ => "",
                };
                (Some(problem), format!("{}{hint}", sentence(name, problem)))
            }
        }
    }
}

impl std::fmt::Display for CheckFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CheckFailure::Input(what) => f.write_str(what),
            CheckFailure::TimedOut => f.write_str("The connection test timed out. The server may still be loading the model; check it and try again."),
            CheckFailure::Empty => f.write_str("The server responded but returned no translation. Check the model name and its streaming chat API."),
            CheckFailure::Model(error) => write!(f, "{error}"),
        }
    }
}

/// The same check as [`test_authenticated_connection`], keeping what went wrong so it can be said plainly.
pub async fn check_connection(endpoint: &str, model_name: &str, reasoning_effort: &str, provider: maru_core::external_servers::ServerProvider, api_key: Option<maru_core::external_servers::ServerKey>) -> Result<ConnectionReply, CheckFailure> {
    checked(endpoint, model_name, reasoning_effort, provider, api_key, Duration::from_secs(60)).await
}

async fn check_authenticated(endpoint: &str, model_name: &str, reasoning_effort: &str, provider: maru_core::external_servers::ServerProvider, api_key: Option<maru_core::external_servers::ServerKey>, deadline: Duration) -> Result<ConnectionReply, String> {
    checked(endpoint, model_name, reasoning_effort, provider, api_key, deadline).await.map_err(|e| e.to_string())
}

async fn checked(endpoint: &str, model_name: &str, reasoning_effort: &str, provider: maru_core::external_servers::ServerProvider, api_key: Option<maru_core::external_servers::ServerKey>, deadline: Duration) -> Result<ConnectionReply, CheckFailure> {
    let input = CheckFailure::Input;
    let endpoint = crate::discovery::normalize(endpoint).map_err(input)?;
    let url = reqwest::Url::parse(&endpoint).map_err(|_| input("Enter a complete server address, such as http://192.168.1.20:11434.".into()))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(input("The server address must start with http:// or https://.".into()));
    }
    if model_name.trim().is_empty() {
        return Err(input("Enter the model name expected by the server.".into()));
    }
    maru_core::external_servers::validate_connection(&endpoint, model_name, provider, api_key.as_ref()).map_err(input)?;
    let cfg = LlmConfig {
        endpoint: Some(endpoint.clone()), model_name: model_name.trim().into(),
        reasoning_effort: reasoning_effort.into(), request_timeout_secs: 60,
        provider, api_key,
        ..Default::default()
    };
    // No runtime is attached: checking a draft must never start a local model.
    let client = LlmClient::from_config(&cfg);
    let messages = [Message::user("Translate this Korean sentence into English. Return only the translation:\n안녕하세요.")];
    let opts = ChatOpts { max_tokens: Some(64), temperature: Some(0.0), ..Default::default() };
    let start = Instant::now();
    let reply = tokio::time::timeout(deadline, client.stream_chat(&messages, &opts, &mut |_| {}))
        .await.map_err(|_| CheckFailure::TimedOut)?
        .map_err(CheckFailure::Model)?;
    if reply.trim().is_empty() {
        return Err(CheckFailure::Empty);
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
    async fn a_failed_check_is_said_in_plain_words_without_the_servers_text() {
        use crate::problem::Problem;
        let quota = spawn_mock(|_, _| MockResponse { status: 429, chunks: vec![br#"{"error":{"status":"RESOURCE_EXHAUSTED"}}"#.to_vec()], delay_ms: 0 }).await;
        let failure = check_connection(&quota.url, "m", "", Default::default(), None).await.unwrap_err();
        assert_eq!(failure.plain("Gemini"), (Some(Problem::Quota), "Gemini has no quota left.".to_string()));
        let missing = spawn_mock(|_, _| MockResponse { status: 404, chunks: vec![b"{\"error\":\"model 'x' not found\"}".to_vec()], delay_ms: 0 }).await;
        let (problem, text) = check_connection(&missing.url, "x", "", Default::default(), None).await.unwrap_err().plain("NAS");
        assert_eq!(problem, Some(Problem::Failed));
        assert!(text.contains("does not know that model name") && !text.contains('{') && !text.contains("404"), "{text}");
        let (problem, text) = check_connection("http://127.0.0.1:9", "x", "", Default::default(), None).await.unwrap_err().plain("NAS");
        assert_eq!(problem, Some(Problem::Unreachable));
        assert!(text.starts_with("NAS cannot be reached.") && !text.contains("127.0.0.1"), "{text}");
        let (problem, text) = check_connection("nas", " ", "", Default::default(), None).await.unwrap_err().plain("NAS");
        assert!(problem.is_none() && text.contains("model name"), "what was typed is explained as before: {text}");
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
