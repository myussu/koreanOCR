//! Streaming chat client for llama.cpp's OpenAI-compatible server.
//!
//! Only what Maru needs: one streaming call, a health probe, and tolerant SSE
//! parsing that survives chunk boundaries falling inside a UTF-8 character.

use std::time::Duration;

use futures_util::StreamExt;
use maru_core::config::LlmConfig;
use serde::Serialize;
use serde_json::{Value, json};

use crate::{LlmError, Result};

#[derive(Debug, Clone)]
pub struct Message {
    pub role: &'static str,
    pub content: String,
    /// A `data:image/png;base64,…` URL sent alongside the text (vision models only).
    pub image: Option<String>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: "system", content: content.into(), image: None }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: "user", content: content.into(), image: None }
    }
    pub fn user_with_image(content: impl Into<String>, data_url: impl Into<String>) -> Self {
        Self { role: "user", content: content.into(), image: Some(data_url.into()) }
    }
}

impl Serialize for Message {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut m = serializer.serialize_struct("Message", 2)?;
        m.serialize_field("role", self.role)?;
        match &self.image {
            // OpenAI "content parts": the image first, then the instruction.
            Some(url) => m.serialize_field(
                "content",
                &json!([
                    { "type": "image_url", "image_url": { "url": url } },
                    { "type": "text", "text": self.content },
                ]),
            )?,
            None => m.serialize_field("content", &self.content)?,
        }
        m.end()
    }
}

#[derive(Debug, Clone, Default)]
pub struct ChatOpts {
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub stop: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct LlmClient {
    http: reqwest::Client,
    /// Server root without `/v1`, e.g. `http://127.0.0.1:18080`.
    root: String,
    model: String,
    temperature: f32,
    max_tokens: u32,
    idle_timeout: Duration,
    /// Send llama.cpp-specific request fields (`cache_prompt`, …). Off for foreign servers.
    llama_extensions: bool,
}

impl LlmClient {
    pub fn new(root: &str, cfg: &LlmConfig) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .pool_idle_timeout(Duration::from_secs(300))
            .tcp_nodelay(true)
            .build()
            .expect("static client configuration is valid");
        Self {
            http,
            root: root.trim_end_matches('/').trim_end_matches("/v1").to_string(),
            model: cfg.model_name.clone(),
            temperature: cfg.temperature,
            max_tokens: cfg.max_tokens,
            idle_timeout: Duration::from_secs(cfg.request_timeout_secs.max(1)),
            llama_extensions: cfg.endpoint.is_none(),
        }
    }

    /// Client for the configured endpoint, or the local llama-server port.
    pub fn from_config(cfg: &LlmConfig) -> Self {
        match &cfg.endpoint {
            Some(url) => Self::new(url, cfg),
            None => Self::new(&format!("http://127.0.0.1:{}", cfg.port), cfg),
        }
    }

    pub fn root(&self) -> &str {
        &self.root
    }

    /// `true` once llama-server reports the model loaded. Servers without a
    /// `/health` route (answering 404) are assumed to be ready.
    pub async fn health(&self) -> bool {
        match self.http.get(format!("{}/health", self.root)).timeout(Duration::from_secs(2)).send().await {
            Ok(r) => r.status().is_success() || r.status().as_u16() == 404,
            Err(_) => false,
        }
    }

    pub async fn wait_ready(&self, timeout: Duration) -> Result<()> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if self.health().await {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(LlmError::NotReady(format!("no healthy response from {} within {timeout:?}", self.root)));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    fn body(&self, messages: &[Message], opts: &ChatOpts) -> Value {
        let mut body = json!({
            "model": self.model,
            "messages": messages,
            "stream": true,
            "temperature": opts.temperature.unwrap_or(self.temperature),
            "max_tokens": opts.max_tokens.unwrap_or(self.max_tokens),
        });
        if !opts.stop.is_empty() {
            body["stop"] = json!(opts.stop);
        }
        if self.llama_extensions {
            // Reuse the KV cache for the shared system-prompt prefix, and keep
            // "thinking" models (Qwen3-style templates) from reasoning aloud.
            body["cache_prompt"] = json!(true);
            body["chat_template_kwargs"] = json!({ "enable_thinking": false });
        }
        body
    }

    /// Stream a chat completion. `on_delta` receives text as it is generated;
    /// the full text is returned at the end. Dropping the future cancels the
    /// request (the server stops generating when the connection closes).
    pub async fn stream_chat(&self, messages: &[Message], opts: &ChatOpts, on_delta: &mut (dyn FnMut(&str) + Send)) -> Result<String> {
        let url = format!("{}/v1/chat/completions", self.root);
        let resp = self
            .http
            .post(&url)
            .json(&self.body(messages, opts))
            .send()
            .await
            .map_err(|source| LlmError::Unreachable { url: self.root.clone(), source })?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(LlmError::Http { status: status.as_u16(), body: body.chars().take(400).collect() });
        }

        let mut stream = resp.bytes_stream();
        let mut parser = SseParser::default();
        let mut full = String::new();
        loop {
            let next = tokio::time::timeout(self.idle_timeout, stream.next())
                .await
                .map_err(|_| LlmError::Stalled(self.idle_timeout.as_secs()))?;
            let Some(chunk) = next else { break };
            let chunk = chunk.map_err(|e| LlmError::Stream(e.to_string()))?;
            for event in parser.push(&chunk) {
                match event {
                    SseEvent::Delta(text) => {
                        on_delta(&text);
                        full.push_str(&text);
                    }
                    SseEvent::Error(e) => return Err(LlmError::Api(e)),
                    SseEvent::Done => return Ok(full),
                }
            }
        }
        for event in parser.finish() {
            if let SseEvent::Delta(text) = event {
                on_delta(&text);
                full.push_str(&text);
            }
        }
        Ok(full)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum SseEvent {
    Delta(String),
    Error(String),
    Done,
}

/// Incremental `text/event-stream` parser for chat-completion chunks.
#[derive(Debug, Default)]
pub struct SseParser {
    buf: Vec<u8>,
}

impl SseParser {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        self.buf.extend_from_slice(bytes);
        let mut events = Vec::new();
        // Split on raw bytes: a network chunk may end inside a multi-byte character.
        while let Some(i) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=i).collect();
            if let Some(e) = parse_line(&line) {
                events.push(e);
            }
        }
        events
    }

    pub fn finish(&mut self) -> Vec<SseEvent> {
        let rest = std::mem::take(&mut self.buf);
        parse_line(&rest).into_iter().collect()
    }
}

