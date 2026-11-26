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
    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: "assistant", content: content.into(), image: None }
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
    /// Live diagnostics only; leaves other requests and their wire format alone.
    pub log_timings: bool,
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
    /// `reasoning_effort` for foreign servers; `None` sends nothing.
    reasoning_effort: Option<String>,
    runtime: Option<(std::sync::Arc<crate::model_runtime::ModelRuntime>, crate::model_runtime::ModelRole)>,
    request_deadline: Option<Duration>,
    /// Ollama's `keep_alive` for a server given by `endpoint`; `None` leaves the server's own timeout.
    keep_alive: Option<String>,
    auth: crate::auth::Authentication,
    /// Optional request fields this server refused (HTTP 400 naming them); shared by clones of this client.
    rejected: std::sync::Arc<std::sync::Mutex<Vec<&'static str>>>,
    /// Ask the server to report token counts on every request ([`Self::counting_tokens`]); benchmarks only.
    count_tokens: bool,
    /// Tokens the server reported so far; shared by clones of this client.
    used: std::sync::Arc<TokenCounter>,
}

#[derive(Debug, Default)]
struct TokenCounter {
    replies: std::sync::atomic::AtomicU64,
    input: std::sync::atomic::AtomicU64,
    output: std::sync::atomic::AtomicU64,
}

/// Tokens a server said it used, summed over the replies that carried a count.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenUse {
    /// Replies that reported their tokens (a server that never does leaves this at 0).
    pub replies: u64,
    pub input: u64,
    /// Everything generated, thinking included where the server counts it.
    pub output: u64,
}

/// Fields Maru can leave out when a server refuses them: sampling, thinking, Ollama's keep-alive, usage reports.
const OPTIONAL_FIELDS: [&str; 4] = ["temperature", "reasoning_effort", "keep_alive", "stream_options"];

/// The optional field a 400 answer names, if the request carried it.
fn rejected_field(error: &str, body: &Value) -> Option<&'static str> {
    let error = error.to_ascii_lowercase();
    OPTIONAL_FIELDS.into_iter().find(|field| body.get(*field).is_some() && error.contains(field))
}

impl LlmClient {
    pub fn new(root: &str, cfg: &LlmConfig) -> Self {
        let auth = crate::auth::Authentication::from_config(cfg, root);
        let mut builder = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .pool_idle_timeout(Duration::from_secs(300))
            .tcp_nodelay(true);
        if auth.authenticated() { builder = builder.redirect(reqwest::redirect::Policy::none()); }
        let http = builder.build()
            .expect("static client configuration is valid");
        Self {
            http,
            root: auth.root(root),
            model: cfg.model_name.clone(),
            temperature: cfg.temperature,
            max_tokens: cfg.max_tokens,
            idle_timeout: Duration::from_secs(cfg.request_timeout_secs.max(1)),
            llama_extensions: cfg.endpoint.is_none(),
            reasoning_effort: Some(cfg.reasoning_effort.trim().to_string()).filter(|e| !e.is_empty()),
            runtime: None,
            request_deadline: None,
            keep_alive: (cfg.endpoint.is_some() && cfg.server_keep_minutes > 0 && !auth.cloud()).then(|| format!("{}m", cfg.server_keep_minutes)),
            auth,
            rejected: Default::default(),
            count_tokens: false,
            used: Default::default(),
        }
    }

    /// Ask the server for token counts on every request, to add up what a benchmark cost ([`Self::tokens_used`]).
    pub fn counting_tokens(mut self) -> Self {
        self.count_tokens = true;
        self
    }

    /// What the server reported since this client was made.
    pub fn tokens_used(&self) -> TokenUse {
        use std::sync::atomic::Ordering::Relaxed;
        TokenUse { replies: self.used.replies.load(Relaxed), input: self.used.input.load(Relaxed), output: self.used.output.load(Relaxed) }
    }

    /// The model this client asks for.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// The models this server (and, for a hosted one, this key) offers, as it names them.
    pub async fn list_models(&self) -> Result<Vec<String>> {
        let url = format!("{}{}", self.root, self.auth.models_route());
        let request = self.auth.request(self.http.get(&url).timeout(Duration::from_secs(20)))?;
        let resp = request.send().await.map_err(|source| LlmError::Unreachable { url: self.root.clone(), source })?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(LlmError::Http { status: status.as_u16(), body: self.auth.redact(text).chars().take(400).collect() });
        }
        let listed: Value = serde_json::from_str(&text).map_err(|_| LlmError::Api("the server's model list was not JSON".into()))?;
        // OpenAI-style `data[].id`; Ollama's and Google's native lists use `models[].name`.
        let entries = listed.get("data").or_else(|| listed.get("models")).and_then(Value::as_array).cloned().unwrap_or_default();
        let mut names: Vec<String> = entries
            .iter()
            .filter_map(|m| m.get("id").or_else(|| m.get("name")).and_then(Value::as_str))
            .map(|name| name.trim_start_matches("models/").to_string())
            .collect();
        names.sort();
        names.dedup();
        Ok(names)
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

