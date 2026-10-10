//! Detector + reader: PP-OCR knows where every line of text is, a vision model reads the whole
//! image and gets the letters right more often. Run both at once, then line them up by text.
//!
//! Measured on real Korean screenshots:
//!
//! * PaddleOCR-VL on the whole image: 3.5% character error, but it skips some blocks of text and
//!   returns no positions.
//! * PP-OCR: 15% (stylised handwriting is its weak spot), but it finds every line, with boxes.
//! * Feeding the vision model one *crop* per line was far worse (31%): a crop of busy or slanted
//!   text contains pieces of the neighbouring lines, and a vision model reads whatever it sees.
//!   Whole-image reading lets it decide for itself what is text. So no crops here.
//!
//! The merge keeps PP-OCR's lines and boxes, and swaps in the reader's wording wherever the
//! reader's text matches a line (see [`maru_ocr::align::better_readings`]). Lines the reader
//! never produced keep PP-OCR's own text, so nothing it found is lost.

use std::sync::Arc;

use image::RgbImage;
use maru_ocr::align::better_readings;
use maru_ocr::{OcrEngine, OcrError, OcrOutput, Result, regroup_words, words_from_chars};

pub struct HybridOcr {
    detector: Arc<dyn OcrEngine>,
    reader: Arc<dyn OcrEngine>,
}

impl HybridOcr {
    pub fn new(detector: Arc<dyn OcrEngine>, reader: Arc<dyn OcrEngine>) -> Self {
        Self { detector, reader }
    }
}

impl OcrEngine for HybridOcr {
    fn name(&self) -> &'static str {
        "hybrid"
    }

    fn recognize(&self, image: &RgbImage) -> Result<OcrOutput> {
        // Different hardware (CPU and GPU), so running them together costs the slower one only.
        let (detected, read) = std::thread::scope(|scope| {
            let detector = scope.spawn(|| self.detector.recognize(image));
            let read = self.reader.recognize(image);
            let detected = detector.join().unwrap_or_else(|_| Err(OcrError::Run("the detector thread panicked".into())));
            (detected, read)
        });
        match (detected, read) {
            (Ok(mut out), Ok(read)) => {
                merge(&mut out, &read);
                Ok(out)
            }
            (Ok(out), Err(e)) => {
                tracing::warn!("the reading model failed ({e}); using the detector's text");
                Ok(out)
            }
            // No geometry, but the text is the better half.
            (Err(e), Ok(read)) => {
                tracing::warn!("the detector failed ({e}); using the reading model's text without positions");
                Ok(read)
            }
            (Err(e), Err(_)) => Err(e),
        }
    }
}

fn merge(out: &mut OcrOutput, read: &OcrOutput) {
    let reference = read.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n");
    if reference.trim().is_empty() {
        return;
    }
    let texts: Vec<&str> = out.lines.iter().map(|l| l.text.as_str()).collect();
    let better = better_readings(&texts, &reference);
    let mut kept = Vec::with_capacity(out.lines.len());
    for (mut line, better) in std::mem::take(&mut out.lines).into_iter().zip(better) {
        match better {
            Some(text) => {
                // Same number of letters: the old per-character positions still apply.
                line.words = regroup_words(&line.words, &text).unwrap_or_else(|| words_from_chars(&text, line.rect, &[]));
                line.text = text;
                kept.push(line);
            }
            // Unmatched single marks next to a successful reading are noise (a stray `'`, `|`).
            None if line.text.chars().filter(|c| !c.is_whitespace()).count() <= 1 => {}
            None => kept.push(line),
        }
    }
    out.lines = kept;
}

#[cfg(test)]
mod tests {
    use maru_core::{Rect, WordBox};
    use maru_ocr::OcrLine;

    use super::*;

    struct Fixed(Result<OcrOutput>);
    impl OcrEngine for Fixed {
        fn name(&self) -> &'static str {
            "fixed"
        }
        fn recognize(&self, _: &RgbImage) -> Result<OcrOutput> {
            match &self.0 {
                Ok(o) => Ok(o.clone()),
                Err(e) => Err(OcrError::Run(e.to_string())),
            }
        }
    }

    fn line(text: &str, y: f32) -> OcrLine {
        let rect = Rect::new(10.0, y, 300.0, 20.0);
        let words = text
            .split_whitespace()
            .enumerate()
            .map(|(i, t)| WordBox { text: t.into(), rect: Rect::new(10.0 + i as f32 * 100.0, y, 90.0, 20.0) })
            .collect();
        OcrLine { text: text.into(), rect, words, confidence: 0.9 }
    }

    fn run(detector: Result<OcrOutput>, reader: Result<OcrOutput>) -> Result<OcrOutput> {
        HybridOcr::new(Arc::new(Fixed(detector)), Arc::new(Fixed(reader))).recognize(&RgbImage::new(1, 1))
    }

    fn texts(o: &OcrOutput) -> Vec<&str> {
        o.lines.iter().map(|l| l.text.as_str()).collect()
    }

    #[test]
    fn the_readers_wording_replaces_the_detectors_and_the_boxes_stay() {
        let detector = OcrOutput { lines: vec![line("전금도", 10.0), line("충분히 '잘하고 있어요", 40.0), line("'", 70.0), line("승차권", 100.0)] };
        let reader = OcrOutput { lines: vec![line("지금도", 0.0), line("충분히 잘하고 있어요.", 0.0)] };
        let out = run(Ok(detector.clone()), Ok(reader)).unwrap();
        // 승차권 was never read by the model: it stays. The lone `'` is dropped.
        assert_eq!(texts(&out), ["지금도", "충분히 잘하고 있어요.", "승차권"]);
        assert_eq!(out.lines[0].rect, detector.lines[0].rect, "positions always come from the detector");
        assert_eq!(out.lines[0].words.len(), 1);
        assert_eq!(out.lines[1].words.len(), 3, "word boxes follow the reader's spacing");
    }

    #[test]
    fn letters_fixed_in_place_keep_the_detectors_word_boxes() {
        let detector = OcrOutput { lines: vec![line("전금도 충분히", 10.0)] };
        let reader = OcrOutput { lines: vec![line("지금도 충분히", 0.0)] };
        let out = run(Ok(detector.clone()), Ok(reader)).unwrap();
        assert_eq!(out.lines[0].words.iter().map(|w| w.rect).collect::<Vec<_>>(), detector.lines[0].words.iter().map(|w| w.rect).collect::<Vec<_>>());
    }

    #[test]
    fn if_one_engine_fails_the_other_still_answers() {
        let detector = OcrOutput { lines: vec![line("승차권", 10.0)] };
        let reader = OcrOutput { lines: vec![line("지금도", 0.0)] };
        let only_detector = run(Ok(detector.clone()), Err(OcrError::Run("server down".into()))).unwrap();
        assert_eq!(texts(&only_detector), ["승차권"]);
        let only_reader = run(Err(OcrError::Run("model missing".into())), Ok(reader)).unwrap();
        assert_eq!(texts(&only_reader), ["지금도"]);
        assert!(run(Err(OcrError::Run("a".into())), Err(OcrError::Run("b".into()))).is_err());
    }

    #[test]
    fn an_empty_reading_changes_nothing() {
        let detector = OcrOutput { lines: vec![line("승차권", 10.0), line("'", 40.0)] };
        let out = run(Ok(detector), Ok(OcrOutput::default())).unwrap();
        assert_eq!(texts(&out), ["승차권", "'"], "no reading to judge noise against");
    }
}