fn parse_line(line: &[u8]) -> Option<SseEvent> {
    let line = String::from_utf8_lossy(line);
    let payload = line.trim().strip_prefix("data:")?.trim();
    if payload == "[DONE]" {
        return Some(SseEvent::Done);
    }
    let v: Value = serde_json::from_str(payload).ok()?;
    if let Some(err) = v.get("error") {
        let msg = err.get("message").and_then(Value::as_str).or_else(|| err.as_str()).unwrap_or("unknown error");
        return Some(SseEvent::Error(msg.to_string()));
    }
    let text = v.pointer("/choices/0/delta/content").and_then(Value::as_str)?;
    (!text.is_empty()).then(|| SseEvent::Delta(text.to_string()))
}

#[cfg(any(test, feature = "testing"))]
pub mod testing {
    //! A tiny in-process stand-in for llama-server, used by this crate's tests and the engine's.
    use std::sync::{Arc, Mutex};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    pub struct MockResponse {
        pub status: u16,
        pub chunks: Vec<Vec<u8>>,
        pub delay_ms: u64,
    }

    pub struct MockServer {
        pub url: String,
        pub requests: Arc<Mutex<Vec<(String, String)>>>,
    }

    /// One SSE `data:` event carrying a content delta.
    pub fn sse_event(delta: &str) -> Vec<u8> {
        let v = serde_json::json!({ "choices": [{ "delta": { "content": delta } }] });
        format!("data: {v}\n\n").into_bytes()
    }

    /// A full streamed answer, one event per delta, then `[DONE]`.
    pub fn sse_answer(deltas: &[&str]) -> Vec<Vec<u8>> {
        let mut out: Vec<Vec<u8>> = deltas.iter().map(|d| sse_event(d)).collect();
        out.push(b"data: [DONE]\n\n".to_vec());
        out
    }

