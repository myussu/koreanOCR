//! Authentication and each hosted service's request dialect (Azure, Gemini, Anthropic's OpenAI-compatible
//! endpoint); ordinary local servers keep their requests.
use crate::{LlmError, Result};
use maru_core::config::LlmConfig;
use maru_core::external_servers::{ServerKey, ServerProvider};
use reqwest::{RequestBuilder, header::HeaderValue};
use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Authentication {
    pub provider: ServerProvider,
    key: Option<ServerKey>,
    /// Anthropic's OpenAI-compatible endpoint, saved as an ordinary OpenAI-compatible server: it is told apart by
    /// its address, so a connection saved before Maru knew about it works without being edited.
    anthropic: bool,
}

/// How a Claude model is kept from thinking before a short answer. The endpoint ignores `reasoning_effort`, thinking
/// is on by default, and what it thinks counts against `max_tokens`: with Maru's small limits a line came back empty.
/// Which way of switching it off a model accepts differs, so they are tried in this order and the answer remembered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClaudeThinking {
    /// `thinking: {"type": "disabled"}`.
    Disabled = 0,
    /// `thinking: {"type": "between_tools"}`, for the models that refuse `disabled` and name this instead.
    BetweenTools = 1,
    /// Thinking stays on (wanted, or the model cannot switch it off): the answer gets room beyond it instead.
    Left = 2,
}

/// Tokens added to a request's limit when a Claude model thinks first.
const THINKING_ROOM: u64 = 2048;

impl ClaudeThinking {
    pub fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Disabled,
            1 => Self::BetweenTools,
            _ => Self::Left,
        }
    }
    /// Where to start: off for `reasoning_effort = "none"` (the default), on for anything else.
    pub fn for_effort(effort: Option<&str>) -> Self {
        if effort.is_some_and(|e| e.eq_ignore_ascii_case("none")) { Self::Disabled } else { Self::Left }
    }
    /// What to try after the endpoint refused the `thinking` field with this message.
    pub fn after_refusal(self, message: &str) -> Self {
        if self == Self::Disabled && message.contains("between_tools") { Self::BetweenTools } else { Self::Left }
    }
}

/// `https://api.anthropic.com`, with or without `/v1`.
fn is_anthropic(root: &str) -> bool {
    let host = root.split_once("://").map_or(root, |(_, rest)| rest).split(['/', ':']).next().unwrap_or_default();
    host.eq_ignore_ascii_case("api.anthropic.com")
}