    pub fn with_runtime(mut self, runtime: std::sync::Arc<crate::model_runtime::ModelRuntime>, role: crate::model_runtime::ModelRole) -> Self {
        self.runtime = Some((runtime, role));
        self
    }

    pub fn has_runtime(&self) -> bool { self.runtime.is_some() }
    /// Optional overall deadline for the snapshot fallback; ordinary clients keep their existing timeout.
    pub fn with_request_deadline(mut self, deadline: Duration) -> Self { self.request_deadline = Some(deadline); self }
    pub fn loaded_role(&self) -> Option<crate::model_runtime::ModelRole> { self.runtime.as_ref().and_then(|(runtime, _)| runtime.current_role()) }
    pub async fn ensure_loaded(&self) -> Result<()> {
        let _lease = self.model_lease().await?;
        Ok(())
    }

    pub(crate) async fn model_lease(&self) -> Result<Option<crate::model_runtime::ModelLease>> {
        match &self.runtime {
            Some((runtime, role)) => runtime.acquire(*role).await.map(Some),
            None => Ok(None),
        }
    }

    pub(crate) fn model_name(&self) -> &str {
        &self.model
    }

    /// Both clients name the same model on the same server.
    pub fn same_server_model(&self, other: &LlmClient) -> bool {
        self.root == other.root && self.model == other.model && self.auth == other.auth
    }

    /// A hosted API (Azure, Gemini): nothing to load, and every request is billed.
    pub fn is_hosted(&self) -> bool {
        self.auth.cloud()
    }

    /// This client asks its server to keep the model loaded (`[llm] server_keep_minutes`).
    pub fn holds_server_model(&self) -> bool {
        self.keep_alive.is_some() && !self.rejected.lock().unwrap_or_else(|p| p.into_inner()).contains(&"keep_alive")
    }

    /// Load the model on its server now, so the first real request does not wait for it.
    pub async fn hold_server_model(&self) -> Result<()> {
        let opts = ChatOpts { max_tokens: Some(1), ..Default::default() };
        self.stream_chat_reply(&[Message::user("안녕")], &opts, &mut |_| {}).await.map(|_| ())
    }

    /// Ask the server to unload the model now (Ollama's `/api/generate` with `keep_alive: 0`). It does not load
    /// a model that is not loaded. Other servers have no such route; that is not an error.
    pub async fn release_server_model(&self) -> Result<()> {
        if self.auth.cloud() { return Ok(()); }
        let request = self.http.post(format!("{}/api/generate", self.root))
            .json(&json!({ "model": self.model, "keep_alive": 0 }))
            .timeout(Duration::from_secs(5));
        let resp = self.auth.request(request)?.send().await.map_err(|source| LlmError::Unreachable { url: self.root.clone(), source })?;
        match resp.status() {
            s if s.is_success() || s == reqwest::StatusCode::NOT_FOUND => Ok(()),
            s => Err(LlmError::Http { status: s.as_u16(), body: self.auth.redact(resp.text().await.unwrap_or_default()).chars().take(400).collect() }),
        }
    }

