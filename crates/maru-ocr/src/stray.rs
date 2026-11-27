//! Marks of the background read as text next to a subtitle: `M`, `0`, `AA`, `FIRST`, `HHOATAEEN…`.
//!
//! On the owner's labelled game subtitles (run 4) every such mark PP-OCR read was a line of its own, with no Hangul in
//! it, while every real Latin letter or digit (`B32 프로토콜에…`, `07531 포드 랜딩 완료.`) was inside a Korean line.
//! So in a reading that has Korean, a separate line without any Hangul is dropped, unless it sits on the same row as a
//! Korean line, at its height and close to it: that is a piece of the line the detector cut off (`에이도스` + `7`).
//! Only for subtitles: a menu or a HUD has real lines of numbers and Latin on their own.

use std::sync::Arc;

use image::RgbImage;

use crate::{OcrEngine, OcrLine, OcrOutput, Result};

fn has_hangul(text: &str) -> bool {
    text.chars().any(|c| ('\u{AC00}'..='\u{D7A3}').contains(&c))
}

/// `line` continues `korean` on the same row: their heights agree, they overlap vertically, and the gap between them is
/// at most one and a half times the Korean line's height.
fn same_row(line: &OcrLine, korean: &OcrLine) -> bool {
    let (a, b) = (line.rect, korean.rect);
    if a.h <= 0.0 || b.h <= 0.0 {
        return false;
    }
    let ratio = a.h / b.h;
    let overlap = (a.y + a.h).min(b.y + b.h) - a.y.max(b.y);
    let gap = if a.x >= b.x + b.w { a.x - (b.x + b.w) } else if b.x >= a.x + a.w { b.x - (a.x + a.w) } else { 0.0 };
    (0.75..=1.33).contains(&ratio) && overlap >= 0.5 * a.h.min(b.h) && gap <= 1.5 * b.h
}

/// The reading without separate lines of background marks (see the module comment).
pub fn drop_stray_lines(mut out: OcrOutput) -> OcrOutput {
    let korean: Vec<OcrLine> = out.lines.iter().filter(|l| has_hangul(&l.text)).cloned().collect();
    if korean.is_empty() {
        return out;
    }
    out.lines.retain(|l| has_hangul(&l.text) || korean.iter().any(|k| same_row(l, k)));
    out
}

/// An engine whose readings go through [`drop_stray_lines`]: live subtitles with `[continuous] drop_stray_lines`.
pub struct StrayLinesOcr {
    inner: Arc<dyn OcrEngine>,
}

impl StrayLinesOcr {
    pub fn wrap(inner: Arc<dyn OcrEngine>) -> Arc<dyn OcrEngine> {
        Arc::new(Self { inner })
    }
}

impl OcrEngine for StrayLinesOcr {
    fn name(&self) -> &'static str {
        self.inner.name()
    }
    fn recognize(&self, image: &RgbImage) -> Result<OcrOutput> {
        self.inner.recognize(image).map(drop_stray_lines)
    }
    fn recognize_now(&self, image: &RgbImage) -> Result<OcrOutput> {
        self.inner.recognize_now(image).map(drop_stray_lines)
    }
    fn recognize_scaled(&self, image: &RgbImage, min_scale: f32) -> Result<OcrOutput> {
        self.inner.recognize_scaled(image, min_scale).map(drop_stray_lines)
    }
    fn recognize_now_scaled(&self, image: &RgbImage, min_scale: f32) -> Result<OcrOutput> {
        self.inner.recognize_now_scaled(image, min_scale).map(drop_stray_lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maru_core::Rect;

    fn line(text: &str, x: f32, y: f32, w: f32, h: f32) -> OcrLine {
        OcrLine { text: text.into(), rect: Rect::new(x, y, w, h), confidence: 0.9, words: vec![] }
    }

    fn texts(out: &OcrOutput) -> Vec<&str> {
        out.lines.iter().map(|l| l.text.as_str()).collect()
    }

    #[test]
    fn marks_read_from_the_background_as_lines_of_their_own_are_dropped() {
        // Shapes measured in run 4: a letter above the subtitle, zeros below it, a word of the scenery to one side.
        let out = OcrOutput { lines: vec![line("M", 300.0, 10.0, 20.0, 23.0), line("원숭이가 셋…", 500.0, 60.0, 260.0, 35.0), line("0", 700.0, 110.0, 12.0, 14.0), line("FIRST", 40.0, 0.0, 300.0, 106.0)] };
        assert_eq!(texts(&drop_stray_lines(out)), ["원숭이가 셋…"]);
    }

    #[test]
    fn latin_and_digits_inside_a_korean_line_stay() {
        for text in ["B32 프로토콜에 의거, 드랍 포드 전기 현재 위치에서 사출됩니다.", "07531 포드 랜딩 완료.", "에이도스 7에서 번영"] {
            let out = OcrOutput { lines: vec![line(text, 300.0, 60.0, 600.0, 35.0)] };
            assert_eq!(texts(&drop_stray_lines(out)), [text]);
        }
    }

    #[test]
    fn a_number_the_detector_cut_off_the_end_of_a_korean_line_stays() {
        let out = OcrOutput { lines: vec![line("최초의 도시 에이도스", 300.0, 60.0, 400.0, 35.0), line("7", 720.0, 61.0, 18.0, 33.0)] };
        assert_eq!(texts(&drop_stray_lines(out)), ["최초의 도시 에이도스", "7"]);
        // The same digit far away, or much smaller, is not part of that line.
        let far = OcrOutput { lines: vec![line("최초의 도시 에이도스", 300.0, 60.0, 400.0, 35.0), line("7", 900.0, 61.0, 18.0, 33.0)] };
        assert_eq!(texts(&drop_stray_lines(far)), ["최초의 도시 에이도스"]);
        let small = OcrOutput { lines: vec![line("최초의 도시 에이도스", 300.0, 60.0, 400.0, 35.0), line("0 0", 705.0, 70.0, 30.0, 14.0)] };
        assert_eq!(texts(&drop_stray_lines(small)), ["최초의 도시 에이도스"]);
    }

    #[test]
    fn a_reading_with_no_korean_is_left_as_it_is() {
        let out = OcrOutput { lines: vec![line("HP 100", 0.0, 0.0, 80.0, 20.0), line("OK", 0.0, 30.0, 30.0, 20.0)] };
        assert_eq!(texts(&drop_stray_lines(out)), ["HP 100", "OK"]);
    }
}
