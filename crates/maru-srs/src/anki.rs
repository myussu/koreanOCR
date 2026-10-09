//! Anki export: live through AnkiConnect, or as a tab-separated file that
//! Anki's "Import File" understands.

use std::io::Write;

use maru_core::config::AnkiConfig;
use serde_json::{Value, json};

use crate::store::{Context, WordRecord};
use crate::{Result, SrsError};

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Highlight the word inside its sentence (best effort: plain substring match).
fn highlight(sentence: &str, surface: &str) -> String {
    let s = esc(sentence);
    let w = esc(surface);
    if !w.is_empty() && s.contains(&w) { s.replacen(&w, &format!("<b>{w}</b>"), 1) } else { s }
}

/// (front, back) HTML for a card.
pub fn build_card(word: &WordRecord, contexts: &[Context]) -> (String, String) {
    let front = esc(&word.key);
    let mut back = String::new();
    if let Some(m) = &word.meaning {
        back.push_str(&format!("<div class=\"meaning\">{}</div>", esc(m)));
    }
    let mut meta: Vec<String> = Vec::new();
    if !word.pos.is_empty() {
        meta.push(esc(&word.pos));
    }
    if let Some(l) = &word.level {
        meta.push(esc(l));
    }
    if !meta.is_empty() {
        back.push_str(&format!("<div class=\"meta\">{}</div>", meta.join(" · ")));
    }
    for c in contexts.iter().take(2) {
        back.push_str(&format!("<hr><div class=\"ctx\">{}", highlight(&c.sentence, &word.surface)));
        if let Some(t) = &c.translation {
            back.push_str(&format!("<br><i>{}</i>", esc(t)));
        }
        back.push_str("</div>");
    }
    (front, back)
}

/// Write cards as TSV (front, back, tags) for Anki's file import.
pub fn export_tsv<W: Write>(mut out: W, cards: &[(WordRecord, Vec<Context>)]) -> Result<()> {
    for (word, contexts) in cards {
        let (front, back) = build_card(word, contexts);
        let one_line = |s: String| s.replace(['\t', '\n', '\r'], " ");
        writeln!(out, "{}\t{}\tmaru", one_line(front), one_line(back)).map_err(|e| SrsError::Other(e.to_string()))?;
    }
    Ok(())
}

pub struct AnkiClient {
    http: reqwest::Client,
    cfg: AnkiConfig,
}