    /// `true` once llama-server reports the model loaded. Servers without a
    /// `/health` route (answering 404) are assumed to be ready.
    pub async fn health(&self) -> bool {
        // Hosted APIs have no llama health endpoint. The explicit Settings check verifies inference.
        if self.auth.cloud() { return self.auth.authenticated(); }
        let Ok(request) = self.auth.request(self.http.get(format!("{}/health", self.root)).timeout(Duration::from_secs(2))) else { return false; };
        match request.send().await {
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
        if opts.log_timings || self.count_tokens {
            body["stream_options"] = json!({ "include_usage": true });
        }
        if self.llama_extensions {
            // Reuse the KV cache for the shared system-prompt prefix, and keep
            // "thinking" models (Qwen3-style templates) from reasoning aloud.
            body["cache_prompt"] = json!(true);
            body["chat_template_kwargs"] = json!({ "enable_thinking": false });
        } else if let Some(effort) = &self.reasoning_effort {
            // A thinking model behind a foreign server (Ollama, …) would otherwise spend `max_tokens` reasoning and
            // answer with nothing; Ollama maps "none" to "no thinking".
            body["reasoning_effort"] = json!(effort);
        }
        if let Some(keep) = &self.keep_alive {
            // Each request restarts the server's countdown, so the model stays while you play and leaves after.
            body["keep_alive"] = json!(keep);
        }
        self.auth.body(&mut body);
        body
    }

    /// Stream a chat completion. `on_delta` receives text as it is generated;
    /// the full text is returned at the end. Dropping the future cancels the
    /// request (the server stops generating when the connection closes).
    /// A reply cut off at `max_tokens` is returned as it is; use [`Self::stream_chat_reply`] to tell.
    pub async fn stream_chat(&self, messages: &[Message], opts: &ChatOpts, on_delta: &mut (dyn FnMut(&str) + Send)) -> Result<String> {
        self.stream_chat_reply(messages, opts, on_delta).await.map(|reply| reply.text)
    }

    /// Like [`Self::stream_chat`], but also says whether the reply stopped at the token limit.
    pub async fn stream_chat_reply(&self, messages: &[Message], opts: &ChatOpts, on_delta: &mut (dyn FnMut(&str) + Send)) -> Result<Reply> {
        self.stream_route(self.auth.chat_route(), &self.body(messages, opts), opts.log_timings, None, on_delta).await
    }

    pub(crate) async fn stream_warmup(&self, messages: &[Message]) -> Result<()> {
        let opts = ChatOpts { max_tokens: Some(1), ..Default::default() };
        self.stream_chat_reply(messages, &opts, &mut |_| {}).await.map(|_| ())
    }

    /// The one streaming loop behind every request: chat completions and llama.cpp's native `/completion`
    /// share the connect, header and per-chunk stall timeouts and the same idea of a finished reply.
    pub(crate) async fn stream_route(
        &self,
        route: &str,
        body: &Value,
        log_timings: bool,
        deadline: Option<Duration>,
        on_delta: &mut (dyn FnMut(&str) + Send),
    ) -> Result<Reply> {
        let _lease = self.model_lease().await?;
        let started = std::time::Instant::now();
        let url = format!("{}{route}", self.root);
        let mut body = body.clone();
        let resp = loop {
            if let Some(fields) = body.as_object_mut() {
                for field in self.rejected.lock().unwrap_or_else(|p| p.into_inner()).iter() {
                    fields.remove(*field);
                }
            }
            let request = self.auth.request(self.http.post(&url).json(&body))?;
            let request = match deadline.or(self.request_deadline) { Some(deadline) => request.timeout(deadline), None => request };
            let resp = tokio::time::timeout(self.idle_timeout, request.send())
                .await
                .map_err(|_| LlmError::Stalled(self.idle_timeout.as_secs()))?
                .map_err(|source| LlmError::Unreachable { url: self.root.clone(), source })?;
            let status = resp.status();
            if status.is_success() {
                break resp;
            }
            let text = tokio::time::timeout(self.idle_timeout, resp.text()).await
                .map_err(|_| LlmError::Stalled(self.idle_timeout.as_secs()))?
                .unwrap_or_default();
            // Hosted models refuse some optional fields (a reasoning model's temperature, Gemini 3's "no thinking",
            // Ollama's keep_alive elsewhere): leave the named field out for this server and ask once more.
            if status == reqwest::StatusCode::BAD_REQUEST && let Some(field) = rejected_field(&text, &body) {
                tracing::info!(server = %self.root, field, "the server does not accept this field; sending without it");
                self.rejected.lock().unwrap_or_else(|p| p.into_inner()).push(field);
                continue;
            }
            return Err(LlmError::Http { status: status.as_u16(), body: self.auth.redact(text).chars().take(400).collect() });
        };
        if log_timings {
            tracing::info!(headers_ms = started.elapsed().as_millis() as u64, "live model response");
        }

        let mut stream = resp.bytes_stream();
        let mut parser = SseParser::default();
        let mut reply = Reply::default();
        // Set once the server says why it stopped; a stream that just ends without it was cut short.
        let mut finished = false;
        // Some servers repeat the running token count in every chunk: the last one is the reply's.
        let mut usage: Option<(u64, u64)> = None;
        let count = |usage: Option<(u64, u64)>| {
            if let Some((input, output)) = usage {
                use std::sync::atomic::Ordering::Relaxed;
                self.used.replies.fetch_add(1, Relaxed);
                self.used.input.fetch_add(input, Relaxed);
                self.used.output.fetch_add(output, Relaxed);
            }
        };
        loop {
            let next = tokio::time::timeout(self.idle_timeout, stream.next())
                .await
                .map_err(|_| LlmError::Stalled(self.idle_timeout.as_secs()))?;
            let ended = next.is_none();
            let events = match next {
                Some(chunk) => parser.push(&chunk.map_err(|e| LlmError::Stream(e.to_string()))?),
                None => parser.finish(),
            };
            for event in events {
                match event {
                    SseEvent::Delta(text) => {
                        on_delta(&text);
                        reply.text.push_str(&text);
                    }
                    SseEvent::Error(e) => return Err(LlmError::Api(self.auth.redact(e))),
                    SseEvent::Finish(reason) => match reason.as_str() {
                        // The token cap is deliberate (live lines, deep dives…): keep the text, but say so.
                        "length" => { reply.capped = true; finished = true; }
                        "content_filter" => return Err(LlmError::Api("the server's content filter stopped the reply".into())),
                        // "stop", and other servers' names for a natural end ("eos_token", "end_turn"…).
                        _ => finished = true,
                    },
                    SseEvent::Timings(t) => {
                        if log_timings {
                            tracing::info!(prompt_ms = t.get("prompt_ms").and_then(|v| v.as_f64()),
                                decode_ms = t.get("predicted_ms").and_then(|v| v.as_f64()),
                                decode_tokens = t.get("predicted_n").and_then(|v| v.as_u64()),
                                tokens_per_second = t.get("predicted_per_second").and_then(|v| v.as_f64()),
                                cache_tokens = t.get("cache_n").and_then(|v| v.as_u64()), "live model timings");
                        }
                    }
                    SseEvent::Usage { input, output } => usage = Some((input, output)),
                    SseEvent::Done => {
                        count(usage);
                        return Ok(reply);
                    }
                }
            }
            if ended {
                // Some servers (and llama.cpp's `/completion`) never send `[DONE]`; a stop reason is enough.
                count(usage);
                return if finished { Ok(reply) } else { Err(LlmError::Stream("the reply ended before the model finished".into())) };
            }
        }
    }
}

/// A streamed reply. `capped` means the server stopped at the token limit, so its end may be missing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reply {
    pub text: String,
    pub capped: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SseEvent {
    Delta(String),
    Error(String),
    Done,
    Timings(Value),
    Finish(String),
    /// Token counts for the whole reply (the last chunk of a stream asked for with `stream_options.include_usage`).
    Usage { input: u64, output: u64 },
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
            events.extend(parse_line(&line));
        }
        events
    }

    pub fn finish(&mut self) -> Vec<SseEvent> {
        let rest = std::mem::take(&mut self.buf);
        parse_line(&rest)
    }
}

