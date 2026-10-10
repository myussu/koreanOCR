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

pub trait Morphology: Send + Sync {
    fn name(&self) -> &'static str;
    fn tokenize(&self, text: &str) -> Result<Vec<RawToken>, MorphError>;

    /// The same text with missing spaces inserted (OCR drops them), if this analyser can.
    fn respace(&self, _text: &str) -> Option<String> {
        None
    }
}