impl AnkiClient {
    pub fn new(cfg: AnkiConfig) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .expect("static client configuration is valid");
        Self { http, cfg }
    }

    async fn call(&self, action: &str, params: Value) -> Result<Value> {
        let body = json!({ "action": action, "version": 6, "params": params });
        let resp: Value = self
            .http
            .post(&self.cfg.url)
            .json(&body)
            .send()
            .await
            .map_err(|e| SrsError::Anki(format!("cannot reach AnkiConnect at {}: {e}", self.cfg.url)))?
            .json()
            .await
            .map_err(|e| SrsError::Anki(format!("bad AnkiConnect response: {e}")))?;
        match resp.get("error") {
            Some(Value::String(e)) => Err(SrsError::Anki(e.clone())),
            _ => Ok(resp.get("result").cloned().unwrap_or(Value::Null)),
        }
    }

    /// AnkiConnect API version, proving Anki is running with the add-on.
    pub async fn ping(&self) -> Result<u64> {
        Ok(self.call("version", json!({})).await?.as_u64().unwrap_or(0))
    }

    /// Add a card. Returns `None` if Anki already has an identical front.
    pub async fn add(&self, word: &WordRecord, contexts: &[Context]) -> Result<Option<u64>> {
        self.call("createDeck", json!({ "deck": self.cfg.deck })).await?;
        let (front, back) = build_card(word, contexts);
        let note = json!({
            "deckName": self.cfg.deck,
            "modelName": self.cfg.note_type,
            "fields": { "Front": front, "Back": back },
            "options": { "allowDuplicate": false },
            "tags": ["maru"],
        });
        match self.call("addNote", json!({ "note": note })).await {
            Ok(v) => Ok(v.as_u64()),
            Err(SrsError::Anki(e)) if e.contains("duplicate") => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use maru_core::WordState;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;

    fn word() -> WordRecord {
        WordRecord {
            key: "공부하다".into(),
            surface: "공부했어요".into(),
            pos: "verb".into(),
            meaning: Some("to study <hard>".into()),
            level: Some("TOPIK 1".into()),
            lookups: 2,
            saved: true,
            marked_known: false,
            stability: None,
            difficulty: None,
            due: None,
            reps: 0,
            lapses: 0,
            state: WordState::Learning,
        }
    }

    fn ctx() -> Vec<Context> {
        vec![Context { sentence: "어제 공부했어요.".into(), translation: Some("I studied yesterday.".into()), ..Default::default() }]
    }

    #[test]
    fn card_escapes_html_and_highlights_the_word() {
        let (front, back) = build_card(&word(), &ctx());
        assert_eq!(front, "공부하다");
        assert!(back.contains("to study &lt;hard&gt;"));
        assert!(back.contains("어제 <b>공부했어요</b>."));
        assert!(back.contains("<i>I studied yesterday.</i>"));
        assert!(back.contains("verb · TOPIK 1"));
    }

    #[test]
    fn tsv_is_one_line_per_card() {
        let mut buf = Vec::new();
        export_tsv(&mut buf, &[(word(), ctx())]).unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert_eq!(text.lines().count(), 1);
        assert_eq!(text.lines().next().unwrap().split('\t').count(), 3);
    }

    /// Minimal AnkiConnect stand-in: answers each request with the next canned body.
    async fn fake_anki(replies: Vec<&'static str>) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = tokio::spawn(async move {
            let mut seen = Vec::new();
            for reply in replies {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = vec![0u8; 16384];
                let mut total = 0;
                loop {
                    let n = sock.read(&mut buf[total..]).await.unwrap();
                    total += n;
                    let text = String::from_utf8_lossy(&buf[..total]).to_string();
                    if let Some(i) = text.find("\r\n\r\n") {
                        let len = text[..i]
                            .lines()
                            .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap()))
                            .unwrap_or(0);
                        if total >= i + 4 + len {
                            seen.push(text[i + 4..i + 4 + len].to_string());
                            break;
                        }
                    }
                    if n == 0 {
                        break;
                    }
                }
                let resp = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    reply.len(),
                    reply
                );
                sock.write_all(resp.as_bytes()).await.unwrap();
            }
            seen
        });
        (url, handle)
    }

    #[tokio::test]
    async fn adds_a_note_through_ankiconnect() {
        let (url, server) = fake_anki(vec![r#"{"result":1,"error":null}"#, r#"{"result":1700000000000,"error":null}"#]).await;
        let client = AnkiClient::new(AnkiConfig { enabled: true, url, ..Default::default() });
        let id = client.add(&word(), &ctx()).await.unwrap();
        assert_eq!(id, Some(1_700_000_000_000));
        let seen = server.await.unwrap();
        assert!(seen[0].contains("createDeck"));
        assert!(seen[1].contains("addNote") && seen[1].contains("\"Front\":\"공부하다\""));
    }

    #[tokio::test]
    async fn duplicates_are_not_errors_but_other_failures_are() {
        let (url, _s) = fake_anki(vec![
            r#"{"result":1,"error":null}"#,
            r#"{"result":null,"error":"cannot create note because it is a duplicate"}"#,
            r#"{"result":1,"error":null}"#,
            r#"{"result":null,"error":"model was not found: Basic"}"#,
        ])
        .await;
        let client = AnkiClient::new(AnkiConfig { enabled: true, url, ..Default::default() });
        assert_eq!(client.add(&word(), &[]).await.unwrap(), None);
        let err = client.add(&word(), &[]).await.unwrap_err();
        assert!(err.to_string().contains("model was not found"));
    }

    #[tokio::test]
    async fn unreachable_anki_gives_a_helpful_error() {
        let client = AnkiClient::new(AnkiConfig { enabled: true, url: "http://127.0.0.1:1".into(), ..Default::default() });
        let err = client.ping().await.unwrap_err().to_string();
        assert!(err.contains("cannot reach AnkiConnect"), "{err}");
    }
}
