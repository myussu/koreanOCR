//! Sentence cards: a line you met in a game, in Korean with its sound, on the front; its translation and the words you
//! picked from it on the back. A note type of its own (`learning.anki.sentence_note_type`), so word progress
//! ([`AnkiClient::states`]) never counts sentences, and it has no reverse card.

use serde_json::{Value, json};

use crate::anki::{AnkiClient, ExistingNote, Media, esc, search_text, str_of, tag_of};
use crate::{Result, SrsError};

/// Bump when the sentence card layout changes: [`AnkiClient::ensure_sentence_model`] then updates it in place.
pub const SENTENCE_MODEL_VERSION: u32 = 1;

/// The fields of the sentence note type; the sentence is what Anki treats as the unique key.
pub const SENTENCE_FIELDS: [&str; 5] = ["Sentence", "Translation", "SentenceAudio", "Words", "Source"];

/// A word picked from the sentence, listed on the back of its card.
#[derive(Debug, Clone, PartialEq)]
pub struct SentenceWord {
    pub word: String,
    pub meaning: String,
}

#[derive(Debug, Clone)]
pub struct SentenceRequest {
    pub sentence: String,
    pub translation: String,
    pub words: Vec<SentenceWord>,
    pub deck: String,
    pub tags: Vec<String>,
    pub audio: Option<Media>,
    /// The program the line was met in, kept on the card.
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SentenceAdded {
    New { note_id: u64, deck: String },
    Exists(ExistingNote),
}

fn sentence_css() -> String {
    format!(
        "/* maru-sentence-v{SENTENCE_MODEL_VERSION} */\n\
.card {{ font-family: -apple-system, 'Segoe UI', 'Malgun Gothic', sans-serif; font-size: 20px; text-align: center; color: #1f2430; background: #fff; }}\n\
.nightMode .card {{ color: #e6e8ee; background: #1d2026; }}\n\
.ko {{ font-family: 'Malgun Gothic', 'Noto Sans KR', 'Apple SD Gothic Neo', sans-serif; }}\n\
.line {{ font-size: 1.5em; font-weight: 600; line-height: 1.45; margin: 0.4em 0.5em; }}\n\
.meaning {{ font-size: 1.15em; margin: 0.4em 0.5em; }}\n\
.words {{ display: inline-block; text-align: left; margin-top: 0.9em; font-size: 0.9em; }}\n\
.words div {{ margin: 0.25em 0; }}\n\
.words b {{ color: #2563eb; font-weight: 600; }}\n\
.nightMode .words b {{ color: #6ea0ff; }}\n\
.src {{ margin-top: 1em; font-size: 0.65em; opacity: 0.55; }}\n\
hr#answer {{ border: 0; border-top: 1px solid #c9cfdd; margin: 0.8em 0; }}\n"
    )
}

/// `(name, front, back)` of the one card: read (and hear) the Korean line, then check its meaning.
fn sentence_template() -> (&'static str, &'static str, &'static str) {
    (
        "Sentence",
        "<div class=\"ko line\">{{Sentence}}</div>\n{{SentenceAudio}}",
        "{{FrontSide}}\n<hr id=answer>\n<div class=\"meaning\">{{Translation}}</div>\n\
{{#Words}}<div class=\"words ko\">{{Words}}</div>{{/Words}}\n\
{{#Source}}<div class=\"src\">{{Source}}</div>{{/Source}}",
    )
}

/// The back's word list: `<div><b>공부하다</b> — to study</div>…`.
pub fn words_html(words: &[SentenceWord]) -> String {
    words
        .iter()
        .filter(|w| !w.word.trim().is_empty())
        .map(|w| match w.meaning.trim() {
            "" => format!("<div><b>{}</b></div>", esc(w.word.trim())),
            meaning => format!("<div><b>{}</b> — {}</div>", esc(w.word.trim()), esc(meaning)),
        })
        .collect()
}

impl AnkiClient {
    fn sentence_model(&self) -> &str {
        &self.config().sentence_note_type
    }

