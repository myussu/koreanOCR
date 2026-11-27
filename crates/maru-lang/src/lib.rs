//! Korean language analysis for Maru.
//!
//! * [`hangul`] — syllable maths, script and direction detection
//! * [`pronounce`] — pronunciation rules and Revised Romanization
//! * [`tags`] — learner-friendly labels for Kiwi's tag set
//! * [`morph`] — the analyser trait, with Kiwi and a rule-based fallback
//! * [`eojeol`] — grouping morphemes into words with lemmas and formulas

pub mod eojeol;
pub mod hangul;
pub mod heuristic;
#[cfg(feature = "kiwi")]
pub mod kiwi;
pub mod morph;
pub mod pronounce;
pub mod tags;

use std::sync::Arc;

use maru_core::Eojeol;

pub use eojeol::{conjugation_steps, display_form};
pub use heuristic::HeuristicAnalyzer;
pub use morph::{AnalysisOptions, MorphError, Morphology, RawToken, dialect_mask};

/// Result of analysing a Korean string.
#[derive(Debug, Clone)]
pub struct Analysis {
    pub text: String,
    pub eojeols: Vec<Eojeol>,
    pub romanization: String,
    pub pronunciation: String,
    /// Which analyser produced it (`kiwi` or `heuristic`).
    pub analyzer: &'static str,
}

/// Cheap to clone; share one per process.
#[derive(Clone)]
pub struct Analyzer {
    primary: Arc<dyn Morphology>,
}

impl Analyzer {
    pub fn new(primary: Arc<dyn Morphology>) -> Self {
        Self { primary }
    }

    pub fn heuristic() -> Self {
        Self::new(Arc::new(HeuristicAnalyzer::new()))
    }

    /// Kiwi when its library can be loaded, otherwise the heuristic analyser.
    #[cfg(feature = "kiwi")]
    pub fn kiwi_or_fallback() -> Self {
        match kiwi::KiwiAnalyzer::spawn() {
            Ok(k) => Self::new(Arc::new(k)),
            Err(e) => {
                tracing::warn!("Kiwi unavailable ({e}); using the rule-based analyser — run `maru setup` to install Kiwi");
                Self::heuristic()
            }
        }
    }

    pub fn name(&self) -> &'static str {
        self.primary.name()
    }

    /// `text` with missing spaces inserted, when the analyser can do that (Kiwi can).
    pub fn respace(&self, text: &str) -> Option<String> {
        self.primary.respace(text)
    }

    /// Could `text` stand as a finished word? True when its last morpheme is a particle, an
    /// ending, a number, a foreign word, a symbol or a free-standing adverb. False for a noun
    /// compound's first half, and when the analyser cannot tell.
    pub fn ends_a_word(&self, text: &str) -> bool {
        self.primary.tokenize(text).ok().and_then(|t| t.last().map(|t| tags::can_end_word(&t.tag))).unwrap_or(false)
    }

    /// Never fails: if the primary analyser errors, the heuristic one answers.
    pub fn analyze(&self, text: &str) -> Analysis {
        self.analyze_with(text, AnalysisOptions::default())
    }

    /// [`Self::analyze`] with dialects or typo correction allowed (a program's profile asks for them).
    pub fn analyze_with(&self, text: &str, options: AnalysisOptions) -> Analysis {
        let tokenized = if options == AnalysisOptions::default() { self.primary.tokenize(text) } else { self.primary.tokenize_with(text, options) };
        let (tokens, analyzer) = match tokenized {
            Ok(t) => (t, self.primary.name()),
            Err(e) => {
                tracing::warn!("{} failed ({e}); falling back to heuristic analysis", self.primary.name());
                (HeuristicAnalyzer.tokenize(text).unwrap_or_default(), "heuristic")
            }
        };
        Analysis {
            text: text.to_owned(),
            eojeols: eojeol::group(text, &tokens),
            romanization: pronounce::romanize(text),
            pronunciation: pronounce::pronounce(text),
            analyzer,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Broken;
    impl Morphology for Broken {
        fn name(&self) -> &'static str {
            "broken"
        }
        fn tokenize(&self, _: &str) -> Result<Vec<RawToken>, MorphError> {
            Err(MorphError::Failed("boom".into()))
        }
    }

    /// Records the options it was asked to analyse with.
    struct Recording(std::sync::Mutex<Vec<AnalysisOptions>>);
    impl Morphology for Recording {
        fn name(&self) -> &'static str {
            "recording"
        }
        fn tokenize(&self, text: &str) -> Result<Vec<RawToken>, MorphError> {
            self.tokenize_with(text, AnalysisOptions::default())
        }
        fn tokenize_with(&self, text: &str, options: AnalysisOptions) -> Result<Vec<RawToken>, MorphError> {
            self.0.lock().unwrap().push(options);
            HeuristicAnalyzer.tokenize(text)
        }
    }

    #[test]
    fn a_programs_dialects_and_typo_correction_reach_the_analyser_and_the_default_is_standard_korean() {
        let rec = Arc::new(Recording(Default::default()));
        let a = Analyzer::new(rec.clone());
        a.analyze("왔나? 기다릿데이");
        let gyeongsang = AnalysisOptions { dialects: dialect_mask(&["Gyeongsang"]).0, typo: true };
        a.analyze_with("왔나? 기다릿데이", gyeongsang);
        assert_eq!(*rec.0.lock().unwrap(), vec![AnalysisOptions::default(), gyeongsang]);
    }

    #[test]
    fn dialect_names_become_kiwis_bits_and_unknown_names_are_named() {
        assert_eq!(dialect_mask(&["gyeongsang"]), (1 << 3, vec![]));
        assert_eq!(dialect_mask(&["jeolla", "JEJU"]).0, (1 << 4) | (1 << 5));
        assert_eq!(dialect_mask(&["satoori"]), (0, vec!["satoori".to_string()]));
        assert_eq!(dialect_mask::<&str>(&[]).0, 0, "none: standard Korean");
    }

    #[test]
    fn falls_back_when_primary_fails() {
        let a = Analyzer::new(Arc::new(Broken)).analyze("학교에서 공부했어요.");
        assert_eq!(a.analyzer, "heuristic");
        assert_eq!(a.eojeols.len(), 2);
        assert_eq!(a.eojeols[1].lemma.as_deref(), Some("공부하다"));
        assert_eq!(a.romanization, "hakgyoeseo gongbuhaesseoyo.");
    }

    #[test]
    fn empty_text_has_no_words() {
        assert!(Analyzer::heuristic().analyze("   ").eojeols.is_empty());
    }
}
