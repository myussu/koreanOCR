use serde::{Deserialize, Serialize};

// ───────────────────────────── geometry ─────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

/// Axis-aligned rectangle in *physical screen pixels* (virtual-desktop space).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn union(&self, o: &Rect) -> Rect {
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        Rect::new(x, y, self.right().max(o.right()) - x, self.bottom().max(o.bottom()) - y)
    }
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x <= self.right() && p.y >= self.y && p.y <= self.bottom()
    }
    pub fn is_empty(&self) -> bool {
        self.w <= 0.0 || self.h <= 0.0
    }
    /// Fraction of `self` covered by `other` (0..=1).
    pub fn overlap_fraction(&self, other: &Rect) -> f32 {
        let w = (self.right().min(other.right()) - self.x.max(other.x)).max(0.0);
        let h = (self.bottom().min(other.bottom()) - self.y.max(other.y)).max(0.0);
        let area = self.w * self.h;
        if area <= 0.0 { 0.0 } else { (w * h) / area }
    }
}

// ───────────────────────────── languages ─────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    Ko,
    En,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// Korean on screen → English meaning + word breakdown.
    KoToEn,
    /// English on screen → Korean (several registers) + breakdown of the Korean.
    EnToKo,
}

impl Direction {
    pub fn source_lang(self) -> Lang {
        match self {
            Direction::KoToEn => Lang::Ko,
            Direction::EnToKo => Lang::En,
        }
    }
}

/// Politeness level for English → Korean.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Register {
    /// 해요체 — the everyday polite default.
    #[default]
    Polite,
    /// 합쇼체 — formal/deferential.
    Formal,
    /// 반말 — casual, among friends.
    Casual,
}

// ───────────────────────────── capture ─────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureSource {
    /// Exact text read through the OS accessibility API.
    Accessibility,
    /// Text obtained by copying the selection (clipboard is restored).
    Clipboard,
    /// Recognised from pixels.
    Ocr,
    /// Word under the cursor.
    Hover,
    /// Watching a fixed region (subtitles, dialogue boxes).
    Continuous,
    /// Typed or pasted into Maru itself.
    Typed,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AppInfo {
    /// Lower-cased executable file name, e.g. `chrome.exe`.
    pub exe: String,
    pub title: String,
}

/// A recognised word/segment with its position on screen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WordBox {
    pub text: String,
    pub rect: Rect,
}

/// Everything known about what the user pointed at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Capture {
    pub text: String,
    pub source: CaptureSource,
    /// Bounding box of the selection on screen (for panel placement).
    pub rect: Option<Rect>,
    /// Per-word boxes when the source can provide them.
    #[serde(default)]
    pub words: Vec<WordBox>,
    pub cursor: Option<Point>,
    pub app: Option<AppInfo>,
    /// Mean recogniser confidence for OCR captures (0..=1).
    pub confidence: Option<f32>,
}

impl Capture {
    pub fn typed(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            source: CaptureSource::Typed,
            rect: None,
            words: Vec::new(),
            cursor: None,
            app: None,
            confidence: None,
        }
    }
}

// ───────────────────────────── linguistics ─────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Lexical stem (noun, verb root, adjective root, adverb…).
    Stem,
    /// Postposition (은/는, 이/가, 을/를, 에서…).
    Particle,
    /// Verbal/adjectival ending (어요, ㄴ다, 는데…).
    Ending,
    /// Derivational affix (하다, 되다, 적, 님…).
    Affix,
    Symbol,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pos {
    Noun,
    Pronoun,
    Numeral,
    Verb,
    Adjective,
    Adverb,
    Determiner,
    Interjection,
    Particle,
    Punctuation,
    Foreign,
    Other,
}

impl Pos {
    pub fn label(self) -> &'static str {
        match self {
            Pos::Noun => "noun",
            Pos::Pronoun => "pronoun",
            Pos::Numeral => "numeral",
            Pos::Verb => "verb",
            Pos::Adjective => "adjective",
            Pos::Adverb => "adverb",
            Pos::Determiner => "determiner",
            Pos::Interjection => "interjection",
            Pos::Particle => "particle",
            Pos::Punctuation => "punctuation",
            Pos::Foreign => "foreign",
            Pos::Other => "other",
        }
    }
}