    /// Make sure the sentence note type exists and is the current layout (see [`AnkiClient::ensure_model`]).
    pub async fn ensure_sentence_model(&self) -> Result<()> {
        let name = self.sentence_model();
        if name.trim().is_empty() {
            return Err(SrsError::Anki("Choose a note type name for sentence cards in Settings → Anki".into()));
        }
        let (template_name, front, back) = sentence_template();
        let names = self.call("modelNames", json!({})).await?;
        if !names.as_array().is_some_and(|a| a.iter().any(|n| n.as_str() == Some(name))) {
            self.call(
                "createModel",
                json!({ "modelName": name, "inOrderFields": SENTENCE_FIELDS, "css": sentence_css(), "isCloze": false,
                        "cardTemplates": [{ "Name": template_name, "Front": front, "Back": back }] }),
            )
            .await?;
            return Ok(());
        }
        let fields = self.call("modelFieldNames", json!({ "modelName": name })).await?;
        let have: Vec<String> = fields.as_array().map(|a| a.iter().map(str_of).collect()).unwrap_or_default();
        let missing: Vec<&str> = SENTENCE_FIELDS.iter().copied().filter(|f| !have.iter().any(|h| h == f)).collect();
        if !missing.is_empty() {
            return Err(SrsError::Anki(format!(
                "the note type \"{name}\" already exists in Anki but lacks the fields {}. Rename it there, or change [learning.anki] sentence_note_type",
                missing.join(", ")
            )));
        }
        let styling = self.call("modelStyling", json!({ "modelName": name })).await?;
        if !str_of(&styling["css"]).contains(&format!("maru-sentence-v{SENTENCE_MODEL_VERSION} ")) {
            self.call("updateModelTemplates", json!({ "model": { "name": name, "templates": { template_name: { "Front": front, "Back": back } } } })).await?;
            self.call("updateModelStyling", json!({ "model": { "name": name, "css": sentence_css() } })).await?;
        }
        Ok(())
    }

    /// The sentence's note, if Anki has one.
    pub async fn find_sentence(&self, sentence: &str) -> Result<Option<ExistingNote>> {
        let query = format!("\"note:{}\" \"Sentence:{}\"", search_text(self.sentence_model()), search_text(&esc(sentence.trim())));
        self.first_note(&query).await
    }

    /// Add a sentence card, unless Anki has that sentence already (then say where).
    pub async fn add_sentence(&self, req: &SentenceRequest) -> Result<SentenceAdded> {
        self.ensure_sentence_model().await?;
        if let Some(existing) = self.find_sentence(&req.sentence).await? {
            return Ok(SentenceAdded::Exists(existing));
        }
        self.call("createDeck", json!({ "deck": req.deck })).await?;
        let mut tags: Vec<String> = Vec::new();
        for t in req.tags.iter().map(|t| tag_of(t)).filter(|t| !t.is_empty()) {
            if !tags.contains(&t) {
                tags.push(t);
            }
        }
        let audio: Vec<Value> = req.audio.iter().map(|m| m.attachment("SentenceAudio")).collect();
        let note = json!({
            "deckName": req.deck,
            "modelName": self.sentence_model(),
            "fields": {
                "Sentence": esc(req.sentence.trim()),
                "Translation": esc(req.translation.trim()),
                "SentenceAudio": "",
                "Words": words_html(&req.words),
                "Source": req.source.as_deref().map(esc).unwrap_or_default(),
            },
            "options": { "allowDuplicate": false },
            "tags": tags,
            "audio": audio,
        });
        match self.call("addNote", json!({ "note": note })).await {
            Ok(v) => {
                let note_id = v.as_u64().ok_or_else(|| SrsError::Anki("Anki did not say which note it made".into()))?;
                Ok(SentenceAdded::New { note_id, deck: req.deck.clone() })
            }
            Err(SrsError::Anki(e)) if e.contains("duplicate") => match self.find_sentence(&req.sentence).await? {
                Some(existing) => Ok(SentenceAdded::Exists(existing)),
                None => Err(SrsError::Anki(e)),
            },
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_back_lists_the_picked_words_with_their_meanings_escaped() {
        let words = [
            SentenceWord { word: "공부하다".into(), meaning: "to study <hard>".into() },
            SentenceWord { word: " 학교 ".into(), meaning: String::new() },
            SentenceWord { word: " ".into(), meaning: "nothing".into() },
        ];
        assert_eq!(words_html(&words), "<div><b>공부하다</b> — to study &lt;hard&gt;</div><div><b>학교</b></div>");
    }

    #[test]
    fn the_sentence_card_uses_every_field_and_has_no_reverse() {
        let (_, front, back) = sentence_template();
        for field in SENTENCE_FIELDS {
            assert!(format!("{front}{back}").contains(&format!("{{{{{field}}}}}")), "{field} is on no card");
        }
        assert!(sentence_css().starts_with(&format!("/* maru-sentence-v{SENTENCE_MODEL_VERSION} */")));
    }
}
