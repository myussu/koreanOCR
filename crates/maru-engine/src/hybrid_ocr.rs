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
    /// A line PP-OCR read with at least this confidence keeps its text when the reader's differs by one syllable.
    keep_sure: Option<f32>,
}

impl HybridOcr {
    pub fn new(detector: Arc<dyn OcrEngine>, reader: Arc<dyn OcrEngine>) -> Self {
        Self { detector, reader, keep_sure: None }
    }

    /// See `[ocr] keep_sure_lines`. Run 3 found the reader's typical mistake on clean subtitle lettering to be one
    /// changed syllable in a line PP-OCR had right (`왜 울지?` → `왜 을지?`, `좋군그래` → `종군그래`).
    pub fn keeping_sure_lines(mut self, confidence: Option<f32>) -> Self {
        self.keep_sure = confidence;
        self
    }
}

/// The letters and digits of `a` and `b` are the same but for one Hangul syllable.
pub(crate) fn one_syllable_apart(a: &str, b: &str) -> bool {
    let letters = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).collect::<Vec<char>>();
    let (a, b) = (letters(a), letters(b));
    let syllable = |c: char| ('\u{AC00}'..='\u{D7A3}').contains(&c);
    if a.len() != b.len() {
        return false;
    }
    let differ: Vec<(char, char)> = a.iter().zip(&b).filter(|(x, y)| x != y).map(|(x, y)| (*x, *y)).collect();
    matches!(differ.as_slice(), [(x, y)] if syllable(*x) && syllable(*y))
}

impl OcrEngine for HybridOcr {
    fn name(&self) -> &'static str {
        "hybrid"
    }

    fn recognize(&self, image: &RgbImage) -> Result<OcrOutput> {
        self.read(image, false, 1.0)
    }

    /// While the reader loads, the detector's own reading answers (live subtitles must not stall for seconds).
    fn recognize_now(&self, image: &RgbImage) -> Result<OcrOutput> {
        self.read(image, true, 1.0)
    }

    /// Only the detector enlarges the picture: the reading model gets the image as it is (a larger one costs it more time).
    fn recognize_scaled(&self, image: &RgbImage, min_scale: f32) -> Result<OcrOutput> {
        self.read(image, false, min_scale)
    }

    fn recognize_now_scaled(&self, image: &RgbImage, min_scale: f32) -> Result<OcrOutput> {
        self.read(image, true, min_scale)
    }
}

