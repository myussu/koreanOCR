//! A client for the OpenAI speech API: `POST {base}/v1/audio/speech`.
//!
//! The local `tts-server` speaks it, and so do many other servers and paid services, so one client
//! covers a model on this PC, one on another PC, and a cloud API. Only what the configuration sets
//! is sent, because strict servers reject fields they do not know.

use std::time::Duration;

use maru_core::config::TtsConfig;
use serde_json::{Value, json};

use crate::{Result, TtsError, cache};

/// Speech audio and what kind of file it is.
#[derive(Debug, Clone, PartialEq)]
pub struct Audio {
    pub bytes: Vec<u8>,
    /// `audio/wav`, `audio/mpeg`, …
    pub mime: String,
}

pub struct TtsClient {
    http: reqwest::Client,
    /// Up to and including `/v1`.
    root: String,
    api_key: Option<String>,
    model: String,
    voice: String,
    language: String,
    instructions: Option<String>,
    speed: f32,
}

/// `http://pc:8080`, `http://pc:8080/` and `http://pc:8080/v1` all mean the same server.
pub fn api_root(address: &str) -> String {
    let a = address.trim().trim_end_matches('/');
    let a = a.strip_suffix("/audio/speech").unwrap_or(a);
    if a.ends_with("/v1") { a.to_string() } else { format!("{a}/v1") }
}

impl TtsClient {
    pub fn new(address: &str, cfg: &TtsConfig) -> Self {
        let http = reqwest::Client::builder().timeout(Duration::from_secs(cfg.timeout_secs.max(5))).build().unwrap_or_default();
        Self {
            http,
            root: api_root(address),
            api_key: cfg.api_key.clone().map(|k| k.trim().to_string()).filter(|k| !k.is_empty()),
            model: cfg.model.clone(),
            voice: cfg.voice.clone(),
            language: cfg.language.trim().to_string(),
            instructions: cfg.instructions.clone().map(|i| i.trim().to_string()).filter(|i| !i.is_empty()),
            speed: cfg.speed,
        }
    }

    pub fn root(&self) -> &str {
        &self.root
    }

    /// The JSON sent for `text`.
    pub fn request_body(&self, text: &str) -> Value {
        let mut body = json!({ "model": self.model, "input": text, "voice": self.voice, "response_format": "wav" });
        let fields = body.as_object_mut().expect("an object");
        if !self.language.is_empty() {
            fields.insert("language".into(), json!(self.language));
        }
        if let Some(i) = &self.instructions {
            fields.insert("instructions".into(), json!(i));
        }
        if (self.speed - 1.0).abs() > 1e-3 {
            fields.insert("speed".into(), json!(self.speed));
        }
        body
    }

    /// Identifies how this client makes `text` sound, for the audio cache.
    pub fn cache_key(&self, text: &str) -> String {
        cache::key_of(&[&self.root, &self.model, &self.voice, &self.language, self.instructions.as_deref().unwrap_or(""), &format!("{:.2}", self.speed), text])
    }

    fn get(&self, path: &str) -> reqwest::RequestBuilder {
        let req = self.http.get(format!("{}{path}", self.root));
        match &self.api_key {
            Some(k) => req.bearer_auth(k),
            None => req,
        }
    }

    fn unreachable(&self, e: &reqwest::Error) -> TtsError {
        let detail = if e.is_timeout() { "it did not answer in time".to_string() } else if e.is_connect() { "connection refused or host not found".to_string() } else { e.to_string() };
        TtsError::Unreachable { url: self.root.clone(), detail }
    }

    /// Is a speech server there? Free: asks for the model list (or `/health`) only.
    pub async fn check(&self) -> Result<()> {
        let health = self.root.trim_end_matches("/v1").to_string() + "/health";
        if let Ok(r) = self.http.get(&health).timeout(Duration::from_secs(3)).send().await
            && r.status().is_success()
        {
            return Ok(());
        }
        match self.get("/models").timeout(Duration::from_secs(5)).send().await {
            Ok(r) if r.status().is_success() || r.status().as_u16() == 404 => Ok(()),
            Ok(r) if r.status().as_u16() == 401 || r.status().as_u16() == 403 => Err(TtsError::Http { status: r.status().as_u16(), body: "the server refused the API key".into() }),
            Ok(r) => Err(TtsError::Http { status: r.status().as_u16(), body: r.text().await.unwrap_or_default() }),
            Err(e) => Err(self.unreachable(&e)),
        }
    }