impl Authentication {
    pub fn from_config(cfg: &LlmConfig, root: &str) -> Self {
        if cfg.endpoint.is_none() {
            return Self::default();
        }
        let auth = Self {
            provider: cfg.provider,
            key: cfg
                .api_key
                .clone()
                .filter(|key| !key.as_str().trim().is_empty()),
            anthropic: cfg.provider == ServerProvider::Openai && is_anthropic(root),
        };
        if cfg
            .endpoint
            .as_ref()
            .is_some_and(|endpoint| auth.root(endpoint) != auth.root(root))
        {
            return Self::default();
        }
        auth
    }
    pub fn authenticated(&self) -> bool {
        self.key.is_some()
    }
    /// A Claude model is on the other end, through either of Anthropic's endpoints (thinking is handled the same).
    pub fn anthropic(&self) -> bool {
        self.anthropic || self.native_anthropic()
    }
    /// Anthropic's own Messages API: another request body and other stream events (`anthropic.rs`).
    pub fn native_anthropic(&self) -> bool {
        self.provider == ServerProvider::Anthropic
    }
    /// Write `mode` into a request for Claude; `limit` is the `max_tokens` the task asked for.
    pub fn claude_thinking(&self, body: &mut Value, mode: ClaudeThinking, limit: Option<u64>) {
        let Some(fields) = body.as_object_mut() else { return };
        match mode {
            ClaudeThinking::Disabled => fields.insert("thinking".into(), serde_json::json!({ "type": "disabled" })),
            ClaudeThinking::BetweenTools => fields.insert("thinking".into(), serde_json::json!({ "type": "between_tools" })),
            ClaudeThinking::Left => fields.remove("thinking"),
        };
        if let Some(limit) = limit {
            let room = if mode == ClaudeThinking::Left { THINKING_ROOM } else { 0 };
            fields.insert("max_tokens".into(), serde_json::json!(limit + room));
        }
    }
    #[cfg(test)]
    pub fn with_anthropic_dialect(mut self) -> Self {
        self.anthropic = true;
        self
    }
    /// A hosted API: no `/health`, no Ollama routes or fields, and it always needs a key.
    pub fn cloud(&self) -> bool {
        matches!(self.provider, ServerProvider::Azure | ServerProvider::Gemini | ServerProvider::Anthropic) || self.anthropic
    }
    pub fn root(&self, root: &str) -> String {
        if self.provider == ServerProvider::Gemini {
            // Gemini's OpenAI-compatible base already carries its version (`/v1beta/openai`); a bare host gets it.
            let root = root.trim_end_matches('/');
            let has_path = root.split_once("://").is_some_and(|(_, rest)| rest.contains('/'));
            return if has_path { root.into() } else { format!("{root}/v1beta/openai") };
        }
        let root = root.trim_end_matches('/').trim_end_matches("/v1");
        if self.provider == ServerProvider::Azure && !root.ends_with("/openai") {
            format!("{root}/openai")
        } else {
            root.into()
        }
    }
    /// Where chat completions live under [`Self::root`].
    pub fn chat_route(&self) -> &'static str {
        match self.provider {
            ServerProvider::Gemini => "/chat/completions",
            ServerProvider::Anthropic => "/v1/messages",
            _ => "/v1/chat/completions",
        }
    }
    /// Where the list of models lives under [`Self::root`].
    pub fn models_route(&self) -> &'static str {
        if self.provider == ServerProvider::Gemini { "/models" } else { "/v1/models" }
    }
    pub fn request(&self, request: RequestBuilder) -> Result<RequestBuilder> {
        let Some(key) = &self.key else {
            if self.provider == ServerProvider::Azure {
                return Err(LlmError::Api(
                    "Enter the Azure OpenAI resource API key in Settings.".into(),
                ));
            }
            if self.provider == ServerProvider::Gemini {
                return Err(LlmError::Api("Enter the Gemini API key in Settings.".into()));
            }
            if self.anthropic() {
                return Err(LlmError::Api("Enter the Anthropic API key in Settings.".into()));
            }
            return Ok(request);
        };
        if self.native_anthropic() {
            let mut header = HeaderValue::from_str(key.as_str()).map_err(|_| LlmError::Api("The API key is not a valid HTTP header. Enter it again.".into()))?;
            header.set_sensitive(true);
            return Ok(request.header("x-api-key", header).header("anthropic-version", crate::anthropic::API_VERSION));
        }
        let (name, text) = if self.provider == ServerProvider::Azure {
            ("api-key", key.as_str().to_string())
        } else {
            ("authorization", format!("Bearer {}", key.as_str()))
        };
        let mut header = HeaderValue::from_str(&text).map_err(|_| {
            LlmError::Api("The API key is not a valid HTTP header. Enter it again.".into())
        })?;
        header.set_sensitive(true);
        Ok(request.header(name, header))
    }
    pub fn body(&self, body: &mut Value) {
        let Some(fields) = body.as_object_mut() else { return };
        if self.cloud() {
            fields.remove("keep_alive");
        }
        if self.anthropic {
            // Current Claude models answer 400 to any temperature below 1, and the endpoint ignores
            // `reasoning_effort`: without them the first request of every run is not wasted on a refusal.
            fields.remove("temperature");
            fields.remove("reasoning_effort");
        }
        if self.provider == ServerProvider::Azure {
            if let Some(limit) = fields.remove("max_tokens") {
                fields.insert("max_completion_tokens".into(), limit);
            }
            // Temperature stays: a deployment that refuses it (a reasoning model) answers 400 naming it, and the
            // client then asks again without it (`client.rs`, rejected optional fields).
            if fields.get("reasoning_effort").is_some_and(|v| v == "none") {
                fields.remove("reasoning_effort");
            }
        }
    }
    pub fn redact(&self, text: String) -> String {
        match &self.key {
            Some(key) => text.replace(key.as_str(), "[redacted]"),
            None => text,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn claude(endpoint: &str, key: Option<&str>) -> Authentication {
        let cfg = LlmConfig { endpoint: Some(endpoint.into()), api_key: key.map(|k| k.to_string().into()), ..Default::default() };
        Authentication::from_config(&cfg, endpoint)
    }

    #[test]
    fn anthropics_compatible_endpoint_is_a_hosted_service_not_a_local_server() {
        for endpoint in ["https://api.anthropic.com", "https://api.anthropic.com/v1/", "https://API.anthropic.com/v1"] {
            let auth = claude(endpoint, Some("sk-test"));
            assert!(auth.cloud(), "{endpoint}: no health probe, no keep-alive, no Ollama routes");
            assert_eq!((auth.root(endpoint).to_lowercase().as_str(), auth.chat_route()), ("https://api.anthropic.com", "/v1/chat/completions"));
            let mut body = json!({ "model": "claude-sonnet-5-5", "stream": true, "temperature": 0.1, "max_tokens": 80, "reasoning_effort": "none", "keep_alive": "15m", "stream_options": { "include_usage": true } });
            auth.body(&mut body);
            assert_eq!(body, json!({ "model": "claude-sonnet-5-5", "stream": true, "max_tokens": 80, "stream_options": { "include_usage": true } }), "what current Claude models refuse or ignore is left out");
        }
        let missing = claude("https://api.anthropic.com", None).request(reqwest::Client::new().get("https://api.anthropic.com")).unwrap_err().to_string();
        assert!(missing.contains("Anthropic API key"), "{missing}");
    }

    #[test]
    fn another_openai_compatible_server_keeps_its_fields_whatever_its_name() {
        for endpoint in ["http://192.168.1.20:11434", "https://anthropic.example.org", "https://api.anthropic.com.example.org"] {
            let auth = claude(endpoint, None);
            assert!(!auth.cloud(), "{endpoint}");
            let mut body = json!({ "temperature": 0.1, "reasoning_effort": "none", "keep_alive": "15m" });
            auth.body(&mut body);
            assert_eq!(body, json!({ "temperature": 0.1, "reasoning_effort": "none", "keep_alive": "15m" }));
        }
    }
    #[tokio::test]
    async fn an_explicit_different_client_root_does_not_inherit_a_configured_cloud_key() {
        let server = crate::testing::spawn_mock(|_, _| crate::testing::MockResponse {
            status: 200,
            chunks: crate::testing::sse_answer(&["Hello"]),
            delay_ms: 0,
        })
        .await;
        let cfg = LlmConfig {
            endpoint: Some("https://production.openai.azure.com".into()),
            provider: ServerProvider::Azure,
            api_key: Some("private-cloud-key".to_string().into()),
            ..Default::default()
        };
        crate::LlmClient::new(&server.url, &cfg)
            .stream_chat(
                &[crate::Message::user("hello")],
                &Default::default(),
                &mut |_| {},
            )
            .await
            .unwrap();
        assert!(
            server
                .headers
                .lock()
                .unwrap()
                .iter()
                .all(|header| !header.contains("private-cloud-key"))
        );
        assert_eq!(server.requests.lock().unwrap()[0].0, "/v1/chat/completions");
    }
    #[tokio::test]
    async fn an_authenticated_redirect_cannot_contact_another_server_or_forward_the_key() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let destination = crate::testing::spawn_mock(|_, _| crate::testing::MockResponse {
            status: 200,
            chunks: crate::testing::sse_answer(&["Wrong server"]),
            delay_ms: 0,
        })
        .await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let location = format!("{}/v1/chat/completions", destination.url);
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0u8; 4096];
            let _ = socket.read(&mut buffer).await;
            let response = format!(
                "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            let _ = socket.write_all(response.as_bytes()).await;
        });
        let cfg = LlmConfig {
            endpoint: Some(endpoint),
            api_key: Some("private-key".to_string().into()),
            ..Default::default()
        };
        let error = crate::LlmClient::from_config(&cfg)
            .stream_chat(
                &[crate::Message::user("hello")],
                &Default::default(),
                &mut |_| {},
            )
            .await
            .unwrap_err();
        assert!(matches!(error, LlmError::Http { status: 302, .. }));
        assert!(destination.requests.lock().unwrap().is_empty());
    }
    #[tokio::test]
    async fn a_missing_azure_key_is_refused_before_any_request_is_sent() {
        let server = crate::testing::spawn_mock(|_, _| crate::testing::MockResponse {
            status: 200,
            chunks: crate::testing::sse_answer(&["Hello"]),
            delay_ms: 0,
        })
        .await;
        let cfg = LlmConfig {
            endpoint: Some(server.url.clone()),
            provider: ServerProvider::Azure,
            ..Default::default()
        };
        let error = crate::LlmClient::from_config(&cfg)
            .stream_chat(
                &[crate::Message::user("hello")],
                &Default::default(),
                &mut |_| {},
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("API key"));
        assert!(server.requests.lock().unwrap().is_empty());
    }
    #[tokio::test]
    async fn azure_inference_uses_the_deployment_and_key_without_health_or_unload_requests() {
        let server = crate::testing::spawn_mock(|_, _| crate::testing::MockResponse {
            status: 200,
            chunks: crate::testing::sse_answer(&["Hello."]),
            delay_ms: 0,
        })
        .await;
        let cfg = LlmConfig {
            endpoint: Some(format!("{}/openai/v1/", server.url)),
            provider: ServerProvider::Azure,
            api_key: Some("azure-secret".to_string().into()),
            model_name: "my-deployment".into(),
            server_keep_minutes: 20,
            ..Default::default()
        };
        let client = crate::LlmClient::from_config(&cfg);
        assert!(client.health().await);
        assert!(!client.holds_server_model());
        let result = client
            .stream_chat(
                &[crate::Message::user("안녕하세요.")],
                &Default::default(),
                &mut |_| {},
            )
            .await
            .unwrap();
        assert_eq!(result, "Hello.");
        client.release_server_model().await.unwrap();
        let requests = server.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].0, "/openai/v1/chat/completions");
        let body: Value = serde_json::from_str(&requests[0].1).unwrap();
        assert_eq!(body["model"], "my-deployment");
        assert!(body.get("max_completion_tokens").is_some());
        assert!(body.get("keep_alive").is_none());
        assert!(!requests[0].1.contains("azure-secret"));
        let headers = server.headers.lock().unwrap();
        assert!(headers[0].contains("api-key: azure-secret"));
        assert!(!headers[0].contains("authorization:"));
    }
    #[tokio::test]
    async fn a_generic_server_gets_its_own_bearer_key_and_errors_do_not_expose_it() {
        let server = crate::testing::spawn_mock(|_, _| crate::testing::MockResponse {
            status: 401,
            chunks: vec![b"invalid generic-secret".to_vec()],
            delay_ms: 0,
        })
        .await;
        let cfg = LlmConfig {
            endpoint: Some(server.url.clone()),
            api_key: Some("generic-secret".to_string().into()),
            ..Default::default()
        };
        let error = crate::LlmClient::from_config(&cfg)
            .stream_chat(
                &[crate::Message::user("hello")],
                &Default::default(),
                &mut |_| {},
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("401"));
        assert!(!error.contains("generic-secret"));
        assert!(server.headers.lock().unwrap()[0].contains("authorization: Bearer generic-secret"));
    }
    #[test]
    fn azure_resource_and_v1_addresses_use_the_same_route_without_ollama_fields() {
        let auth = Authentication {
            provider: ServerProvider::Azure,
            key: None,
            ..Default::default()
        };
        for base in [
            "https://example.azure.com",
            "https://example.azure.com/openai",
            "https://example.azure.com/openai/v1/",
        ] {
            assert_eq!(auth.root(base), "https://example.azure.com/openai");
        }
        let mut body = json!({"max_tokens":200,"temperature":0.1,"reasoning_effort":"none","keep_alive":"20m","model":"deployment"});
        auth.body(&mut body);
        assert_eq!(
            body,
            json!({"max_completion_tokens":200,"temperature":0.1,"model":"deployment"})
        );
    }
    #[test]
    fn gemini_uses_its_versioned_openai_route_and_keeps_sampling_and_thinking_fields() {
        let auth = Authentication { provider: ServerProvider::Gemini, key: Some("k".to_string().into()), ..Default::default() };
        for base in ["https://generativelanguage.googleapis.com", "https://generativelanguage.googleapis.com/v1beta/openai/"] {
            assert_eq!(auth.root(base), "https://generativelanguage.googleapis.com/v1beta/openai");
        }
        assert_eq!(auth.chat_route(), "/chat/completions");
        let mut body = json!({"max_tokens":200,"temperature":0.1,"reasoning_effort":"none","keep_alive":"20m"});
        auth.body(&mut body);
        assert_eq!(body, json!({"max_tokens":200,"temperature":0.1,"reasoning_effort":"none"}));
    }

    #[test]
    fn local_clients_never_inherit_cloud_keys_or_the_azure_request_dialect() {
        let cfg = LlmConfig {
            provider: ServerProvider::Azure,
            api_key: Some("private-key".to_string().into()),
            ..Default::default()
        };
        let auth = Authentication::from_config(&cfg, "http://127.0.0.1:8080");
        assert!(!auth.authenticated());
        assert_eq!(auth.provider, ServerProvider::Openai);
    }
    #[test]
    fn keys_are_sent_only_as_sensitive_headers_and_never_as_url_or_body_fields() {
        let http = reqwest::Client::new();
        for (provider, name, value) in [
            (
                ServerProvider::Openai,
                "authorization",
                "Bearer private-key",
            ),
            (ServerProvider::Azure, "api-key", "private-key"),
        ] {
            let auth = Authentication {
                provider,
                key: Some("private-key".to_string().into()),
                ..Default::default()
            };
            let request = auth
                .request(
                    http.post("http://127.0.0.1/v1/chat/completions")
                        .json(&json!({"model":"deployment"})),
                )
                .unwrap()
                .build()
                .unwrap();
            assert_eq!(request.headers()[name], value);
            assert!(request.headers()[name].is_sensitive());
            assert!(!request.url().as_str().contains("private-key"));
            assert!(!format!("{request:?}").contains("private-key"));
            assert_eq!(auth.redact("bad private-key".into()), "bad [redacted]");
        }
    }
}
