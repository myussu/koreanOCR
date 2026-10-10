//! Detector + reader: one engine finds the text lines (and their boxes), a second re-reads
//! each line from a cropped image.
//!
//! Measured on real screenshots, a vision-language reader (PaddleOCR-VL) misreads far less than
//! PP-OCR, but handed a whole busy image it skips blocks of text and gives no positions.
//! PP-OCR is the opposite: it finds every line, with boxes, but misreads stylised glyphs. Each
//! covers the other's weakness, and reading one line at a time is what the VLM was built for.

use image::imageops::{self, FilterType};
use image::RgbImage;
use maru_core::Rect;
use maru_ocr::{OcrEngine, OcrOutput, Result, words_from_chars};
use std::sync::Arc;

/// Line crops are enlarged to at least this tall: a VLM sees a few pixels of text as noise.
const MIN_CROP_HEIGHT: u32 = 56;
/// ...but never by more than this factor.
const MAX_UPSCALE: f32 = 4.0;

pub struct HybridOcr {
    detector: Arc<dyn OcrEngine>,
    reader: Arc<dyn OcrEngine>,
    /// Crops read at the same time (match the server's slot count).
    parallel: usize,
}

impl HybridOcr {
    pub fn new(detector: Arc<dyn OcrEngine>, reader: Arc<dyn OcrEngine>, parallel: usize) -> Self {
        Self { detector, reader, parallel: parallel.max(1) }
    }
}

impl OcrEngine for HybridOcr {
    fn name(&self) -> &'static str {
        "hybrid"
    }

    fn recognize(&self, image: &RgbImage) -> Result<OcrOutput> {
        let mut out = self.detector.recognize(image)?;
        let crops: Vec<RgbImage> = out.lines.iter().map(|l| crop_line(image, l.rect)).collect();
        let mut texts: Vec<Option<String>> = vec![None; crops.len()];
        for (batch, slots) in crops.chunks(self.parallel).zip(texts.chunks_mut(self.parallel)) {
            std::thread::scope(|scope| {
                for (crop, slot) in batch.iter().zip(slots.iter_mut()) {
                    let reader = &self.reader;
                    scope.spawn(move || {
                        *slot = reader.recognize(crop).ok().map(|o| o.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join(" "));
                    });
                }
            });
        }
        for (line, text) in out.lines.iter_mut().zip(texts) {
            let Some(text) = text.map(|t| t.trim().to_string()) else { continue };
            if plausible(&line.text, &text) {
                line.words = words_from_chars(&text, line.rect, &[]);
                line.text = text;
            }
        }
        Ok(out)
    }
}

/// The line with some margin (glyph tops and tails stick out of tight boxes), enlarged if tiny.
fn crop_line(image: &RgbImage, rect: Rect) -> RgbImage {
    let (pad_x, pad_y) = (rect.h * 0.4, rect.h * 0.25);
    let x0 = (rect.x - pad_x).max(0.0).floor() as u32;
    let y0 = (rect.y - pad_y).max(0.0).floor() as u32;
    let x1 = ((rect.right() + pad_x).ceil() as u32).min(image.width());
    let y1 = ((rect.bottom() + pad_y).ceil() as u32).min(image.height());
    let (w, h) = (x1.saturating_sub(x0).max(1), y1.saturating_sub(y0).max(1));
    let crop = imageops::crop_imm(image, x0.min(image.width().saturating_sub(1)), y0.min(image.height().saturating_sub(1)), w, h).to_image();
    if crop.height() >= MIN_CROP_HEIGHT {
        return crop;
    }
    let scale = (MIN_CROP_HEIGHT as f32 / crop.height() as f32).min(MAX_UPSCALE);
    imageops::resize(&crop, (crop.width() as f32 * scale).round() as u32, (crop.height() as f32 * scale).round() as u32, FilterType::CatmullRom)
}

/// Is the reader's text a believable re-reading of the detector's? A reader that returns nothing,
/// or a wall of text for a short line, has failed on this crop; keep the detector's text then.
fn plausible(old: &str, new: &str) -> bool {
    let count = |s: &str| s.chars().filter(|c| !c.is_whitespace()).count();
    let (old, new) = (count(old), count(new));
    new > 0 && new * 2 >= old && new <= old * 2 + 2
}