/// One morpheme as produced by the analyser.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Morpheme {
    pub form: String,
    /// Raw Sejong/Kiwi-style tag (`NNG`, `JKS`, `EF`…).
    pub tag: String,
    pub role: Role,
    /// Short English description of the tag, e.g. "subject particle".
    pub label: String,
    /// Character offsets into the analysed string.
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WordState {
    /// Never looked up.
    #[default]
    New,
    /// Saved / being reviewed.
    Learning,
    /// Stable enough that glosses can fade.
    Known,
}

/// A space-delimited Korean word (어절) with its internal structure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Eojeol {
    pub index: usize,
    pub surface: String,
    pub char_start: usize,
    pub char_end: usize,
    pub morphs: Vec<Morpheme>,
    /// Dictionary form of the main content word (공부하다, 한국어, 예쁘다).
    pub lemma: Option<String>,
    pub pos: Pos,
    /// Worth glossing (false for punctuation, bare numbers, latin text…).
    pub content: bool,
    /// "했어요 = 하다 + 았/었 + 어요"
    pub formula: String,
    pub romanization: String,
    /// The Hangul as actually pronounced (합니다 → 함니다).
    pub pronunciation: String,
    /// Short dictionary meanings, filled by the dictionary stage.
    #[serde(default)]
    pub dict_glosses: Vec<String>,
    /// Course level label, e.g. "TOPIK 1", when known.
    #[serde(default)]
    pub level: Option<String>,
    #[serde(default)]
    pub state: WordState,
    #[serde(default)]
    pub lookups: u32,
    /// On-screen box of this word when positions are known.
    #[serde(default)]
    pub rect: Option<Rect>,
}

impl Eojeol {
    /// Key used for vocabulary tracking: the lemma when we have one.
    pub fn vocab_key(&self) -> &str {
        self.lemma.as_deref().unwrap_or(&self.surface)
    }
}

/// What the vocabulary tracker knows about a word (sent to the UI after every change).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WordStatus {
    pub key: String,
    pub state: WordState,
    pub lookups: u32,
    pub saved: bool,
    #[serde(default)]
    pub meaning: Option<String>,
    #[serde(default)]
    pub level: Option<String>,
    /// Unix seconds when the next review is due.
    #[serde(default)]
    pub due: Option<i64>,
}

/// A meaning chosen *in context* (by the LLM, or the dictionary as fallback).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Gloss {
    pub index: usize,
    pub meaning: String,
    #[serde(default)]
    pub note: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_union_and_overlap() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(5.0, 5.0, 10.0, 10.0);
        assert_eq!(a.union(&b), Rect::new(0.0, 0.0, 15.0, 15.0));
        assert!((a.overlap_fraction(&b) - 0.25).abs() < 1e-6);
        assert!(a.contains(Point { x: 3.0, y: 3.0 }));
        assert!(!a.contains(Point { x: 11.0, y: 3.0 }));
    }

    #[test]
    fn capture_roundtrips_through_json() {
        let c = Capture::typed("안녕하세요");
        let json = serde_json::to_string(&c).unwrap();
        let back: Capture = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
    }
}

// ───────────────────────────── screen scan ─────────────────────────────

/// Where a scan label came from, cheapest first. The overlay shows whatever it has and
/// upgrades the label as better ones arrive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabelSource {
    /// Nothing yet (the model has not answered).
    #[default]
    Pending,
    /// The dictionary's meaning, available instantly.
    Dictionary,
    /// Seen before: remembered from an earlier scan.
    Cache,
    /// A fixed translation from the app profile's glossary.
    Glossary,
    /// Written by the language model for this scan.
    Model,
}

/// One piece of Korean text found on screen by a scan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScanItem {
    /// Stable within one scan; later `ScanLabel` events refer to it.
    pub item: u32,
    /// Where the text is, in physical screen pixels.
    pub rect: Rect,
    /// The recognised text.
    pub text: String,
    /// The label to show now (empty while pending).
    pub label: String,
    pub source: LabelSource,
    /// Every content word in it is one you already know: show it faded, or not at all.
    pub known: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join: Option<ScanJoin>,
}

/// An original label box, retained so joining can be undone without reading the screen again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScanPart {
    pub text: String,
    pub rect: Rect,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScanJoin {
    pub parts: Vec<ScanPart>,
    /// Exact saved rule keys, including nested joins; separation does not delete other rules with the same text.
    pub rules: Vec<Vec<String>>,
    pub as_word: bool,
}