    /// Voice names the server offers (the local server lists its speakers). Empty if it does not say.
    pub async fn voices(&self) -> Vec<String> {
        let Ok(r) = self.get("/audio/voices").timeout(Duration::from_secs(5)).send().await else { return Vec::new() };
        let Ok(v) = r.json::<Value>().await else { return Vec::new() };
        voice_names(&v)
    }

    /// Speak `text`.
    pub async fn speak(&self, text: &str) -> Result<Audio> {
        let mut req = self.http.post(format!("{}/audio/speech", self.root)).json(&self.request_body(text));
        if let Some(k) = &self.api_key {
            req = req.bearer_auth(k);
        }
        let resp = req.send().await.map_err(|e| self.unreachable(&e))?;
        let status = resp.status();
        let header_mime = resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).map(str::to_string);
        let bytes = resp.bytes().await.map_err(|e| self.unreachable(&e))?.to_vec();
        if !status.is_success() {
            return Err(TtsError::Http { status: status.as_u16(), body: error_message(&bytes) });
        }
        audio_from(bytes, header_mime.as_deref())
    }
}

/// What kind of audio `bytes` are, from their first bytes (a header can lie or be missing).
fn audio_from(bytes: Vec<u8>, header: Option<&str>) -> Result<Audio> {
    let mime = if bytes.starts_with(b"RIFF") {
        "audio/wav"
    } else if bytes.starts_with(b"ID3") || (bytes.len() > 2 && bytes[0] == 0xFF && bytes[1] & 0xE0 == 0xE0) {
        "audio/mpeg"
    } else if bytes.starts_with(b"OggS") {
        "audio/ogg"
    } else if bytes.starts_with(b"fLaC") {
        "audio/flac"
    } else if bytes.is_empty() {
        return Err(TtsError::BadAudio("an empty reply".into()));
    } else {
        let shown = error_message(&bytes);
        let kind = header.unwrap_or("an unknown type");
        return Err(TtsError::BadAudio(format!("something that is not audio ({kind}): {shown}")));
    };
    Ok(Audio { bytes, mime: mime.into() })
}

/// The `error.message` of an API error body, or the start of whatever it said.
fn error_message(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    if let Ok(v) = serde_json::from_str::<Value>(&text) {
        let message = v.pointer("/error/message").or_else(|| v.get("message")).or_else(|| v.get("detail")).and_then(Value::as_str);
        if let Some(m) = message {
            return m.to_string();
        }
    }
    text.chars().take(200).collect()
}

