//! Anki: words go in through AnkiConnect (with a deck, tags, spoken audio and a reverse card), your
//! progress comes back out, and Anki is asked to sync with AnkiWeb. There is also a tab-separated
//! file export for Anki's "Import File".
//!
//! The AnkiConnect calls used here were checked against its source (version 6 API): `deckNames`,
//! `createDeck`, `changeDeck`, `modelNames`, `modelFieldNames`, `createModel`, `modelStyling`,
//! `updateModelTemplates`, `updateModelStyling`, `findNotes`, `notesInfo`, `addNote` (with `audio`),
//! `updateNoteFields`, `findCards`, `cardsInfo`, `sync`.

use std::collections::BTreeMap;
use std::io::Write;

use base64::Engine as _;
use maru_core::config::{AnkiConfig, ReverseMode};
use serde_json::{Value, json};

use crate::store::{Context, WordRecord};
use crate::{Result, SrsError};

/// Bump when the card layout changes: [`AnkiClient::ensure_model`] then updates the note type in place.
pub const MODEL_VERSION: u32 = 1;

/// The fields of the note type, first one being what Anki treats as the unique key.
pub const FIELDS: [&str; 12] = [
    "Word",
    "Meaning",
    "Pronunciation",
    "POS",
    "Level",
    "Sentence",
    "Translation",
    "Examples",
    "WordAudio",
    "SentenceAudio",
    "Source",
    "Reverse",
];

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn unesc(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

/// Highlight the word inside its sentence (best effort: plain substring match).
fn highlight(sentence: &str, surface: &str) -> String {
    let s = esc(sentence);
    let w = esc(surface);
    if !w.is_empty() && s.contains(&w) { s.replacen(&w, &format!("<b>{w}</b>"), 1) } else { s }
}

// ───────────────────────────── the file export ─────────────────────────────

/// (front, back) HTML for a card of the plain file export.
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

// ───────────────────────────── the note type ─────────────────────────────

fn css() -> String {
    format!(
        "/* maru-model-v{MODEL_VERSION} */\n\
.card {{ font-family: -apple-system, 'Segoe UI', 'Malgun Gothic', sans-serif; font-size: 20px; text-align: center; color: #1f2430; background: #fff; }}\n\
.nightMode .card {{ color: #e6e8ee; background: #1d2026; }}\n\
.ko {{ font-family: 'Malgun Gothic', 'Noto Sans KR', 'Apple SD Gothic Neo', sans-serif; }}\n\
.big {{ font-size: 2.1em; font-weight: 600; margin: 0.3em 0; }}\n\
.meaning {{ font-size: 1.4em; margin: 0.3em 0; }}\n\
.pron {{ color: #b45309; font-size: 0.85em; }}\n\
.pos, .level {{ display: inline-block; font-size: 0.65em; padding: 0 0.6em; margin: 0 0.15em; border-radius: 1em; background: #e8ebf3; color: #50566a; }}\n\
.nightMode .pos, .nightMode .level {{ background: #2c313c; color: #aab1c5; }}\n\
.ctx {{ margin-top: 1em; font-size: 1.05em; }}\n\
.tr {{ font-style: italic; opacity: 0.75; font-size: 0.85em; }}\n\
.ex {{ margin-top: 0.8em; font-size: 0.85em; opacity: 0.9; }}\n\
.ex div {{ margin: 0.4em 0; }}\n\
.src {{ margin-top: 1em; font-size: 0.65em; opacity: 0.55; }}\n\
b {{ color: #2563eb; }}\n\
.nightMode b {{ color: #6ea0ff; }}\n\
hr#answer {{ border: 0; border-top: 1px solid #c9cfdd; margin: 0.8em 0; }}\n"
    )
}

/// `(name, front, back)` of the two cards: Korean → English and, when the `Reverse` field is
/// filled, English → Korean.
fn templates() -> [(&'static str, &'static str, &'static str); 2] {
    [
        (
            "Korean → English",
            "<div class=\"ko big\">{{Word}}</div>\n{{WordAudio}}\n{{#Level}}<div><span class=\"level\">{{Level}}</span></div>{{/Level}}",
            "<div class=\"ko big\">{{Word}}</div>\n<hr id=answer>\n<div class=\"meaning\">{{Meaning}}</div>\n\
{{#Pronunciation}}<div class=\"pron\">[{{Pronunciation}}]</div>{{/Pronunciation}}\n\
{{#POS}}<span class=\"pos\">{{POS}}</span>{{/POS}}\n\
{{#Sentence}}<div class=\"ctx ko\">{{Sentence}}</div><div class=\"tr\">{{Translation}}</div>{{SentenceAudio}}{{/Sentence}}\n\
{{#Examples}}<div class=\"ex ko\">{{Examples}}</div>{{/Examples}}\n\
{{#Source}}<div class=\"src\">{{Source}}</div>{{/Source}}",
        ),
        (
            "English → Korean",
            "{{#Reverse}}<div class=\"meaning big\">{{Meaning}}</div>\n{{#POS}}<span class=\"pos\">{{POS}}</span>{{/POS}}{{/Reverse}}",
            "{{FrontSide}}\n<hr id=answer>\n<div class=\"ko big\">{{Word}}</div>{{WordAudio}}\n\
{{#Pronunciation}}<div class=\"pron\">[{{Pronunciation}}]</div>{{/Pronunciation}}\n\
{{#Sentence}}<div class=\"ctx ko\">{{Sentence}}</div><div class=\"tr\">{{Translation}}</div>{{SentenceAudio}}{{/Sentence}}\n\
{{#Examples}}<div class=\"ex ko\">{{Examples}}</div>{{/Examples}}\n\
{{#Source}}<div class=\"src\">{{Source}}</div>{{/Source}}",
        ),
    ]
}

// ───────────────────────────── what goes in and comes out ─────────────────────────────

/// A sound file to attach to a note.
#[derive(Debug, Clone, PartialEq)]
pub struct Media {
    pub filename: String,
    pub data: Vec<u8>,
}

impl Media {
    fn attachment(&self, field: &str) -> Value {
        json!({ "filename": self.filename, "data": base64::engine::general_purpose::STANDARD.encode(&self.data), "fields": [field] })
    }
}

/// One word to add.
#[derive(Debug, Clone)]
pub struct AddRequest {
    pub word: WordRecord,
    pub contexts: Vec<Context>,
    /// How the word sounds, e.g. `공부하다` → `공부하다`, `먹었어요` → `머거써요`.
    pub pronunciation: Option<String>,
    pub deck: String,
    pub tags: Vec<String>,
    pub reverse: ReverseMode,
    pub word_audio: Option<Media>,
    pub sentence_audio: Option<Media>,
    /// The program the word was met in, kept on the card.
    pub source: Option<String>,
}

/// A word Anki already has.
#[derive(Debug, Clone, PartialEq)]
pub struct ExistingNote {
    pub note_id: u64,
    /// The deck of its first card.
    pub deck: String,
    /// Days between reviews of that card, 0 while it is still being learned.
    pub interval_days: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Added {
    New { note_id: u64, deck: String, reverse_deck: Option<String>, cards: usize },
    Exists(ExistingNote),
}

/// How one of your cards stands in Anki.
#[derive(Debug, Clone, PartialEq)]
pub struct CardState {
    pub word: String,
    pub deck: String,
    pub interval_days: f32,
    /// Anki's card type: 0 new, 1 learning, 2 review, 3 relearning.
    pub card_type: i64,
    /// Anki's queue: negative means suspended or buried.
    pub queue: i64,
    pub reps: u32,
    pub lapses: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DueCounts {
    pub due: usize,
    pub new: usize,
}

/// Escape text for an Anki search: backslash, quote and the wildcard characters.
fn search_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | '"' | '*' | '_' | ':') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn str_of(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

pub struct AnkiClient {
    http: reqwest::Client,
    cfg: AnkiConfig,
}

impl AnkiClient {
    pub fn new(cfg: AnkiConfig) -> Self {
        let http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(20)).build().expect("static client configuration is valid");
        Self { http, cfg }
    }

    pub fn config(&self) -> &AnkiConfig {
        &self.cfg
    }

    async fn call(&self, action: &str, params: Value) -> Result<Value> {
        let mut body = json!({ "action": action, "version": 6, "params": params });
        if let Some(key) = self.cfg.api_key.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
            body["key"] = json!(key);
        }
        let resp: Value = self
            .http
            .post(&self.cfg.url)
            .json(&body)
            .send()
            .await
            .map_err(|e| SrsError::Anki(format!("cannot reach AnkiConnect at {}: {e}. Is Anki open, with the AnkiConnect add-on installed?", self.cfg.url)))?
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

    /// Every deck in your collection.
    pub async fn decks(&self) -> Result<Vec<String>> {
        let v = self.call("deckNames", json!({})).await?;
        let mut names: Vec<String> = v.as_array().map(|a| a.iter().map(str_of).collect()).unwrap_or_default();
        names.retain(|n| !n.is_empty());
        names.sort_by_key(|n| n.to_lowercase());
        Ok(names)
    }

    /// Make sure the note type exists and is the current layout. Creates it on first use; an older
    /// layout is updated in place; a note type of that name with the wrong fields is an error.
    pub async fn ensure_model(&self) -> Result<()> {
        let name = &self.cfg.note_type;
        let names = self.call("modelNames", json!({})).await?;
        let exists = names.as_array().is_some_and(|a| a.iter().any(|n| n.as_str() == Some(name)));
        let card_templates: Vec<Value> = templates().iter().map(|(n, f, b)| json!({ "Name": n, "Front": f, "Back": b })).collect();
        if !exists {
            self.call("createModel", json!({ "modelName": name, "inOrderFields": FIELDS, "css": css(), "isCloze": false, "cardTemplates": card_templates })).await?;
            return Ok(());
        }
        let fields = self.call("modelFieldNames", json!({ "modelName": name })).await?;
        let have: Vec<String> = fields.as_array().map(|a| a.iter().map(str_of).collect()).unwrap_or_default();
        let missing: Vec<&str> = FIELDS.iter().copied().filter(|f| !have.iter().any(|h| h == f)).collect();
        if !missing.is_empty() {
            return Err(SrsError::Anki(format!(
                "the note type \"{name}\" already exists in Anki but lacks the fields {}. Rename it there, or change [learning.anki] note_type",
                missing.join(", ")
            )));
        }
        let styling = self.call("modelStyling", json!({ "modelName": name })).await?;
        let stamp = format!("maru-model-v{MODEL_VERSION} ");
        if !str_of(&styling["css"]).contains(&stamp) {
            let by_name: serde_json::Map<String, Value> = templates().iter().map(|(n, f, b)| ((*n).to_string(), json!({ "Front": f, "Back": b }))).collect();
            self.call("updateModelTemplates", json!({ "model": { "name": name, "templates": by_name } })).await?;
            self.call("updateModelStyling", json!({ "model": { "name": name, "css": css() } })).await?;
        }
        Ok(())
    }

    fn fields_of(&self, req: &AddRequest) -> BTreeMap<&'static str, String> {
        let w = &req.word;
        let mut f: BTreeMap<&'static str, String> = FIELDS.iter().map(|k| (*k, String::new())).collect();
        f.insert("Word", esc(&w.key));
        f.insert("Meaning", w.meaning.as_deref().map(esc).unwrap_or_default());
        f.insert("Pronunciation", req.pronunciation.as_deref().map(esc).unwrap_or_default());
        f.insert("POS", esc(&w.pos));
        f.insert("Level", w.level.as_deref().map(esc).unwrap_or_default());
        if let Some(c) = req.contexts.first() {
            f.insert("Sentence", highlight(&c.sentence, &w.surface));
            f.insert("Translation", c.translation.as_deref().map(esc).unwrap_or_default());
        }
        f.insert("Source", req.source.as_deref().map(esc).unwrap_or_default());
        if req.reverse != ReverseMode::Off {
            f.insert("Reverse", "y".into());
        }
        f
    }

    /// The word's note, if Anki has one.
    pub async fn find(&self, word: &str) -> Result<Option<ExistingNote>> {
        let query = format!("\"note:{}\" \"Word:{}\"", search_text(&self.cfg.note_type), search_text(&esc(word)));
        let ids = self.call("findNotes", json!({ "query": query })).await?;
        let Some(note_id) = ids.as_array().and_then(|a| a.first()).and_then(Value::as_u64) else { return Ok(None) };
        let info = self.call("notesInfo", json!({ "notes": [note_id] })).await?;
        let first_card = info[0]["cards"][0].as_u64();
        let (mut deck, mut interval) = (String::new(), 0.0);
        if let Some(card) = first_card {
            let cards = self.call("cardsInfo", json!({ "cards": [card] })).await?;
            deck = str_of(&cards[0]["deckName"]);
            interval = cards[0]["interval"].as_f64().unwrap_or(0.0) as f32;
        }
        Ok(Some(ExistingNote { note_id, deck, interval_days: interval }))
    }

    /// Add a word, unless Anki has it already (then say where).
    pub async fn add(&self, req: &AddRequest) -> Result<Added> {
        self.ensure_model().await?;
        if let Some(existing) = self.find(&req.word.key).await? {
            return Ok(Added::Exists(existing));
        }
        self.call("createDeck", json!({ "deck": req.deck })).await?;
        let mut audio = Vec::new();
        if let Some(m) = &req.word_audio {
            audio.push(m.attachment("WordAudio"));
        }
        if let Some(m) = &req.sentence_audio {
            audio.push(m.attachment("SentenceAudio"));
        }
        let mut tags: Vec<String> = Vec::new();
        for t in req.tags.iter().map(|t| tag_of(t)).filter(|t| !t.is_empty()) {
            if !tags.contains(&t) {
                tags.push(t);
            }
        }
        let note = json!({
            "deckName": req.deck,
            "modelName": self.cfg.note_type,
            "fields": self.fields_of(req),
            "options": { "allowDuplicate": false },
            "tags": tags,
            "audio": audio,
        });
        let note_id = match self.call("addNote", json!({ "note": note })).await {
            Ok(v) => v.as_u64().ok_or_else(|| SrsError::Anki("Anki did not say which note it made".into()))?,
            Err(SrsError::Anki(e)) if e.contains("duplicate") => {
                return match self.find(&req.word.key).await? {
                    Some(existing) => Ok(Added::Exists(existing)),
                    None => Err(SrsError::Anki(e)),
                };
            }
            Err(e) => return Err(e),
        };
        // Cards come in template order: Korean → English, then English → Korean.
        let info = self.call("notesInfo", json!({ "notes": [note_id] })).await?;
        let cards: Vec<u64> = info[0]["cards"].as_array().map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
        let mut reverse_deck = None;
        if req.reverse == ReverseMode::Subdeck
            && let Some(&reverse_card) = cards.get(1)
        {
            let sub = format!("{}::{}", req.deck, self.cfg.reverse_subdeck.trim());
            self.call("changeDeck", json!({ "cards": [reverse_card], "deck": sub })).await?;
            reverse_deck = Some(sub);
        }
        Ok(Added::New { note_id, deck: req.deck.clone(), reverse_deck, cards: cards.len() })
    }

    /// Add another sentence to a word's card (its `Examples` field), with its audio. `false` if the
    /// card already shows that sentence.
    pub async fn add_example(&self, note_id: u64, sentence: &str, translation: Option<&str>, surface: &str, audio: Option<&Media>) -> Result<bool> {
        let info = self.call("notesInfo", json!({ "notes": [note_id] })).await?;
        let field = |name: &str| str_of(&info[0]["fields"][name]["value"]);
        let shown = unesc(&format!("{} {}", field("Sentence"), field("Examples")));
        if shown.replace("<b>", "").replace("</b>", "").contains(sentence) {
            return Ok(false);
        }
        let mut html = String::new();
        html.push_str(&format!("<div>{}", highlight(sentence, surface)));
        if let Some(t) = translation.filter(|t| !t.trim().is_empty()) {
            html.push_str(&format!("<br><span class=\"tr\">{}</span>", esc(t)));
        }
        html.push_str("</div>");
        let mut note = json!({ "id": note_id, "fields": { "Examples": format!("{}{html}", field("Examples")) } });
        if let Some(m) = audio {
            note["audio"] = json!([m.attachment("Examples")]);
        }
        self.call("updateNoteFields", json!({ "note": note })).await?;
        Ok(true)
    }

    /// Ask Anki to sync with AnkiWeb now. It uses the login stored in Anki and shows its own progress;
    /// AnkiConnect answers at once, before the sync is done.
    pub async fn sync(&self) -> Result<()> {
        self.call("sync", json!({})).await?;
        Ok(())
    }

    /// How every Korean → English card of the note type stands.
    pub async fn states(&self) -> Result<Vec<CardState>> {
        let query = format!("\"note:{}\" card:1", search_text(&self.cfg.note_type));
        let ids: Vec<u64> = self.call("findCards", json!({ "query": query })).await?.as_array().map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
        let mut out = Vec::new();
        for chunk in ids.chunks(200) {
            let infos = self.call("cardsInfo", json!({ "cards": chunk })).await?;
            for c in infos.as_array().into_iter().flatten() {
                let word = unesc(&str_of(&c["fields"]["Word"]["value"]));
                if word.is_empty() {
                    continue;
                }
                out.push(CardState {
                    word,
                    deck: str_of(&c["deckName"]),
                    interval_days: c["interval"].as_f64().unwrap_or(0.0) as f32,
                    card_type: c["type"].as_i64().unwrap_or(0),
                    queue: c["queue"].as_i64().unwrap_or(0),
                    reps: c["reps"].as_u64().unwrap_or(0) as u32,
                    lapses: c["lapses"].as_u64().unwrap_or(0) as u32,
                });
            }
        }
        Ok(out)
    }

    /// How many of your cards are due now, and how many are new.
    pub async fn due(&self) -> Result<DueCounts> {
        let count = |extra: &'static str| {
            let query = format!("\"note:{}\" {extra}", search_text(&self.cfg.note_type));
            async move { Ok::<usize, SrsError>(self.call("findCards", json!({ "query": query })).await?.as_array().map_or(0, Vec::len)) }
        };
        Ok(DueCounts { due: count("is:due").await?, new: count("is:new").await? })
    }
}

/// An Anki tag: no spaces (they separate tags), no stray quotes; `::` makes a hierarchy.
pub fn tag_of(raw: &str) -> String {
    let t: String = raw.trim().chars().map(|c| if c.is_whitespace() || matches!(c, '"' | '\'' | ',') { '_' } else { c }).collect();
    t.trim_matches(['_', ':']).to_string()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

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
            anki_interval: None,
            state: WordState::Learning,
        }
    }

    fn ctx() -> Vec<Context> {
        vec![Context { sentence: "어제 공부했어요.".into(), translation: Some("I studied yesterday.".into()), ..Default::default() }]
    }

    fn request(deck: &str, reverse: ReverseMode) -> AddRequest {
        AddRequest {
            word: word(),
            contexts: ctx(),
            pronunciation: Some("공부하다".into()),
            deck: deck.into(),
            tags: vec!["maru".into(), "topik 1".into(), "maru".into(), "game::Elden Ring".into()],
            reverse,
            word_audio: Some(Media { filename: "maru_w.wav".into(), data: vec![1, 2, 3] }),
            sentence_audio: Some(Media { filename: "maru_s.wav".into(), data: vec![4, 5] }),
            source: Some("eldenring.exe".into()),
        }
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

    #[test]
    fn tags_and_searches_are_made_safe() {
        assert_eq!(tag_of(" game::Elden Ring "), "game::Elden_Ring");
        assert_eq!(tag_of("a,b\"c"), "a_b_c");
        assert_eq!(tag_of("::"), "");
        assert_eq!(search_text(r#"a"b*c_d:e\f"#), r#"a\"b\*c\_d\:e\\f"#);
    }

    #[test]
    fn the_layout_names_every_field_it_uses_and_the_reverse_card_waits_for_its_flag() {
        let all: String = templates().iter().map(|(_, f, b)| format!("{f}{b}")).collect();
        for field in FIELDS {
            assert!(all.contains(&format!("{{{{{field}}}}}")) || all.contains(&format!("{{{{#{field}}}}}")), "{field} is on no card");
        }
        assert!(templates()[1].1.starts_with("{{#Reverse}}"), "no Reverse flag, no English → Korean card");
        assert!(css().starts_with(&format!("/* maru-model-v{MODEL_VERSION} */")));
    }

    // ───────── a stand-in AnkiConnect that remembers things ─────────

    #[derive(Default)]
    struct Collection {
        decks: Vec<String>,
        models: BTreeMap<String, Vec<String>>,
        styling: BTreeMap<String, String>,
        notes: Vec<(u64, Value, Vec<u64>)>, // id, note json, card ids
        cards: BTreeMap<u64, (String, f64)>, // card id → deck, interval
        next: u64,
    }

    type Log = Arc<Mutex<Vec<(String, Value, Option<String>)>>>;

    /// Serves AnkiConnect's JSON protocol on a free port; every call is logged with the key it carried.
    async fn fake_anki(col: Arc<Mutex<Collection>>) -> (String, Log) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let log: Log = Arc::default();
        let seen = log.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let (col, seen) = (col.clone(), seen.clone());
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 1 << 20];
                    let mut total = 0;
                    let body = loop {
                        let n = sock.read(&mut buf[total..]).await.unwrap_or(0);
                        total += n;
                        let text = String::from_utf8_lossy(&buf[..total]).to_string();
                        if let Some(i) = text.find("\r\n\r\n") {
                            let len = text[..i].lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap())).unwrap_or(0);
                            if total >= i + 4 + len {
                                break text[i + 4..i + 4 + len].to_string();
                            }
                        }
                        if n == 0 {
                            return;
                        }
                    };
                    let req: Value = serde_json::from_str(&body).unwrap();
                    let (action, params) = (str_of(&req["action"]), req["params"].clone());
                    seen.lock().unwrap().push((action.clone(), params.clone(), req["key"].as_str().map(str::to_string)));
                    let reply = match handle(&mut col.lock().unwrap(), &action, &params) {
                        Ok(v) => json!({ "result": v, "error": null }),
                        Err(e) => json!({ "result": null, "error": e }),
                    }
                    .to_string();
                    let resp = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}", reply.len());
                    let _ = sock.write_all(resp.as_bytes()).await;
                });
            }
        });
        (url, log)
    }

    fn handle(c: &mut Collection, action: &str, p: &Value) -> std::result::Result<Value, String> {
        Ok(match action {
            "version" => json!(6),
            "deckNames" => json!(c.decks),
            "createDeck" => {
                let d = str_of(&p["deck"]);
                if !c.decks.contains(&d) {
                    c.decks.push(d);
                }
                json!(1)
            }
            "modelNames" => json!(c.models.keys().collect::<Vec<_>>()),
            "modelFieldNames" => json!(c.models.get(&str_of(&p["modelName"])).ok_or("model was not found")?),
            "createModel" => {
                let name = str_of(&p["modelName"]);
                c.models.insert(name.clone(), p["inOrderFields"].as_array().unwrap().iter().map(str_of).collect());
                c.styling.insert(name, str_of(&p["css"]));
                json!({})
            }
            "modelStyling" => json!({ "css": c.styling.get(&str_of(&p["modelName"])).cloned().unwrap_or_default() }),
            "updateModelTemplates" => json!(null),
            "updateModelStyling" => {
                c.styling.insert(str_of(&p["model"]["name"]), str_of(&p["model"]["css"]));
                json!(null)
            }
            "findNotes" => {
                let q = str_of(&p["query"]);
                let ids: Vec<u64> = c.notes.iter().filter(|(_, n, _)| q.contains(&format!("Word:{}", str_of(&n["fields"]["Word"])))).map(|n| n.0).collect();
                json!(ids)
            }
            "notesInfo" => {
                let ids: Vec<u64> = p["notes"].as_array().unwrap().iter().filter_map(Value::as_u64).collect();
                json!(ids
                    .iter()
                    .map(|id| {
                        let (_, n, cards) = c.notes.iter().find(|n| n.0 == *id).unwrap();
                        let fields: serde_json::Map<String, Value> = n["fields"].as_object().unwrap().iter().map(|(k, v)| (k.clone(), json!({ "value": v, "order": 0 }))).collect();
                        json!({ "noteId": id, "fields": fields, "cards": cards, "tags": n["tags"] })
                    })
                    .collect::<Vec<_>>())
            }
            "addNote" => {
                let n = p["note"].clone();
                if !c.decks.contains(&str_of(&n["deckName"])) {
                    return Err(format!("deck was not found: {}", n["deckName"]));
                }
                c.next += 1;
                let id = 1000 + c.next;
                let reverse = n["fields"]["Reverse"].as_str().is_some_and(|r| !r.is_empty());
                let cards: Vec<u64> = if reverse { vec![id * 10, id * 10 + 1] } else { vec![id * 10] };
                for card in &cards {
                    c.cards.insert(*card, (str_of(&n["deckName"]), 0.0));
                }
                c.notes.push((id, n, cards));
                json!(id)
            }
            "changeDeck" => {
                for card in p["cards"].as_array().unwrap() {
                    c.cards.get_mut(&card.as_u64().unwrap()).unwrap().0 = str_of(&p["deck"]);
                }
                json!(null)
            }
            "cardsInfo" => json!(p["cards"]
                .as_array()
                .unwrap()
                .iter()
                .map(|id| {
                    let id = id.as_u64().unwrap();
                    let (deck, ivl) = c.cards.get(&id).cloned().unwrap();
                    let note = c.notes.iter().find(|n| n.2.contains(&id)).unwrap();
                    json!({ "cardId": id, "deckName": deck, "interval": ivl, "type": if ivl > 0.0 { 2 } else { 0 }, "queue": 0, "reps": 3, "lapses": 1,
                            "fields": { "Word": { "value": note.1["fields"]["Word"], "order": 0 } } })
                })
                .collect::<Vec<_>>()),
            "findCards" => {
                let q = str_of(&p["query"]);
                let want_first = q.contains("card:1");
                let ids: Vec<u64> = c.notes.iter().filter_map(|n| n.2.first().copied()).filter(|_| want_first || q.contains("is:")).collect();
                json!(ids)
            }
            "updateNoteFields" => {
                let id = p["note"]["id"].as_u64().unwrap();
                let note = c.notes.iter_mut().find(|n| n.0 == id).ok_or("note was not found")?;
                for (k, v) in p["note"]["fields"].as_object().unwrap() {
                    note.1["fields"][k] = v.clone();
                }
                json!(null)
            }
            "sync" => json!(null),
            other => return Err(format!("unsupported action {other}")),
        })
    }

    fn client(url: String) -> AnkiClient {
        AnkiClient::new(AnkiConfig { enabled: true, url, ..Default::default() })
    }

    fn calls(log: &Log) -> Vec<String> {
        log.lock().unwrap().iter().map(|c| c.0.clone()).collect()
    }

    #[tokio::test]
    async fn the_first_word_creates_the_note_type_and_the_deck_and_files_the_reverse_card_in_its_subdeck() {
        let col = Arc::new(Mutex::new(Collection::default()));
        let (url, log) = fake_anki(col.clone()).await;
        let anki = client(url);
        let added = anki.add(&request("Korean::Games::Elden Ring", ReverseMode::Subdeck)).await.unwrap();
        assert!(matches!(&added, Added::New { deck, reverse_deck: Some(r), cards: 2, .. } if deck == "Korean::Games::Elden Ring" && r == "Korean::Games::Elden Ring::Reverse"), "{added:?}");

        let c = col.lock().unwrap();
        assert_eq!(c.models["Maru Korean"], FIELDS);
        let (_, note, cards) = &c.notes[0];
        assert_eq!(note["fields"]["Word"], "공부하다");
        assert_eq!(note["fields"]["Meaning"], "to study &lt;hard&gt;");
        assert_eq!(note["fields"]["Sentence"], "어제 <b>공부했어요</b>.");
        assert_eq!(note["fields"]["Reverse"], "y");
        assert_eq!(note["fields"]["Source"], "eldenring.exe");
        assert_eq!(note["tags"], json!(["maru", "topik_1", "game::Elden_Ring"]), "cleaned and de-duplicated");
        let audio = note["audio"].as_array().unwrap();
        assert_eq!((audio[0]["fields"][0].as_str(), audio[1]["fields"][0].as_str()), (Some("WordAudio"), Some("SentenceAudio")));
        assert_eq!(audio[0]["data"], base64::engine::general_purpose::STANDARD.encode([1u8, 2, 3]));
        assert_eq!(c.cards[&cards[0]].0, "Korean::Games::Elden Ring", "Korean → English stays in the deck");
        assert_eq!(c.cards[&cards[1]].0, "Korean::Games::Elden Ring::Reverse", "English → Korean goes to the subdeck");
        drop(c);
        let order = calls(&log);
        let pos = |a: &str| order.iter().position(|x| x == a).unwrap_or_else(|| panic!("{a} was never called: {order:?}"));
        assert!(pos("createModel") < pos("createDeck") && pos("createDeck") < pos("addNote") && pos("addNote") < pos("changeDeck"), "{order:?}");
    }

    #[tokio::test]
    async fn reverse_cards_can_stay_in_the_deck_or_be_left_out() {
        let col = Arc::new(Mutex::new(Collection::default()));
        let (url, log) = fake_anki(col.clone()).await;
        let anki = client(url);
        anki.add(&request("Korean", ReverseMode::SameDeck)).await.unwrap();
        {
            let c = col.lock().unwrap();
            assert_eq!(c.notes[0].2.len(), 2);
            assert!(c.cards.values().all(|(deck, _)| deck == "Korean"));
        }
        assert!(!calls(&log).contains(&"changeDeck".to_string()));

        let mut other = request("Korean", ReverseMode::Off);
        other.word.key = "학교".into();
        anki.add(&other).await.unwrap();
        let c = col.lock().unwrap();
        assert_eq!(c.notes[1].2.len(), 1, "no Reverse flag, no second card");
        assert_eq!(c.notes[1].1["fields"]["Reverse"], "");
    }

    #[tokio::test]
    async fn a_word_that_is_already_there_is_reported_with_its_deck_and_not_added_again() {
        let col = Arc::new(Mutex::new(Collection::default()));
        let (url, log) = fake_anki(col.clone()).await;
        let anki = client(url);
        anki.add(&request("Korean::Book", ReverseMode::Subdeck)).await.unwrap();
        let again = anki.add(&request("Korean::Games", ReverseMode::Subdeck)).await.unwrap();
        let Added::Exists(found) = again else { panic!("expected Exists") };
        assert_eq!(found.deck, "Korean::Book");
        assert_eq!(col.lock().unwrap().notes.len(), 1);
        assert_eq!(calls(&log).iter().filter(|c| *c == "addNote").count(), 1);
    }

    #[tokio::test]
    async fn another_sentence_is_added_to_the_card_once() {
        let col = Arc::new(Mutex::new(Collection::default()));
        let (url, log) = fake_anki(col.clone()).await;
        let anki = client(url);
        let Added::New { note_id, .. } = anki.add(&request("Korean", ReverseMode::Off)).await.unwrap() else { panic!() };
        let audio = Media { filename: "maru_x.wav".into(), data: vec![9] };
        assert!(anki.add_example(note_id, "오늘도 공부했어요.", Some("I studied today too."), "공부했어요", Some(&audio)).await.unwrap());
        let examples = str_of(&col.lock().unwrap().notes[0].1["fields"]["Examples"]);
        assert!(examples.contains("오늘도 <b>공부했어요</b>.") && examples.contains("I studied today too."), "{examples}");
        assert!(!anki.add_example(note_id, "오늘도 공부했어요.", None, "공부했어요", None).await.unwrap(), "the same sentence is not added twice");
        assert!(!anki.add_example(note_id, "어제 공부했어요.", None, "공부했어요", None).await.unwrap(), "nor the one the card starts with");
        let updates: Vec<_> = log.lock().unwrap().iter().filter(|c| c.0 == "updateNoteFields").cloned().collect();
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].1["note"]["audio"][0]["fields"][0], "Examples");
    }

    #[tokio::test]
    async fn an_older_layout_is_upgraded_and_a_wrong_note_type_is_refused() {
        let col = Arc::new(Mutex::new(Collection::default()));
        col.lock().unwrap().models.insert("Maru Korean".into(), FIELDS.iter().map(|f| (*f).to_string()).collect());
        col.lock().unwrap().styling.insert("Maru Korean".into(), "/* maru-model-v0 */".into());
        let (url, log) = fake_anki(col.clone()).await;
        client(url).ensure_model().await.unwrap();
        assert!(calls(&log).contains(&"updateModelTemplates".to_string()) && calls(&log).contains(&"updateModelStyling".to_string()));
        assert!(col.lock().unwrap().styling["Maru Korean"].contains(&format!("maru-model-v{MODEL_VERSION}")));
        log.lock().unwrap().clear();
        client_again(&col).await.ensure_model().await.unwrap();

        let wrong = Arc::new(Mutex::new(Collection::default()));
        wrong.lock().unwrap().models.insert("Maru Korean".into(), vec!["Front".into(), "Back".into()]);
        let (url, _) = fake_anki(wrong).await;
        let err = client(url).ensure_model().await.unwrap_err().to_string();
        assert!(err.contains("lacks the fields") && err.contains("Word"), "{err}");
    }

    async fn client_again(col: &Arc<Mutex<Collection>>) -> AnkiClient {
        let (url, _) = fake_anki(col.clone()).await;
        client(url)
    }

    #[tokio::test]
    async fn how_your_cards_stand_comes_back_as_intervals() {
        let col = Arc::new(Mutex::new(Collection::default()));
        let (url, _) = fake_anki(col.clone()).await;
        let anki = client(url);
        anki.add(&request("Korean", ReverseMode::Subdeck)).await.unwrap();
        col.lock().unwrap().cards.get_mut(&(1001 * 10)).unwrap().1 = 30.0;
        let states = anki.states().await.unwrap();
        assert_eq!(states.len(), 1);
        assert_eq!((states[0].word.as_str(), states[0].deck.as_str(), states[0].interval_days, states[0].card_type), ("공부하다", "Korean", 30.0, 2));
        let due = anki.due().await.unwrap();
        assert_eq!(due, DueCounts { due: 1, new: 1 });
    }

    #[tokio::test]
    async fn the_api_key_goes_with_every_call_and_sync_is_a_plain_call() {
        let col = Arc::new(Mutex::new(Collection::default()));
        let (url, log) = fake_anki(col).await;
        let anki = AnkiClient::new(AnkiConfig { enabled: true, url, api_key: Some(" secret ".into()), ..Default::default() });
        anki.ping().await.unwrap();
        anki.sync().await.unwrap();
        let log = log.lock().unwrap();
        assert!(log.iter().all(|c| c.2.as_deref() == Some("secret")), "{log:?}");
        assert_eq!(log[1].0, "sync");
    }

    #[tokio::test]
    async fn decks_come_back_sorted_and_unreachable_anki_gives_a_helpful_error() {
        let col = Arc::new(Mutex::new(Collection::default()));
        col.lock().unwrap().decks = vec!["Zeta".into(), "alpha".into(), "Default".into()];
        let (url, _) = fake_anki(col).await;
        assert_eq!(client(url).decks().await.unwrap(), ["alpha", "Default", "Zeta"]);
        let err = client("http://127.0.0.1:1".into()).ping().await.unwrap_err().to_string();
        assert!(err.contains("cannot reach AnkiConnect") && err.contains("add-on"), "{err}");
    }
}