    pub async fn spawn_mock(handler: impl Fn(&str, &str) -> MockResponse + Send + Sync + 'static) -> MockServer {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let handler = Arc::new(handler);
        let log = requests.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let handler = handler.clone();
                let log = log.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 65536];
                    let mut total = 0;
                    let (path, body) = loop {
                        let n = sock.read(&mut buf[total..]).await.unwrap_or(0);
                        total += n;
                        let text = String::from_utf8_lossy(&buf[..total]).to_string();
                        if let Some(i) = text.find("\r\n\r\n") {
                            let head = &text[..i];
                            let len = head
                                .lines()
                                .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                                .unwrap_or(0);
                            if total >= i + 4 + len {
                                let path = head.split_whitespace().nth(1).unwrap_or("/").to_string();
                                break (path, text[i + 4..i + 4 + len].to_string());
                            }
                        }
                        if n == 0 {
                            return;
                        }
                    };
                    log.lock().unwrap().push((path.clone(), body.clone()));
                    let resp = handler(&path, &body);
                    let head = format!(
                        "HTTP/1.1 {} X\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n",
                        resp.status
                    );
                    let _ = sock.write_all(head.as_bytes()).await;
                    for chunk in resp.chunks {
                        let _ = sock.write_all(&chunk).await;
                        let _ = sock.flush().await;
                        if resp.delay_ms > 0 {
                            tokio::time::sleep(std::time::Duration::from_millis(resp.delay_ms)).await;
                        }
                    }
                    let _ = sock.shutdown().await;
                });
            }
        });
        MockServer { url, requests }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    fn cfg() -> LlmConfig {
        LlmConfig { request_timeout_secs: 2, ..Default::default() }
    }

    #[test]
    fn image_messages_use_openai_content_parts_and_text_messages_stay_plain() {
        let plain = serde_json::to_value(Message::user("hi")).unwrap();
        assert_eq!(plain, json!({ "role": "user", "content": "hi" }));
        let with_image = serde_json::to_value(Message::user_with_image("OCR:", "data:image/png;base64,AAAA")).unwrap();
        assert_eq!(with_image["content"][0]["image_url"]["url"], "data:image/png;base64,AAAA");
        assert_eq!(with_image["content"][1], json!({ "type": "text", "text": "OCR:" }));
    }

    #[test]
    fn sse_parser_survives_split_multibyte_characters() {
        let event = sse_event("안녕");
        let mut p = SseParser::default();
        // cut inside the first Hangul character (3 bytes in UTF-8)
        let cut = event.iter().position(|&b| b >= 0x80).unwrap() + 1;
        assert!(p.push(&event[..cut]).is_empty());
        assert_eq!(p.push(&event[cut..]), vec![SseEvent::Delta("안녕".into())]);
        assert_eq!(p.push(b"data: [DONE]\n\n"), vec![SseEvent::Done]);
    }

    #[test]
    fn sse_parser_ignores_noise_and_surfaces_errors() {
        let mut p = SseParser::default();
        let evs = p.push(b": keep-alive\n\nevent: ping\ndata: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\ndata: {\"error\":{\"message\":\"boom\"}}\n\n");
        assert_eq!(evs, vec![SseEvent::Error("boom".into())]);
    }

    #[tokio::test]
    async fn streams_deltas_in_order_and_returns_full_text() {
        let server = spawn_mock(|_, _| MockResponse { status: 200, chunks: sse_answer(&["I ", "study ", "Korean."]), delay_ms: 2 }).await;
        let client = LlmClient::new(&server.url, &cfg());
        let mut seen = Vec::new();
        let full = client
            .stream_chat(&[Message::system("s"), Message::user("u")], &ChatOpts::default(), &mut |d| seen.push(d.to_string()))
            .await
            .unwrap();
        assert_eq!(full, "I study Korean.");
        assert_eq!(seen, ["I ", "study ", "Korean."]);

        let (path, body) = server.requests.lock().unwrap()[0].clone();
        assert_eq!(path, "/v1/chat/completions");
        let body: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["stream"], true);
        assert_eq!(body["cache_prompt"], true);
        assert_eq!(body["messages"][0]["role"], "system");
    }

    #[tokio::test]
    async fn http_errors_carry_the_body() {
        let server = spawn_mock(|_, _| MockResponse { status: 503, chunks: vec![b"model is loading".to_vec()], delay_ms: 0 }).await;
        let client = LlmClient::new(&server.url, &cfg());
        let err = client.stream_chat(&[Message::user("x")], &ChatOpts::default(), &mut |_| {}).await.unwrap_err();
        assert!(matches!(&err, LlmError::Http { status: 503, body } if body.contains("loading")), "{err}");
    }

    #[tokio::test]
    async fn unreachable_server_is_reported_clearly() {
        let client = LlmClient::new("http://127.0.0.1:1", &cfg());
        let err = client.stream_chat(&[Message::user("x")], &ChatOpts::default(), &mut |_| {}).await.unwrap_err();
        assert!(matches!(err, LlmError::Unreachable { .. }));
        assert!(!client.health().await);
    }

    #[tokio::test]
    async fn external_endpoints_do_not_get_llama_only_fields() {
        let server = spawn_mock(|_, _| MockResponse { status: 200, chunks: sse_answer(&["ok"]), delay_ms: 0 }).await;
        let cfg = LlmConfig { endpoint: Some(format!("{}/v1", server.url)), ..cfg() };
        let client = LlmClient::from_config(&cfg);
        assert_eq!(client.root(), server.url);
        client.stream_chat(&[Message::user("x")], &ChatOpts::default(), &mut |_| {}).await.unwrap();
        let body = server.requests.lock().unwrap()[0].1.clone();
        assert!(!body.contains("cache_prompt"));
    }

    #[tokio::test]
    async fn stalled_stream_times_out() {
        let server = spawn_mock(|_, _| MockResponse {
            status: 200,
            chunks: vec![sse_event("a"), sse_event("b")],
            delay_ms: 3000,
        })
        .await;
        let cfg = LlmConfig { request_timeout_secs: 1, ..Default::default() };
        let client = LlmClient::new(&server.url, &cfg);
        let err = client.stream_chat(&[Message::user("x")], &ChatOpts::default(), &mut |_| {}).await.unwrap_err();
        assert!(matches!(err, LlmError::Stalled(1)), "{err}");
    }

    #[tokio::test]
    async fn health_reflects_server_state() {
        let server = spawn_mock(|path, _| {
            if path == "/health" { MockResponse { status: 200, chunks: vec![b"{\"status\":\"ok\"}".to_vec()], delay_ms: 0 } } else { MockResponse { status: 404, chunks: vec![], delay_ms: 0 } }
        })
        .await;
        let client = LlmClient::new(&server.url, &cfg());
        client.wait_ready(Duration::from_secs(2)).await.unwrap();
    }
}