fn voice_names(v: &Value) -> Vec<String> {
    let list = v.get("voices").or_else(|| v.get("data")).unwrap_or(v);
    list.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| match x {
                    Value::String(s) => Some(s.clone()),
                    Value::Object(o) => ["name", "id", "voice_id"].iter().find_map(|k| o.get(*k).and_then(Value::as_str)).map(str::to_string),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn addresses_are_normalised() {
        for a in ["http://pc:8080", "http://pc:8080/", "http://pc:8080/v1", "http://pc:8080/v1/", "http://pc:8080/v1/audio/speech"] {
            assert_eq!(api_root(a), "http://pc:8080/v1", "{a}");
        }
        assert_eq!(api_root("https://api.openai.com/v1"), "https://api.openai.com/v1");
    }

    fn client(f: impl FnOnce(&mut TtsConfig)) -> TtsClient {
        let mut cfg = TtsConfig::default();
        f(&mut cfg);
        TtsClient::new("http://127.0.0.1:1", &cfg)
    }

    #[test]
    fn only_what_is_configured_is_sent() {
        let plain = client(|_| {}).request_body("안녕");
        assert_eq!(plain, json!({"model": "qwen3-tts", "input": "안녕", "voice": "sohee", "response_format": "wav"}));
        let full = client(|c| {
            c.language = " Korean ".into();
            c.instructions = Some("speak slowly".into());
            c.speed = 0.8;
        })
        .request_body("안녕");
        assert_eq!(full["language"], "Korean");
        assert_eq!(full["instructions"], "speak slowly");
        assert!((full["speed"].as_f64().unwrap() - 0.8).abs() < 1e-6);
        assert!(client(|c| c.instructions = Some("  ".into())).request_body("x").get("instructions").is_none());
    }

    #[test]
    fn the_cache_key_changes_with_anything_audible_and_ignores_the_api_key() {
        let base = client(|_| {}).cache_key("안녕");
        assert_eq!(base, client(|c| c.api_key = Some("secret".into())).cache_key("안녕"));
        assert_ne!(base, client(|c| c.voice = "other".into()).cache_key("안녕"));
        assert_ne!(base, client(|c| c.speed = 0.9).cache_key("안녕"));
        assert_ne!(base, client(|_| {}).cache_key("안녕하세요"));
    }

    #[test]
    fn replies_are_recognised_by_their_bytes() {
        assert_eq!(audio_from(crate::wav::encode_pcm16(&[0; 4], 24_000), Some("application/octet-stream")).unwrap().mime, "audio/wav");
        assert_eq!(audio_from(b"ID3\x03rest".to_vec(), None).unwrap().mime, "audio/mpeg");
        assert_eq!(audio_from(vec![0xFF, 0xFB, 0x90], None).unwrap().mime, "audio/mpeg");
        assert_eq!(audio_from(b"OggS....".to_vec(), None).unwrap().mime, "audio/ogg");
        let err = audio_from(br#"{"error":{"message":"bad voice"}}"#.to_vec(), Some("application/json")).unwrap_err().to_string();
        assert!(err.contains("not audio") && err.contains("bad voice"), "{err}");
        assert!(audio_from(Vec::new(), None).is_err());
    }

    #[test]
    fn voice_lists_come_in_several_shapes() {
        assert_eq!(voice_names(&json!({"voices": [{"name": "sohee", "kind": "speaker"}, {"name": "mine", "kind": "registered"}]})), ["sohee", "mine"]);
        assert_eq!(voice_names(&json!({"data": [{"id": "alloy"}, {"voice_id": "nova"}]})), ["alloy", "nova"]);
        assert_eq!(voice_names(&json!(["a", "b"])), ["a", "b"]);
        assert!(voice_names(&json!({"nope": 1})).is_empty());
    }

    /// A one-shot HTTP server on a free port that answers with `reply` and reports what it was sent.
    fn serve(reply: Vec<u8>, content_type: &'static str, status: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = vec![0u8; 16384];
            let mut got = Vec::new();
            loop {
                let n = s.read(&mut buf).unwrap();
                got.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&got).to_string();
                if let Some(split) = text.find("\r\n\r\n") {
                    let len = text[..split].lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap())).unwrap_or(0);
                    if got.len() >= split + 4 + len {
                        break;
                    }
                }
            }
            let head = format!("HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", reply.len());
            s.write_all(head.as_bytes()).unwrap();
            s.write_all(&reply).unwrap();
            String::from_utf8_lossy(&got).to_string()
        });
        (url, handle)
    }

    #[tokio::test]
    async fn speaks_to_a_server_and_authenticates() {
        let wav = crate::wav::encode_pcm16(&vec![100; 2400], 24_000);
        let (url, seen) = serve(wav.clone(), "audio/wav", "200 OK");
        let cfg = TtsConfig { api_key: Some("sk-test".into()), ..Default::default() };
        let audio = TtsClient::new(&url, &cfg).speak("안녕하세요").await.unwrap();
        assert_eq!(audio.bytes, wav);
        let request = seen.join().unwrap();
        assert!(request.starts_with("POST /v1/audio/speech "), "{request}");
        assert!(request.to_ascii_lowercase().contains("authorization: bearer sk-test"), "{request}");
        assert!(request.contains("안녕하세요") && request.contains("\"voice\":\"sohee\""), "{request}");
    }

    #[tokio::test]
    async fn an_error_reply_says_what_the_server_said() {
        let (url, _) = serve(br#"{"error":{"message":"voice 'x' not found"}}"#.to_vec(), "application/json", "400 Bad Request");
        let err = TtsClient::new(&url, &TtsConfig::default()).speak("a").await.unwrap_err();
        assert!(matches!(&err, TtsError::Http { status: 400, body } if body.contains("not found")), "{err}");
    }

    #[tokio::test]
    async fn a_dead_server_is_reported_as_unreachable() {
        let err = TtsClient::new("http://127.0.0.1:1", &TtsConfig::default()).speak("a").await.unwrap_err();
        assert!(matches!(err, TtsError::Unreachable { .. }), "{err}");
        assert!(TtsClient::new("http://127.0.0.1:1", &TtsConfig::default()).check().await.is_err());
    }
}