fn parse_line(line: &[u8]) -> Vec<SseEvent> {
    let line = match std::str::from_utf8(line) {
        Ok(line) => line,
        Err(_) => return vec![SseEvent::Error("invalid UTF-8 in a model response event".into())],
    };
    let Some(payload) = line.trim().strip_prefix("data:").map(str::trim) else { return Vec::new() };
    if payload.is_empty() { return Vec::new(); }
    if payload == "[DONE]" {
        return vec![SseEvent::Done];
    }
    let v: Value = match serde_json::from_str(payload) {
        Ok(Value::Object(object)) => Value::Object(object),
        _ => return vec![SseEvent::Error("malformed model response event".into())],
    };
    if let Some(err) = v.get("error") {
        let msg = err.get("message").and_then(Value::as_str).or_else(|| err.as_str()).unwrap_or("unknown error");
        return vec![SseEvent::Error(msg.to_string())];
    }
    let mut events = Vec::new();
    if let Some(choices) = v.get("choices") {
        let Some(choices) = choices.as_array() else { return vec![SseEvent::Error("malformed response choices".into())] };
        if let Some(choice) = choices.first() {
            if !choice.is_object() { return vec![SseEvent::Error("malformed response choice".into())]; }
            if let Some(delta) = choice.get("delta") {
                if !delta.is_object() { return vec![SseEvent::Error("malformed response delta".into())]; }
                if let Some(content) = delta.get("content").filter(|c| !c.is_null()) {
                    let Some(text) = content.as_str() else { return vec![SseEvent::Error("malformed response content".into())] };
                    if !text.is_empty() { events.push(SseEvent::Delta(text.to_string())); }
                }
            }
            if let Some(reason) = choice.get("finish_reason").filter(|r| !r.is_null()) {
                let Some(reason) = reason.as_str() else { return vec![SseEvent::Error("malformed response finish reason".into())] };
                events.push(SseEvent::Finish(reason.to_string()));
            }
        }
    }
    // llama.cpp's native `/completion` events: `content`, then a final `stop` with the reason in `stop_type`.
    if let Some(content) = v.get("content").filter(|c| !c.is_null()) {
        let Some(text) = content.as_str() else { return vec![SseEvent::Error("malformed completion content".into())] };
        if !text.is_empty() { events.push(SseEvent::Delta(text.to_string())); }
    }
    if v.get("stop").and_then(Value::as_bool) == Some(true) {
        let capped = v.get("truncated").and_then(Value::as_bool) == Some(true) || v.get("stop_type").and_then(Value::as_str) == Some("limit");
        events.push(SseEvent::Finish(if capped { "length" } else { "stop" }.into()));
    }
    if let Some(timings) = v.get("timings") { events.push(SseEvent::Timings(timings.clone())); }
    if let Some(usage) = v.get("usage").filter(|u| u.is_object()) {
        let count = |name: &str| usage.get(name).and_then(Value::as_u64);
        if let Some(input) = count("prompt_tokens") {
            // Some servers leave thinking out of `completion_tokens` but not out of the total.
            let output = count("completion_tokens").unwrap_or(0).max(count("total_tokens").map_or(0, |t| t.saturating_sub(input)));
            events.push(SseEvent::Usage { input, output });
        }
    }
    events
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
        pub headers: Arc<Mutex<Vec<String>>>,
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
        let headers = Arc::new(Mutex::new(Vec::new()));
        let handler = Arc::new(handler);
        let log = requests.clone();
        let header_log = headers.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let handler = handler.clone();
                let log = log.clone();
                let header_log = header_log.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 65536];
                    let mut total = 0;
                    let (path, body, request_head) = loop {
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
                                break (path, text[i + 4..i + 4 + len].to_string(), head.to_string());
                            }
                        }
                        if n == 0 {
                            return;
                        }
                    };
                    log.lock().unwrap().push((path.clone(), body.clone()));
                    header_log.lock().unwrap().push(request_head);
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
        MockServer { url, requests, headers }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn server_timings_split_across_chunks_do_not_become_translation_text() {
        let mut parser = super::SseParser::default();
        assert!(parser.push(b"data: {\"timings\":{\"prompt_ms\":2,").is_empty());
        let events = parser.push(b"\"predicted_ms\":5}}\n\n");
        assert!(matches!(&events[..], [super::SseEvent::Timings(t)] if t["predicted_ms"] == 5));
    }
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
        // Leave time for the connection error rather than the shorter header timeout.
        let client = LlmClient::new("http://127.0.0.1:1", &LlmConfig { request_timeout_secs: 5, ..cfg() });
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

    async fn body_sent_to_external(cfg: LlmConfig) -> Value {
        let server = spawn_mock(|_, _| MockResponse { status: 200, chunks: sse_answer(&["ok"]), delay_ms: 0 }).await;
        let cfg = LlmConfig { endpoint: Some(format!("{}/v1", server.url)), ..cfg };
        LlmClient::from_config(&cfg).stream_chat(&[Message::user("x")], &ChatOpts::default(), &mut |_| {}).await.unwrap();
        let sent = server.requests.lock().unwrap()[0].1.clone();
        serde_json::from_str(&sent).unwrap()
    }

    #[tokio::test]
    async fn an_external_server_is_asked_not_to_think_by_default() {
        let body = body_sent_to_external(cfg()).await;
        assert_eq!(body["reasoning_effort"], "none");
    }

    #[tokio::test]
    async fn the_reasoning_effort_can_be_changed_or_left_out_for_servers_that_reject_it() {
        let low = body_sent_to_external(LlmConfig { reasoning_effort: "low".into(), ..cfg() }).await;
        assert_eq!(low["reasoning_effort"], "low");
        let none_sent = body_sent_to_external(LlmConfig { reasoning_effort: "  ".into(), ..cfg() }).await;
        assert!(none_sent.get("reasoning_effort").is_none());
    }

    #[test]
    fn the_local_server_is_told_about_thinking_its_own_way() {
        let body = LlmClient::new("http://127.0.0.1:1", &cfg()).body(&[Message::user("x")], &ChatOpts::default());
        assert_eq!(body["chat_template_kwargs"]["enable_thinking"], false);
        assert!(body.get("reasoning_effort").is_none());
    }

    #[test]
    fn usage_metadata_is_only_requested_when_live_diagnostics_are_enabled() {
        let client = LlmClient::new("http://127.0.0.1:1", &cfg());
        let messages = [Message::user("x")];
        assert!(client.body(&messages, &ChatOpts::default()).get("stream_options").is_none());
        let live = client.body(&messages, &ChatOpts { log_timings: true, ..Default::default() });
        assert_eq!(live["stream_options"]["include_usage"], true);
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

    #[tokio::test]
    async fn a_server_that_accepts_but_never_sends_headers_times_out() {
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let root = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            std::future::pending::<()>().await;
        });
        let mut client = LlmClient::new(&root, &cfg());
        client.idle_timeout = Duration::from_millis(50);
        let result = tokio::time::timeout(Duration::from_millis(500), client.stream_chat(&[Message::user("x")], &ChatOpts::default(), &mut |_| {})).await;
        server.abort();
        assert!(matches!(result, Ok(Err(LlmError::Stalled(_)))), "{result:?}");
    }

    #[tokio::test]
    async fn incomplete_malformed_filtered_and_error_at_eof_streams_are_refused() {
        for tail in [
            "",
            "data: not-json\n\ndata: [DONE]\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"content_filter\"}]}\n\ndata: [DONE]\n\n",
            "data: {\"error\":{\"message\":\"failed at EOF\"}}",
            "data: {\"choices\":[{\"delta\":{\"content\":7}}]}\n\ndata: [DONE]\n\n",
        ] {
            let server = spawn_mock(move |_, _| MockResponse { status: 200, chunks: vec![sse_event("partial"), tail.as_bytes().to_vec()], delay_ms: 0 }).await;
            let client = LlmClient::new(&server.url, &cfg());
            let result = client.stream_chat(&[Message::user("x")], &ChatOpts::default(), &mut |_| {}).await;
            assert!(result.is_err(), "accepted {tail:?}: {result:?}");
        }
    }

    #[tokio::test]
    async fn a_final_delta_and_stop_reason_in_one_event_are_both_checked() {
        for reason in ["stop", "length"] {
            let server = spawn_mock(move |_, _| MockResponse { status: 200, chunks: vec![format!("data: {{\"choices\":[{{\"delta\":{{\"content\":\"Yes.\"}},\"finish_reason\":\"{reason}\"}}],\"timings\":{{\"predicted_ms\":1}}}}\n\ndata: [DONE]\n\n").into_bytes()], delay_ms: 0 }).await;
            let client = LlmClient::new(&server.url, &cfg());
            let reply = client.stream_chat_reply(&[Message::user("x")], &ChatOpts::default(), &mut |_| {}).await.unwrap();
            assert_eq!(reply, Reply { text: "Yes.".into(), capped: reason == "length" });
        }
    }

    #[tokio::test]
    async fn a_reply_cut_off_at_the_token_limit_keeps_its_text_and_is_marked_capped() {
        let server = spawn_mock(|_, _| MockResponse { status: 200, chunks: vec![sse_event("A long"), sse_event(" answer"), b"data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"length\"}]}\n\ndata: [DONE]\n\n".to_vec()], delay_ms: 0 }).await;
        let client = LlmClient::new(&server.url, &cfg());
        assert_eq!(client.stream_chat(&[Message::user("x")], &ChatOpts::default(), &mut |_| {}).await.unwrap(), "A long answer");
        let reply = client.stream_chat_reply(&[Message::user("x")], &ChatOpts::default(), &mut |_| {}).await.unwrap();
        assert!(reply.capped);
    }

    #[tokio::test]
    async fn servers_that_name_a_natural_end_differently_or_skip_the_done_marker_are_accepted() {
        for tail in [
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"eos_token\"}]}\n\ndata: [DONE]\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        ] {
            let server = spawn_mock(move |_, _| MockResponse { status: 200, chunks: vec![sse_event("Yes."), tail.as_bytes().to_vec()], delay_ms: 0 }).await;
            let reply = LlmClient::new(&server.url, &cfg()).stream_chat_reply(&[Message::user("x")], &ChatOpts::default(), &mut |_| {}).await.unwrap();
            assert_eq!(reply, Reply { text: "Yes.".into(), capped: false }, "{tail:?}");
        }
    }

    #[tokio::test]
    async fn token_counts_are_asked_for_only_by_a_counting_client_and_a_repeated_count_is_added_once() {
        // Like Gemini: the running count in every chunk; like OpenAI: once, in a last chunk without choices.
        let repeated = "data: {\"choices\":[{\"delta\":{\"content\":\"Yes\"}}],\"usage\":{\"prompt_tokens\":30,\"completion_tokens\":1,\"total_tokens\":31}}\n\n\
data: {\"choices\":[{\"delta\":{\"content\":\".\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":30,\"completion_tokens\":2,\"total_tokens\":52}}\n\ndata: [DONE]\n\n";
        let server = spawn_mock(move |_, _| MockResponse { status: 200, chunks: vec![repeated.as_bytes().to_vec()], delay_ms: 0 }).await;
        let plain = LlmClient::new(&server.url, &cfg());
        plain.stream_chat(&[Message::user("hi")], &ChatOpts::default(), &mut |_| {}).await.unwrap();
        assert!(!server.requests.lock().unwrap()[0].1.contains("stream_options"), "ordinary requests are unchanged");
        let counting = LlmClient::new(&server.url, &cfg()).counting_tokens();
        for _ in 0..2 {
            assert_eq!(counting.stream_chat(&[Message::user("hi")], &ChatOpts::default(), &mut |_| {}).await.unwrap(), "Yes.");
        }
        assert!(server.requests.lock().unwrap()[1].1.contains(r#""stream_options":{"include_usage":true}"#));
        // 22 output tokens: the total says more was generated (thinking) than the visible completion.
        assert_eq!(counting.tokens_used(), TokenUse { replies: 2, input: 60, output: 44 });
        assert_eq!(counting.clone().tokens_used().replies, 2, "clones share the count");
    }

    #[tokio::test]
    async fn the_models_a_server_offers_are_listed_by_name() {
        let server = spawn_mock(|path, _| {
            let body = if path == "/v1/models" { r#"{"data":[{"id":"models/gemini-flash"},{"id":"gemma"},{"id":"gemma"}]}"# } else { "{}" };
            MockResponse { status: 200, chunks: vec![body.as_bytes().to_vec()], delay_ms: 0 }
        })
        .await;
        assert_eq!(LlmClient::new(&server.url, &cfg()).list_models().await.unwrap(), ["gemini-flash", "gemma"]);
    }

    #[tokio::test]
    async fn a_server_model_is_kept_only_when_asked_and_released_without_loading_it() {
        let server = spawn_mock(|path, _| if path == "/api/generate" {
            MockResponse { status: 200, chunks: vec![br#"{"done":true,"done_reason":"unload"}"#.to_vec()], delay_ms: 0 }
        } else {
            MockResponse { status: 200, chunks: sse_answer(&["Hi."]), delay_ms: 0 }
        }).await;
        let off = LlmConfig { endpoint: Some(server.url.clone()), ..cfg() };
        LlmClient::from_config(&off).stream_chat(&[Message::user("x")], &ChatOpts::default(), &mut |_| {}).await.unwrap();
        let on = LlmConfig { endpoint: Some(server.url.clone()), server_keep_minutes: 20, model_name: "gemma".into(), ..cfg() };
        let client = LlmClient::from_config(&on);
        assert!(client.holds_server_model());
        client.hold_server_model().await.unwrap();
        client.release_server_model().await.unwrap();
        let requests = server.requests.lock().unwrap();
        assert!(!requests[0].1.contains("keep_alive"), "off sends nothing new: {}", requests[0].1);
        assert!(requests[1].1.contains(r#""keep_alive":"20m""#), "{}", requests[1].1);
        assert_eq!(requests[2].0, "/api/generate");
        let release: Value = serde_json::from_str(&requests[2].1).unwrap();
        assert_eq!(release, json!({ "model": "gemma", "keep_alive": 0 }), "no prompt, so an unloaded model is not loaded");
        // The local llama-server never gets the field, whatever the setting says.
        assert!(!LlmClient::from_config(&LlmConfig { server_keep_minutes: 20, ..cfg() }).holds_server_model());
    }

    #[tokio::test]
    async fn a_refused_optional_field_is_dropped_once_and_remembered_but_other_errors_are_not_retried() {
        let server = spawn_mock(|_, body| {
            if body.contains("\"temperature\"") {
                MockResponse { status: 400, chunks: vec![br#"{"error":{"message":"Unsupported parameter: 'temperature' is not supported with this model.","code":"unsupported_parameter"}}"#.to_vec()], delay_ms: 0 }
            } else {
                MockResponse { status: 200, chunks: sse_answer(&["Hi."]), delay_ms: 0 }
            }
        }).await;
        let client = LlmClient::from_config(&LlmConfig { endpoint: Some(server.url.clone()), ..cfg() });
        assert_eq!(client.stream_chat(&[Message::user("x")], &ChatOpts::default(), &mut |_| {}).await.unwrap(), "Hi.");
        assert_eq!(client.clone().stream_chat(&[Message::user("y")], &ChatOpts::default(), &mut |_| {}).await.unwrap(), "Hi.");
        assert_eq!(server.requests.lock().unwrap().len(), 3, "one refusal, one retry, then no more refusals");

        let failing = spawn_mock(|_, _| MockResponse { status: 400, chunks: vec![br#"{"error":{"message":"The model `nope` does not exist"}}"#.to_vec()], delay_ms: 0 }).await;
        let client = LlmClient::from_config(&LlmConfig { endpoint: Some(failing.url.clone()), ..cfg() });
        assert!(client.stream_chat(&[Message::user("x")], &ChatOpts::default(), &mut |_| {}).await.is_err());
        assert_eq!(failing.requests.lock().unwrap().len(), 1, "an error about something else is not retried");
    }

    #[tokio::test]
    async fn a_cloud_server_that_refuses_keep_alive_is_no_longer_held_or_released() {
        let server = spawn_mock(|_, body| if body.contains("keep_alive") {
            MockResponse { status: 400, chunks: vec![br#"{"error":{"message":"Unrecognized request argument supplied: keep_alive"}}"#.to_vec()], delay_ms: 0 }
        } else {
            MockResponse { status: 200, chunks: sse_answer(&["Hi."]), delay_ms: 0 }
        }).await;
        let client = LlmClient::from_config(&LlmConfig { endpoint: Some(server.url.clone()), server_keep_minutes: 15, ..cfg() });
        assert!(client.holds_server_model());
        client.hold_server_model().await.unwrap();
        assert!(!client.holds_server_model());
    }

    #[test]
    fn llama_native_completion_events_carry_text_and_say_whether_they_hit_the_limit() {
        let mut parser = SseParser::default();
        let events = parser.push(b"data: {\"content\":\"Up\",\"stop\":false}\n\ndata: {\"content\":\"\",\"stop\":true,\"stop_type\":\"eos\"}\n\n");
        assert_eq!(events, vec![SseEvent::Delta("Up".into()), SseEvent::Finish("stop".into())]);
        let events = parser.push(b"data: {\"content\":\"\",\"stop\":true,\"stop_type\":\"limit\"}\n\n");
        assert_eq!(events, vec![SseEvent::Finish("length".into())]);
    }

    #[tokio::test]
    async fn a_done_marker_without_a_trailing_newline_is_accepted() {
        let server = spawn_mock(|_, _| MockResponse { status: 200, chunks: vec![sse_event("Yes."), b"data: [DONE]".to_vec()], delay_ms: 0 }).await;
        assert_eq!(LlmClient::new(&server.url, &cfg()).stream_chat(&[Message::user("x")], &ChatOpts::default(), &mut |_| {}).await.unwrap(), "Yes.");
    }

    #[tokio::test]
    async fn regular_deltas_can_take_longer_than_one_idle_timeout_in_total() {
        let server = spawn_mock(|_, _| MockResponse { status: 200, chunks: sse_answer(&["a", "b", "c", "d", "e"]), delay_ms: 50 }).await;
        let mut client = LlmClient::new(&server.url, &cfg());
        client.idle_timeout = Duration::from_millis(150);
        assert_eq!(client.stream_chat(&[Message::user("x")], &ChatOpts::default(), &mut |_| {}).await.unwrap(), "abcde");
    }

    #[tokio::test]
    async fn an_error_response_with_a_body_that_never_finishes_times_out() {
        let server = spawn_mock(|_, _| MockResponse { status: 503, chunks: vec![b"loading".to_vec()], delay_ms: 500 }).await;
        let mut client = LlmClient::new(&server.url, &cfg());
        client.idle_timeout = Duration::from_millis(50);
        assert!(matches!(client.stream_chat(&[Message::user("x")], &ChatOpts::default(), &mut |_| {}).await, Err(LlmError::Stalled(_))));
    }
}
