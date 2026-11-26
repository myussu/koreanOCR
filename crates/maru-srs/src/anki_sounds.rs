//! Sound for Maru's cards that have none, because speech was off or failed when they were added. Anki is searched for
//! Maru's notes with an empty sound field, so nothing has to be remembered here; only empty fields are ever filled.

use serde_json::{Value, json};

use crate::anki::{AnkiClient, Media, search_text, str_of, unesc};
use crate::Result;

/// A Maru note missing a sound, with the text to speak for each empty field.
#[derive(Debug, Clone, PartialEq)]
pub struct SilentNote {
    pub note_id: u64,
    /// For `WordAudio` (word notes).
    pub word: Option<String>,
    /// For `SentenceAudio` (word notes with a sentence, and sentence notes).
    pub sentence: Option<String>,
}

/// How many notes lack a sound, and the first ones to fill.
#[derive(Debug, Clone, PartialEq)]
pub struct SilentNotes {
    pub total: usize,
    pub notes: Vec<SilentNote>,
}

/// A field's text as spoken: no markup (`<b>…</b>` around the word), entities read back, spacing tidied.
pub fn plain(field: &str) -> String {
    let mut out = String::with_capacity(field.len());
    let mut in_tag = false;
    for c in field.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    unesc(&out.replace("&nbsp;", " ")).split_whitespace().collect::<Vec<_>>().join(" ")
}

fn empty(fields: &Value, name: &str) -> bool {
    fields.get(name).is_some_and(|f| str_of(&f["value"]).trim().is_empty())
}

impl AnkiClient {
    fn silent_query(&self) -> String {
        let words = search_text(&self.config().note_type);
        let sentences = search_text(&self.config().sentence_note_type);
        // `Field:` matches an empty field, `Field:_*` a filled one (Anki's search syntax).
        format!("(\"note:{words}\" (WordAudio: OR (SentenceAudio: Sentence:_*))) OR (\"note:{sentences}\" SentenceAudio:)")
    }

    /// Maru's notes that lack a sound: how many, and up to `limit` of them with what to say.
    pub async fn silent_notes(&self, limit: usize) -> Result<SilentNotes> {
        let ids = self.call("findNotes", json!({ "query": self.silent_query() })).await?;
        let ids: Vec<u64> = ids.as_array().map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
        let total = ids.len();
        let first: Vec<u64> = ids.into_iter().take(limit).collect();
        if first.is_empty() {
            return Ok(SilentNotes { total, notes: Vec::new() });
        }
        let info = self.call("notesInfo", json!({ "notes": first })).await?;
        let notes = info
            .as_array()
            .map(|a| a.as_slice())
            .unwrap_or_default()
            .iter()
            .filter_map(|n| {
                let fields = &n["fields"];
                let text = |name: &str| fields.get(name).map(|f| plain(&str_of(&f["value"]))).filter(|t| !t.is_empty());
                let word = text("Word").filter(|_| empty(fields, "WordAudio"));
                let sentence = text("Sentence").filter(|_| empty(fields, "SentenceAudio"));
                (word.is_some() || sentence.is_some()).then(|| SilentNote { note_id: n["noteId"].as_u64().unwrap_or_default(), word, sentence })
            })
            .filter(|n| n.note_id != 0)
            .collect();
        Ok(SilentNotes { total, notes })
    }

    /// Attach sounds to a note's empty sound fields. A field that has a sound by now (added by hand meanwhile) is kept.
    pub async fn add_sounds(&self, note_id: u64, word: Option<&Media>, sentence: Option<&Media>) -> Result<bool> {
        let info = self.call("notesInfo", json!({ "notes": [note_id] })).await?;
        let fields = &info[0]["fields"];
        let (mut audio, mut filled) = (Vec::new(), serde_json::Map::new());
        for (media, name) in [(word, "WordAudio"), (sentence, "SentenceAudio")] {
            if let Some(m) = media.filter(|_| empty(fields, name)) {
                audio.push(m.attachment(name));
                // AnkiConnect attaches a sound only to a field the update also names; with `fields: {}` it answers
                // success and attaches nothing (seen on a real Anki). The field is empty, so naming it changes nothing else.
                filled.insert(name.to_string(), json!(""));
            }
        }
        if audio.is_empty() {
            return Ok(false);
        }
        self.call("updateNoteFields", json!({ "note": { "id": note_id, "fields": filled, "audio": audio } })).await?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_field_is_spoken_without_its_markup() {
        assert_eq!(plain("저는 <b>학교에서</b>&nbsp;공부했어요"), "저는 학교에서 공부했어요");
        assert_eq!(plain("a &lt;b&gt; &amp; c"), "a <b> & c");
        assert_eq!(plain("  "), "");
    }
}
