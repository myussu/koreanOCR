//! Text recognition for pixels.
//!
//! The engine is behind [`OcrEngine`] so the rest of Maru does not care whether
//! the text came from PP-OCR (this crate) or Windows' built-in OCR (maru-platform).

pub mod align;
mod geometry;
#[cfg(feature = "paddle")]
pub mod paddle;
mod prep;

use image::RgbImage;
use maru_core::{Point, Rect, WordBox};
use thiserror::Error;

pub use geometry::{reading_order, regroup_words, resplit_words, words_from_chars};
pub use prep::{Prepared, prepare};

#[derive(Debug, Error)]
pub enum OcrError {
    #[error("OCR model files missing: {0}")]
    MissingModels(String),
    #[error("OCR engine failed to start: {0}")]
    Init(String),
    #[error("OCR failed: {0}")]
    Run(String),
}

pub type Result<T, E = OcrError> = std::result::Result<T, E>;

/// One recognised line of text.
#[derive(Debug, Clone, PartialEq)]
pub struct OcrLine {
    pub text: String,
    pub rect: Rect,
    /// 0..=1
    pub confidence: f32,
    /// Space-delimited words with positions, in reading order.
    pub words: Vec<WordBox>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct OcrOutput {
    pub lines: Vec<OcrLine>,
}

impl OcrOutput {
    pub fn is_empty(&self) -> bool {
        self.lines.iter().all(|l| l.text.trim().is_empty())
    }

    /// All lines joined with newlines.
    pub fn text(&self) -> String {
        self.lines.iter().map(|l| l.text.trim()).filter(|t| !t.is_empty()).collect::<Vec<_>>().join("\n")
    }

    /// Mean confidence weighted by line length.
    pub fn confidence(&self) -> f32 {
        let (sum, n) = self.lines.iter().fold((0.0, 0usize), |(s, n), l| {
            let len = l.text.chars().count();
            (s + l.confidence * len as f32, n + len)
        });
        if n == 0 { 0.0 } else { sum / n as f32 }
    }

    pub fn words(&self) -> Vec<WordBox> {
        self.lines.iter().flat_map(|l| l.words.iter().cloned()).collect()
    }

    /// Union of all line boxes.
    pub fn bounds(&self) -> Option<Rect> {
        self.lines.iter().map(|l| l.rect).reduce(|a, b| a.union(&b))
    }

    /// Move image coordinates into screen coordinates: `screen = origin + img / scale`.
    pub fn to_screen(mut self, origin: Point, scale: f32) -> Self {
        let map = |r: Rect| Rect::new(origin.x + r.x / scale, origin.y + r.y / scale, r.w / scale, r.h / scale);
        for line in &mut self.lines {
            line.rect = map(line.rect);
            for w in &mut line.words {
                w.rect = map(w.rect);
            }
        }
        self
    }
}

/// Anything that can read text from an image.
pub trait OcrEngine: Send + Sync {
    fn name(&self) -> &'static str;
    /// Recognise text in `image`. Coordinates in the result are image pixels.
    fn recognize(&self, image: &RgbImage) -> Result<OcrOutput>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str, conf: f32, x: f32) -> OcrLine {
        OcrLine {
            text: text.into(),
            rect: Rect::new(x, 10.0, 100.0, 20.0),
            confidence: conf,
            words: vec![WordBox { text: text.into(), rect: Rect::new(x, 10.0, 100.0, 20.0) }],
        }
    }

    #[test]
    fn confidence_is_weighted_by_length() {
        let out = OcrOutput { lines: vec![line("aaaa", 1.0, 0.0), line("b", 0.0, 0.0)] };
        assert!((out.confidence() - 0.8).abs() < 1e-6);
        assert_eq!(OcrOutput::default().confidence(), 0.0);
    }

    #[test]
    fn to_screen_applies_origin_and_scale() {
        let out = OcrOutput { lines: vec![line("hi", 1.0, 20.0)] }.to_screen(Point { x: 1000.0, y: 500.0 }, 2.0);
        assert_eq!(out.lines[0].rect, Rect::new(1010.0, 505.0, 50.0, 10.0));
        assert_eq!(out.words()[0].rect, Rect::new(1010.0, 505.0, 50.0, 10.0));
    }

    #[test]
    fn text_skips_blank_lines() {
        let out = OcrOutput { lines: vec![line("a", 1.0, 0.0), line("  ", 1.0, 0.0), line("b", 1.0, 0.0)] };
        assert_eq!(out.text(), "a\nb");
        assert!(!out.is_empty());
    }
}
