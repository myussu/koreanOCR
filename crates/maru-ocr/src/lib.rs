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
/// An engine that is only built the first time it is used.
///
/// A second, differently tuned copy of PP-OCR (for whole-screen scans) costs a second or so to
/// load and about a hundred megabytes; most sessions never scan, so nothing is paid until then.
pub struct LazyOcr {
    build: Box<dyn Fn() -> std::result::Result<std::sync::Arc<dyn OcrEngine>, String> + Send + Sync>,
    engine: std::sync::OnceLock<std::result::Result<std::sync::Arc<dyn OcrEngine>, String>>,
}

impl LazyOcr {
    pub fn new(build: impl Fn() -> std::result::Result<std::sync::Arc<dyn OcrEngine>, String> + Send + Sync + 'static) -> Self {
        Self { build: Box::new(build), engine: std::sync::OnceLock::new() }
    }
}

impl OcrEngine for LazyOcr {
    fn name(&self) -> &'static str {
        "lazy"
    }

    fn recognize(&self, image: &RgbImage) -> Result<OcrOutput> {
        match self.engine.get_or_init(|| (self.build)()) {
            Ok(engine) => engine.recognize(image),
            Err(why) => Err(OcrError::Init(why.clone())),
        }
    }

    fn recognize_scaled(&self, image: &RgbImage, min_scale: f32) -> Result<OcrOutput> {
        match self.engine.get_or_init(|| (self.build)()) {
            Ok(engine) => engine.recognize_scaled(image, min_scale),
            Err(why) => Err(OcrError::Init(why.clone())),
        }
    }
}

pub trait OcrEngine: Send + Sync {
    fn name(&self) -> &'static str;
    /// Recognise text in `image`. Coordinates in the result are image pixels.
    fn recognize(&self, image: &RgbImage) -> Result<OcrOutput>;
    /// Like [`recognize`](Self::recognize), but asks a detector to enlarge the picture to at least `min_scale` times its size
    /// first (coordinates in the result are still in `image` pixels). A hint: an engine that cannot use it ignores it.
    /// Live subtitles use it, because a tight strip around a subtitle is tall enough to escape the automatic enlargement
    /// that small crops get while its lettering is still small.
    fn recognize_scaled(&self, image: &RgbImage, min_scale: f32) -> Result<OcrOutput> {
        let _ = min_scale;
        self.recognize(image)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lazy_engine_is_built_once_on_first_use() {
        struct Echo;
        impl OcrEngine for Echo {
            fn name(&self) -> &'static str {
                "echo"
            }
            fn recognize(&self, _: &RgbImage) -> Result<OcrOutput> {
                Ok(OcrOutput::default())
            }
        }
        let built = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = built.clone();
        let lazy = LazyOcr::new(move || {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(std::sync::Arc::new(Echo) as std::sync::Arc<dyn OcrEngine>)
        });
        assert_eq!(built.load(std::sync::atomic::Ordering::SeqCst), 0, "nothing is built until it is needed");
        let img = RgbImage::new(4, 4);
        assert!(lazy.recognize(&img).is_ok() && lazy.recognize(&img).is_ok());
        assert_eq!(built.load(std::sync::atomic::Ordering::SeqCst), 1);
        let broken = LazyOcr::new(|| Err("no models".into()));
        assert!(broken.recognize(&img).is_err() && broken.recognize(&img).is_err());
    }

    #[test]
    fn the_enlargement_hint_reaches_the_engine_behind_a_lazy_one_and_is_ignored_by_engines_without_it() {
        use std::sync::atomic::{AtomicU32, Ordering};
        struct Records(std::sync::Arc<AtomicU32>);
        impl OcrEngine for Records {
            fn name(&self) -> &'static str {
                "records"
            }
            fn recognize(&self, _: &RgbImage) -> Result<OcrOutput> {
                Ok(OcrOutput::default())
            }
            fn recognize_scaled(&self, _: &RgbImage, min_scale: f32) -> Result<OcrOutput> {
                self.0.store((min_scale * 10.0) as u32, Ordering::SeqCst);
                Ok(OcrOutput::default())
            }
        }
        let seen = std::sync::Arc::new(AtomicU32::new(0));
        let inner = seen.clone();
        let lazy = LazyOcr::new(move || Ok(std::sync::Arc::new(Records(inner.clone())) as std::sync::Arc<dyn OcrEngine>));
        lazy.recognize_scaled(&RgbImage::new(4, 4), 2.0).unwrap();
        assert_eq!(seen.load(Ordering::SeqCst), 20);

        struct Plain;
        impl OcrEngine for Plain {
            fn name(&self) -> &'static str {
                "plain"
            }
            fn recognize(&self, _: &RgbImage) -> Result<OcrOutput> {
                Ok(OcrOutput { lines: vec![line("안녕", 0.9, 0.0)] })
            }
        }
        assert_eq!(Plain.recognize_scaled(&RgbImage::new(4, 4), 2.0).unwrap().lines.len(), 1, "falls back to recognize");
    }

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
