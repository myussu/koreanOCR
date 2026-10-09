use serde::{Deserialize, Serialize};

use crate::types::{Capture, Direction, Eojeol, Gloss, Register};

/// Which text a breakdown/gloss refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    /// The text the user selected (Korean in `KoToEn`).
    Source,
    /// The text we produced (Korean in `EnToKo`).
    Target,
}

/// A translation alternative. `KoToEn` only uses `Primary`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Variant {
    Primary,
    Polite,
    Formal,
    Casual,
}

impl Variant {
    pub fn from_register(r: Register) -> Self {
        match r {
            Register::Polite => Variant::Polite,
            Register::Formal => Variant::Formal,
            Register::Casual => Variant::Casual,
        }
    }
}

/// Pipeline stages, used for timing and error attribution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Capture,
    Ocr,
    Morphology,
    Dictionary,
    Llm,
    FirstToken,
    Total,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DictSense {
    pub gloss: String,
    #[serde(default)]
    pub examples: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DictEntry {
    pub lemma: String,
    pub pos: String,
    pub senses: Vec<DictSense>,
    #[serde(default)]
    pub hanja: Option<String>,
    #[serde(default)]
    pub etymology: Option<String>,
    #[serde(default)]
    pub level: Option<String>,
}

/// One step in explaining a conjugated form.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConjugationStep {
    pub form: String,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeepSection {
    Usage,
    Examples,
    Nuance,
}

/// Everything the UI can be told about a request, in the order it happens.
///
/// Every variant carries the request `id`; the UI drops events whose id is not
/// the latest, which is how superseded requests are cancelled without races.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Started {
        id: u64,
        direction: Direction,
        capture: Capture,
        /// Served from the cache: everything arrives in one burst.
        cached: bool,
    },
    /// Instant, deterministic analysis of a Korean string.
    Breakdown {
        id: u64,
        side: Side,
        eojeols: Vec<Eojeol>,
        romanization: String,
        pronunciation: String,
    },
    /// Streaming translation text.
    Translation { id: u64, variant: Variant, delta: String },
    TranslationDone { id: u64, variant: Variant, text: String },
    /// A contextual meaning for one word, streamed as the LLM produces it.
    Gloss { id: u64, side: Side, gloss: Gloss },
    /// A short grammar/nuance remark for the whole sentence.
    Note { id: u64, text: String },

    // ── deep dive (click on a word) ──
    DeepEntries { id: u64, entries: Vec<DictEntry>, conjugation: Vec<ConjugationStep> },
    DeepDelta { id: u64, section: DeepSection, delta: String },

    // ── vocabulary ──
    /// A word's learning state changed (saved, reviewed, marked known).
    WordState { id: u64, status: crate::types::WordStatus },

    Timing { id: u64, stage: Stage, ms: u32 },
    Error { id: u64, stage: Stage, message: String },
    Done { id: u64 },
}

impl Event {
    pub fn id(&self) -> u64 {
        match self {
            Event::Started { id, .. }
            | Event::Breakdown { id, .. }
            | Event::Translation { id, .. }
            | Event::TranslationDone { id, .. }
            | Event::Gloss { id, .. }
            | Event::Note { id, .. }
            | Event::DeepEntries { id, .. }
            | Event::DeepDelta { id, .. }
            | Event::WordState { id, .. }
            | Event::Timing { id, .. }
            | Event::Error { id, .. }
            | Event::Done { id } => *id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_json_is_tagged_snake_case() {
        let e = Event::Translation { id: 7, variant: Variant::Primary, delta: "Hi".into() };
        let v: serde_json::Value = serde_json::to_value(&e).unwrap();
        assert_eq!(v["type"], "translation");
        assert_eq!(v["variant"], "primary");
        assert_eq!(e.id(), 7);
        let back: Event = serde_json::from_value(v).unwrap();
        assert_eq!(back, e);
    }
}
