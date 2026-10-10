//! Restores spaces that OCR drops ("125열차", "천까지승차해").
//!
//! The breakdown groups morphemes into 어절 by the spaces in the text, so a missing
//! space glues two words into one block. Kiwi can re-insert them; the wrapper only
//! ever *adds* breaks (see [`maru_ocr::resplit_words`]), so recognition and boxes stay intact.

use std::sync::Arc;

use image::RgbImage;
use maru_lang::Analyzer;
use maru_ocr::{OcrEngine, OcrLine, OcrOutput, Result, resplit_words};

pub struct RespacingOcr {
    inner: Arc<dyn OcrEngine>,
    analyzer: Analyzer,
}

impl RespacingOcr {
    /// Wrap `inner`; returns it untouched when the analyser cannot respace.
    pub fn wrap(inner: Arc<dyn OcrEngine>, analyzer: &Analyzer) -> Arc<dyn OcrEngine> {
        if analyzer.respace("테스트").is_none() {
            return inner;
        }
        Arc::new(Self { inner, analyzer: analyzer.clone() })
    }

    fn fix(&self, line: &mut OcrLine) {
        if !line.text.chars().any(|c| ('가'..='힣').contains(&c)) {
            return;
        }
        let words: Vec<String> = line.words.iter().map(|w| w.text.clone()).collect();
        let Some(spaced) = self.analyzer.respace(&words.join(" ")) else { return };
        if let Some(words) = resplit_words(&line.words, &spaced) {
            line.text = words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
            line.words = words;
        }
    }
}

impl OcrEngine for RespacingOcr {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn recognize(&self, image: &RgbImage) -> Result<OcrOutput> {
        let mut out = self.inner.recognize(image)?;
        out.lines.iter_mut().for_each(|l| self.fix(l));
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use maru_core::{Rect, WordBox};
    use maru_lang::{MorphError, Morphology, RawToken};

    use super::*;

    /// Inserts a space after "까지" — enough to exercise the wrapper without Kiwi.
    struct Spacer;
    impl Morphology for Spacer {
        fn name(&self) -> &'static str {
            "spacer"
        }
        fn tokenize(&self, _: &str) -> std::result::Result<Vec<RawToken>, MorphError> {
            Ok(vec![])
        }
        fn respace(&self, text: &str) -> Option<String> {
            Some(text.replace("까지승", "까지 승"))
        }
    }

    struct Fixed(OcrOutput);
    impl OcrEngine for Fixed {
        fn name(&self) -> &'static str {
            "fixed"
        }
        fn recognize(&self, _: &RgbImage) -> Result<OcrOutput> {
            Ok(self.0.clone())
        }
    }

    fn line(text: &str) -> OcrLine {
        let words = text
            .split(' ')
            .enumerate()
            .map(|(i, t)| WordBox { text: t.into(), rect: Rect::new(i as f32 * 100.0, 0.0, 90.0, 20.0) })
            .collect();
        OcrLine { text: text.into(), rect: Rect::new(0.0, 0.0, 300.0, 20.0), words, confidence: 0.9 }
    }

    #[test]
    fn missing_spaces_are_restored_in_text_and_boxes() {
        let analyzer = Analyzer::new(Arc::new(Spacer));
        let inner: Arc<dyn OcrEngine> = Arc::new(Fixed(OcrOutput { lines: vec![line("출발 10분 천까지승차해 주시기"), line("KTX 125")] }));
        let out = RespacingOcr::wrap(inner, &analyzer).recognize(&RgbImage::new(1, 1)).unwrap();
        assert_eq!(out.lines[0].text, "출발 10분 천까지 승차해 주시기");
        assert_eq!(out.lines[0].words.len(), 5);
        assert!(out.lines[0].words[2].rect.right() <= out.lines[0].words[3].rect.x + 0.01);
        assert_eq!(out.lines[1].text, "KTX 125"); // no Hangul: untouched
    }

    #[test]
    fn an_analyser_that_cannot_respace_leaves_the_engine_alone() {
        let inner: Arc<dyn OcrEngine> = Arc::new(Fixed(OcrOutput::default()));
        let wrapped = RespacingOcr::wrap(inner.clone(), &Analyzer::heuristic());
        assert!(Arc::ptr_eq(&inner, &wrapped));
    }
}