impl HybridOcr {
    fn read(&self, image: &RgbImage, now: bool, min_scale: f32) -> Result<OcrOutput> {
        // Different hardware (CPU and GPU), so running them together costs the slower one only.
        let (detected, read) = std::thread::scope(|scope| {
            let detector = scope.spawn(|| if now { self.detector.recognize_now_scaled(image, min_scale) } else { self.detector.recognize_scaled(image, min_scale) });
            let read = if now { self.reader.recognize_now(image) } else { self.reader.recognize(image) };
            let detected = detector.join().unwrap_or_else(|_| Err(OcrError::Run("the detector thread panicked".into())));
            (detected, read)
        });
        match (detected, read) {
            (Ok(mut out), Ok(read)) => {
                merge(&mut out, &read, self.keep_sure);
                Ok(out)
            }
            (Ok(out), Err(OcrError::NotReady(why))) => {
                tracing::debug!("{why}; using the detector's text");
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

fn merge(out: &mut OcrOutput, read: &OcrOutput, keep_sure: Option<f32>) {
    let reference = read.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n");
    if reference.trim().is_empty() {
        return;
    }
    let texts: Vec<&str> = out.lines.iter().map(|l| l.text.as_str()).collect();
    let better = better_readings(&texts, &reference);
    let mut kept = Vec::with_capacity(out.lines.len());
    for (mut line, better) in std::mem::take(&mut out.lines).into_iter().zip(better) {
        match better {
            // PP-OCR was sure of this line, and the reader changed a single syllable of it: PP-OCR's stays.
            Some(text) if keep_sure.is_some_and(|t| line.confidence >= t) && one_syllable_apart(&line.text, &text) => kept.push(line),
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

    /// Remembers the enlargement it was asked for (in tenths), and what size of image it was given.
    struct Notes(std::sync::Arc<std::sync::atomic::AtomicU32>);
    impl OcrEngine for Notes {
        fn name(&self) -> &'static str {
            "notes"
        }
        fn recognize(&self, _: &RgbImage) -> Result<OcrOutput> {
            self.0.store(10, std::sync::atomic::Ordering::SeqCst);
            Ok(OcrOutput::default())
        }
        fn recognize_scaled(&self, _: &RgbImage, min_scale: f32) -> Result<OcrOutput> {
            self.0.store((min_scale * 10.0) as u32, std::sync::atomic::Ordering::SeqCst);
            Ok(OcrOutput::default())
        }
    }

    fn sure(text: &str, confidence: f32) -> OcrLine {
        OcrLine { confidence, ..line(text, 0.0) }
    }

    fn run_keeping(detector: OcrLine, reader: &str, keep: Option<f32>) -> String {
        let hybrid = HybridOcr::new(Arc::new(Fixed(Ok(OcrOutput { lines: vec![detector] }))), Arc::new(Fixed(Ok(OcrOutput { lines: vec![line(reader, 0.0)] })))).keeping_sure_lines(keep);
        hybrid.recognize(&RgbImage::new(1, 1)).unwrap().text()
    }

    #[test]
    fn a_line_pp_ocr_was_sure_of_keeps_its_text_when_the_reader_changes_one_syllable() {
        assert_eq!(run_keeping(sure("왜 울지?", 0.99), "왜 을지?", Some(0.95)), "왜 울지?");
        assert_eq!(run_keeping(sure("왜 울지?", 0.99), "왜 을지?", None), "왜 을지?", "off: the reader's text, as before");
        assert_eq!(run_keeping(sure("왜 울지?", 0.90), "왜 을지?", Some(0.95)), "왜 을지?", "PP-OCR was not sure enough");
        assert_eq!(run_keeping(sure("늑대여, 잘 찾아주었다", 0.99), "느낌에, 잘 찾아주었다", Some(0.95)), "느낌에, 잘 찾아주었다", "two syllables apart: the reader decides");
        assert_eq!(run_keeping(sure("아아 얘야", 0.99), "아아, 얘야…", Some(0.95)), "아아, 얘야…", "punctuation and spacing are the reader's");
    }

    #[test]
    fn one_syllable_apart_counts_letters_only() {
        assert!(one_syllable_apart("좋군그래", "종군그래"));
        assert!(one_syllable_apart("하오나…", "하오니"));
        assert!(!one_syllable_apart("좋군그래", "좋군그래"), "the same text");
        assert!(!one_syllable_apart("늑대여", "느낌에"));
        assert!(!one_syllable_apart("B32 가", "B33 가"), "a digit is not a syllable");
        assert!(!one_syllable_apart("가나", "가나다"), "a missing syllable is not a changed one");
    }

    #[test]
    fn only_the_detector_is_asked_to_enlarge_the_picture() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let (detector, reader) = (std::sync::Arc::new(AtomicU32::new(0)), std::sync::Arc::new(AtomicU32::new(0)));
        let hybrid = HybridOcr::new(Arc::new(Notes(detector.clone())), Arc::new(Notes(reader.clone())));
        hybrid.recognize_scaled(&RgbImage::new(1, 1), 2.0).unwrap();
        assert_eq!(detector.load(Ordering::SeqCst), 20, "the detector enlarges");
        assert_eq!(reader.load(Ordering::SeqCst), 10, "the reading model is not asked to");
        hybrid.recognize(&RgbImage::new(1, 1)).unwrap();
        assert_eq!(detector.load(Ordering::SeqCst), 10, "a plain read asks for no enlargement");
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
    fn a_reader_still_loading_leaves_the_detectors_reading() {
        let detector = Arc::new(Fixed(Ok(OcrOutput { lines: vec![line("안녕하세요 여러분", 0.0)] })));
        struct Loading;
        impl OcrEngine for Loading {
            fn name(&self) -> &'static str {
                "loading"
            }
            fn recognize(&self, _: &RgbImage) -> Result<OcrOutput> {
                Err(OcrError::NotReady("the reader is loading".into()))
            }
        }
        let reader = Arc::new(Loading);
        let out = HybridOcr::new(detector, reader).recognize_now(&RgbImage::new(8, 8)).unwrap();
        assert_eq!(texts(&out), ["안녕하세요 여러분"]);
    }

    #[test]
    fn a_live_read_enlarges_for_the_detector_and_still_does_not_wait_for_the_reader() {
        use std::sync::atomic::{AtomicU32, Ordering};
        struct Loading;
        impl OcrEngine for Loading {
            fn name(&self) -> &'static str {
                "loading"
            }
            fn recognize(&self, _: &RgbImage) -> Result<OcrOutput> {
                panic!("a live read must not wait for the reader");
            }
            fn recognize_now(&self, _: &RgbImage) -> Result<OcrOutput> {
                Err(OcrError::NotReady("the reader is loading".into()))
            }
        }
        let asked = std::sync::Arc::new(AtomicU32::new(0));
        let hybrid = HybridOcr::new(Arc::new(Notes(asked.clone())), Arc::new(Loading));
        hybrid.recognize_now_scaled(&RgbImage::new(1, 1), 2.0).unwrap();
        assert_eq!(asked.load(Ordering::SeqCst), 20, "the detector got the enlargement");
        hybrid.recognize_now(&RgbImage::new(1, 1)).unwrap();
        assert_eq!(asked.load(Ordering::SeqCst), 10, "and none when none is asked for");
    }

    #[test]
    fn an_empty_reading_changes_nothing() {
        let detector = OcrOutput { lines: vec![line("승차권", 10.0), line("'", 40.0)] };
        let out = run(Ok(detector), Ok(OcrOutput::default())).unwrap();
        assert_eq!(texts(&out), ["승차권", "'"], "no reading to judge noise against");
    }
}
