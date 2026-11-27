//! The seam between "something that splits Korean into morphemes" and the rest
//! of Maru. Kiwi is the real implementation; the heuristic one keeps the app
//! usable (and tests deterministic) when Kiwi's native library is missing.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum MorphError {
    #[error("morphological analyser unavailable: {0}")]
    Unavailable(String),
    #[error("analysis failed: {0}")]
    Failed(String),
}

/// A morpheme as reported by an analyser, before any interpretation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawToken {
    pub form: String,
    /// Sejong-style tag, possibly with Kiwi's `-I`/`-R` suffix.
    pub tag: String,
    /// Character (not byte) offsets into the analysed text.
    pub start: usize,
    pub end: usize,
}

impl RawToken {
    pub fn new(form: impl Into<String>, tag: impl Into<String>, start: usize, end: usize) -> Self {
        Self { form: form.into(), tag: tag.into(), start, end }
    }
}

/// How one analysis may read a line beyond standard Korean (per program: `[[profile]] dialects`, `typo_correction`).
/// The default is standard Korean only, as before.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AnalysisOptions {
    /// Regional dialects Kiwi may use, as its bit mask (see [`dialect_mask`]); 0 = standard Korean only.
    pub dialects: i32,
    /// Let Kiwi correct typos (and misread letters) while it analyses.
    pub typo: bool,
}

/// Kiwi's dialect names and bits (`KIWI_DIALECT_*`, Kiwi 0.22 and later).
pub const DIALECTS: [(&str, i32); 8] = [
    ("gyeonggi", 1 << 0),
    ("chungcheong", 1 << 1),
    ("gangwon", 1 << 2),
    ("gyeongsang", 1 << 3),
    ("jeolla", 1 << 4),
    ("jeju", 1 << 5),
    ("hwanghae", 1 << 6),
    ("hamgyeong", 1 << 7),
];

/// The mask for dialect names; unknown names are ignored (and named in the second value).
pub fn dialect_mask<S: AsRef<str>>(names: &[S]) -> (i32, Vec<String>) {
    let mut mask = 0;
    let mut unknown = Vec::new();
    for name in names {
        match DIALECTS.iter().find(|(n, _)| n.eq_ignore_ascii_case(name.as_ref().trim())) {
            Some((_, bit)) => mask |= bit,
            None => unknown.push(name.as_ref().to_string()),
        }
    }
    (mask, unknown)
}

pub trait Morphology: Send + Sync {
    fn name(&self) -> &'static str;
    fn tokenize(&self, text: &str) -> Result<Vec<RawToken>, MorphError>;

    /// [`Self::tokenize`] allowing dialects or typo correction; analysers that cannot do either analyse as usual.
    fn tokenize_with(&self, text: &str, _options: AnalysisOptions) -> Result<Vec<RawToken>, MorphError> {
        self.tokenize(text)
    }

    /// The same text with missing spaces inserted (OCR drops them), if this analyser can.
    fn respace(&self, _text: &str) -> Option<String> {
        None
    }
}