#[cfg(test)]
mod tests {
    use maru_core::WordBox;
    use maru_ocr::{OcrError, OcrLine};

    use super::*;

    struct Fixed(OcrOutput);
    impl OcrEngine for Fixed {
        fn name(&self) -> &'static str {
            "fixed"
        }
        fn recognize(&self, _: &RgbImage) -> Result<OcrOutput> {
            Ok(self.0.clone())
        }
    }

    /// Answers by crop width, which identifies the line even when crops are read in parallel.
    struct ByWidth(Vec<(u32, Result<&'static str, ()>)>);
    impl OcrEngine for ByWidth {
        fn name(&self) -> &'static str {
            "by-width"
        }
        fn recognize(&self, img: &RgbImage) -> Result<OcrOutput> {
            let (_, answer) = self.0.iter().find(|(w, _)| img.width() < *w).expect("a matching width");
            match answer {
                Ok(t) => Ok(OcrOutput { lines: vec![OcrLine { text: (*t).into(), rect: Rect::new(0.0, 0.0, 1.0, 1.0), words: vec![], confidence: 1.0 }] }),
                Err(()) => Err(OcrError::Run("no".into())),
            }
        }
    }

    fn line(text: &str, x: f32, y: f32, w: f32) -> OcrLine {
        let rect = Rect::new(x, y, w, 60.0);
        OcrLine { text: text.into(), rect, words: vec![WordBox { text: text.into(), rect }], confidence: 0.9 }
    }

    fn run(lines: Vec<OcrLine>, reader: ByWidth) -> Vec<OcrLine> {
        let h = HybridOcr::new(Arc::new(Fixed(OcrOutput { lines })), Arc::new(reader), 2);
        h.recognize(&RgbImage::new(1000, 400)).unwrap().lines
    }

    #[test]
    fn each_line_is_re_read_from_its_own_crop_and_keeps_its_box() {
        // crop widths: 100 + 48 = 148 and 300 + 48 = 348
        let lines = run(
            vec![line("전금도", 20.0, 20.0, 100.0), line("충분히 잘하고 있어요", 20.0, 150.0, 300.0)],
            ByWidth(vec![(200, Ok("지금도")), (400, Ok("충분히 잘하고 있어요"))]),
        );
        assert_eq!(lines[0].text, "지금도");
        assert_eq!(lines[0].rect, Rect::new(20.0, 20.0, 100.0, 60.0), "geometry comes from the detector");
        assert_eq!(lines[0].words.len(), 1);
        assert_eq!(lines[1].words.len(), 3, "word boxes follow the reader's spacing");
    }

    #[test]
    fn a_failed_or_implausible_reading_keeps_the_detectors_text() {
        let lines = run(
            vec![line("승차권", 20.0, 20.0, 100.0), line("서울", 20.0, 150.0, 300.0)],
            ByWidth(vec![(200, Err(())), (400, Ok("이것은 서울과 전혀 관계가 없는 아주 긴 환각 문장입니다"))]),
        );
        assert_eq!(lines[0].text, "승차권");
        assert_eq!(lines[1].text, "서울");
    }

    #[test]
    fn plausibility_bounds() {
        assert!(plausible("전금도", "지금도"));
        assert!(plausible("호감 집", "호감집"));
        assert!(!plausible("승차권", ""));
        assert!(!plausible("서울", "아주 긴 환각 문장입니다"));
        assert!(!plausible("", ""));
    }

    #[test]
    fn small_crops_are_enlarged_but_never_beyond_the_image_or_the_cap() {
        let img = RgbImage::new(500, 300);
        let tiny = crop_line(&img, Rect::new(10.0, 10.0, 100.0, 12.0)); // padded height 18
        assert!(tiny.height() >= MIN_CROP_HEIGHT - 1 && tiny.height() <= 18 * 4 + 1);
        let tall = crop_line(&img, Rect::new(10.0, 10.0, 100.0, 80.0));
        assert!(tall.height() >= MIN_CROP_HEIGHT);
        let edge = crop_line(&img, Rect::new(480.0, 280.0, 60.0, 30.0)); // box hangs off the image
        assert!(edge.width() >= 1 && edge.height() >= 1);
    }
}
