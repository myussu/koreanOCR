//! Turning "the user pointed at something" into a [`Capture`].
//!
//! Order of attempts for the hotkey, cheapest and most exact first:
//! 1. accessibility selection (exact characters, no OCR error, ~ms);
//! 2. copy the selection (works in anything with Ctrl+C);
//! 3. OCR the block of text under the cursor.

use std::sync::Arc;
use std::time::Duration;

use maru_core::config::CaptureConfig;
use maru_core::{AppInfo, Capture, CaptureSource, Point, Rect, WordBox};
use maru_ocr::{OcrEngine, OcrOutput};
use maru_platform::{Platform, Selection, block};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CaptureError {
    #[error("nothing to read: {0}")]
    NothingFound(String),
    #[error("no OCR engine is available (install the OCR models with `maru setup ocr`, or add the Korean OCR language pack in Windows)")]
    NoOcr,
    #[error("{0}")]
    Failed(String),
}

#[derive(Clone)]
pub struct Capturer {
    platform: Platform,
    ocr: Option<Arc<dyn OcrEngine>>,
    cfg: CaptureConfig,
    min_confidence: f32,
}

fn selection_to_capture(sel: Selection, cursor: Option<Point>, app: Option<AppInfo>) -> Capture {
    Capture { text: sel.text, source: sel.source, rect: sel.rect, words: Vec::new(), cursor, app, confidence: None }
}

impl Capturer {
    pub fn new(platform: Platform, ocr: Option<Arc<dyn OcrEngine>>, cfg: CaptureConfig, min_confidence: f32) -> Self {
        Self { platform, ocr, cfg, min_confidence }
    }

    pub fn has_ocr(&self) -> bool {
        self.ocr.is_some()
    }

    /// OCR a screen rectangle; coordinates in the result are screen pixels.
    pub async fn ocr_rect(&self, rect: Rect) -> Result<OcrOutput, CaptureError> {
        let ocr = self.ocr.clone().ok_or(CaptureError::NoOcr)?;
        let screen = self.platform.screen.clone();
        let bounds = screen.screen_bounds();
        let clipped = clip(rect, bounds);
        if clipped.is_empty() {
            return Err(CaptureError::Failed(format!("region {rect:?} is outside the screen")));
        }
        tokio::task::spawn_blocking(move || {
            let image = screen.grab(clipped).map_err(|e| CaptureError::Failed(e.to_string()))?;
            let out = ocr.recognize(&image).map_err(|e| CaptureError::Failed(e.to_string()))?;
            Ok(out.to_screen(Point { x: clipped.x, y: clipped.y }, 1.0))
        })
        .await
        .map_err(|e| CaptureError::Failed(e.to_string()))?
    }

    /// The selection if there is one, otherwise the text block under the cursor.
    pub async fn selection(&self) -> Result<Capture, CaptureError> {
        let cursor = self.platform.screen.cursor();
        let app = self.platform.screen.foreground_app();
        let mut tried: Vec<&str> = Vec::new();

        if self.cfg.use_accessibility {
            tried.push("accessibility");
            let source = self.platform.selection.clone();
            match tokio::task::spawn_blocking(move || source.read_accessibility()).await {
                Ok(Ok(Some(sel))) if !sel.text.trim().is_empty() => return Ok(selection_to_capture(sel, cursor, app)),
                Ok(Err(e)) => tracing::debug!("accessibility selection failed: {e}"),
                _ => {}
            }
        }
        if self.cfg.use_clipboard_fallback {
            tried.push("clipboard");
            let source = self.platform.selection.clone();
            let timeout = Duration::from_millis(self.cfg.clipboard_timeout_ms);
            match tokio::task::spawn_blocking(move || source.read_clipboard(timeout)).await {
                Ok(Ok(Some(sel))) if !sel.text.trim().is_empty() => return Ok(selection_to_capture(sel, cursor, app)),
                Ok(Err(e)) => tracing::debug!("clipboard selection failed: {e}"),
                _ => {}
            }
        }
        if self.cfg.use_ocr_fallback {
            if self.ocr.is_none() {
                tried.push("ocr (unavailable: run `maru setup ocr` or add the Windows Korean OCR pack)");
            } else if let Some(cursor) = cursor {
                tracing::info!("no selection via {}; reading the text block near ({:.0}, {:.0})", tried.join(" and "), cursor.x, cursor.y);
                return self.block_under(cursor, app).await;
            } else {
                tried.push("ocr (cursor position unknown)");
            }
        }
        Err(CaptureError::NothingFound(format!("tried {}", tried.join(", "))))
    }

    async fn block_under(&self, cursor: Point, app: Option<AppInfo>) -> Result<Capture, CaptureError> {
        let r = self.cfg.ocr_radius_px as f32;
        let out = self.ocr_rect(Rect::new(cursor.x - r, cursor.y - r / 2.0, 2.0 * r, r)).await?;
        let lines = block::block_at(&out.lines, cursor, r / 2.0);
        if lines.is_empty() {
            return Err(CaptureError::NothingFound("no text near the cursor".into()));
        }
        let text = lines.iter().map(|l| l.text.trim()).collect::<Vec<_>>().join(" ");
        let rect = lines.iter().map(|l| l.rect).reduce(|a, b| a.union(&b));
        let words: Vec<WordBox> = lines.iter().flat_map(|l| l.words.iter().cloned()).collect();
        let conf = lines.iter().map(|l| l.confidence).sum::<f32>() / lines.len() as f32;
        self.warn_if_unsure(conf);
        Ok(Capture { text, source: CaptureSource::Ocr, rect, words, cursor: Some(cursor), app, confidence: Some(conf) })
    }

    /// OCR everything inside a user-drawn rectangle.
    pub async fn region(&self, rect: Rect) -> Result<Capture, CaptureError> {
        let out = self.ocr_rect(rect).await?;
        if out.is_empty() {
            return Err(CaptureError::NothingFound("no text recognised in that area".into()));
        }
        let conf = out.confidence();
        self.warn_if_unsure(conf);
        Ok(Capture {
            text: out.lines.iter().map(|l| l.text.trim()).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" "),
            source: CaptureSource::Ocr,
            rect: out.bounds(),
            words: out.words(),
            cursor: self.platform.screen.cursor(),
            app: self.platform.screen.foreground_app(),
            confidence: Some(conf),
        })
    }

    /// The word under `point` (accessibility first, then a small OCR crop).
    pub async fn hover(&self, point: Point) -> Result<Capture, CaptureError> {
        let app = self.platform.screen.foreground_app();
        if self.cfg.use_accessibility {
            let source = self.platform.selection.clone();
            if let Ok(Ok(Some(sel))) = tokio::task::spawn_blocking(move || source.word_at(point)).await
                && !sel.text.trim().is_empty()
            {
                return Ok(selection_to_capture(sel, Some(point), app));
            }
        }
        let out = self.ocr_rect(Rect::new(point.x - 180.0, point.y - 30.0, 360.0, 60.0)).await?;
        let word = out
            .words()
            .into_iter()
            .map(|w| {
                let d = if w.rect.contains(point) { 0.0 } else { distance(&w.rect, point) };
                (d, w)
            })
            .filter(|(d, _)| *d <= 24.0)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, w)| w);
        let Some(word) = word else { return Err(CaptureError::NothingFound("no word under the cursor".into())) };
        let text = word.text.trim_matches(|c: char| !c.is_alphanumeric() && !maru_lang::hangul::is_hangul(c)).to_string();
        if text.is_empty() {
            return Err(CaptureError::NothingFound("no word under the cursor".into()));
        }
        Ok(Capture {
            text,
            source: CaptureSource::Hover,
            rect: Some(word.rect),
            words: vec![word],
            cursor: Some(point),
            app,
            confidence: Some(out.confidence()),
        })
    }

    fn warn_if_unsure(&self, confidence: f32) {
        if confidence < self.min_confidence {
            tracing::info!("OCR confidence {confidence:.2} is below {:.2}; the translation will be told the text may contain errors", self.min_confidence);
        }
    }
}

fn distance(r: &Rect, p: Point) -> f32 {
    let dx = (r.x - p.x).max(0.0).max(p.x - r.right());
    let dy = (r.y - p.y).max(0.0).max(p.y - r.bottom());
    (dx * dx + dy * dy).sqrt()
}

fn clip(r: Rect, bounds: Rect) -> Rect {
    let x = r.x.max(bounds.x);
    let y = r.y.max(bounds.y);
    Rect::new(x, y, (r.right().min(bounds.right()) - x).max(0.0), (r.bottom().min(bounds.bottom()) - y).max(0.0))
}

#[cfg(test)]
pub(crate) mod testing {
    //! A scripted OCR engine shared by this crate's tests.
    use std::sync::Mutex;

    use image::RgbImage;
    use maru_core::{Rect, WordBox};
    use maru_ocr::{OcrEngine, OcrLine, OcrOutput, Result};

    /// Reads the red channel of the top-left pixel as the "page number" and returns
    /// the matching scripted output, so a test controls OCR through the frames it feeds.
    pub struct FakeOcr {
        pub pages: Vec<(u8, OcrOutput)>,
        pub calls: Mutex<u32>,
    }

    impl FakeOcr {
        pub fn new(pages: Vec<(u8, OcrOutput)>) -> Self {
            Self { pages, calls: Mutex::new(0) }
        }
    }

    impl OcrEngine for FakeOcr {
        fn name(&self) -> &'static str {
            "fake"
        }
        fn recognize(&self, image: &RgbImage) -> Result<OcrOutput> {
            *self.calls.lock().unwrap() += 1;
            let key = image.get_pixel(0, 0)[0];
            Ok(self.pages.iter().find(|(k, _)| *k == key).map(|(_, o)| o.clone()).unwrap_or_default())
        }
    }

    pub fn line(text: &str, x: f32, y: f32, w: f32) -> OcrLine {
        let rect = Rect::new(x, y, w, 20.0);
        let words = text
            .split_whitespace()
            .enumerate()
            .map(|(i, t)| WordBox { text: t.into(), rect: Rect::new(x + i as f32 * 60.0, y, 55.0, 20.0) })
            .collect();
        OcrLine { text: text.into(), rect, confidence: 0.95, words }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use image::{Rgb, RgbImage};
    use maru_platform::testing::{FakeScreen, FakeSelection};

    use super::testing::*;
    use super::*;

    fn selection(text: &str, source: CaptureSource) -> Selection {
        Selection { text: text.into(), rect: Some(Rect::new(100.0, 100.0, 200.0, 24.0)), source }
    }

    fn capturer(sel: Arc<FakeSelection>, screen: Arc<FakeScreen>, ocr: Option<Arc<dyn OcrEngine>>) -> Capturer {
        let platform = Platform { selection: sel, screen, os_ocr: None };
        Capturer::new(platform, ocr, CaptureConfig::default(), 0.8)
    }

    fn page(r: u8) -> RgbImage {
        RgbImage::from_pixel(640, 320, Rgb([r, 255, 255]))
    }

    #[tokio::test]
    async fn accessibility_wins_and_nothing_else_is_tried() {
        let sel = Arc::new(FakeSelection::default());
        *sel.accessibility.lock().unwrap() = Some(selection("저는 학생입니다", CaptureSource::Accessibility));
        *sel.clipboard.lock().unwrap() = Some(selection("WRONG", CaptureSource::Clipboard));
        let c = capturer(sel.clone(), Arc::new(FakeScreen::new(vec![])), None);
        let cap = c.selection().await.unwrap();
        assert_eq!((cap.text.as_str(), cap.source), ("저는 학생입니다", CaptureSource::Accessibility));
        assert_eq!(cap.rect, Some(Rect::new(100.0, 100.0, 200.0, 24.0)));
        assert_eq!(*sel.calls.lock().unwrap(), ["accessibility"]);
    }

    #[tokio::test]
    async fn falls_back_to_the_clipboard() {
        let sel = Arc::new(FakeSelection::default());
        *sel.clipboard.lock().unwrap() = Some(selection("복사된 글", CaptureSource::Clipboard));
        let c = capturer(sel.clone(), Arc::new(FakeScreen::new(vec![])), None);
        let cap = c.selection().await.unwrap();
        assert_eq!(cap.source, CaptureSource::Clipboard);
        assert_eq!(*sel.calls.lock().unwrap(), ["accessibility", "clipboard"]);
    }

    #[tokio::test]
    async fn falls_back_to_ocr_of_the_block_under_the_cursor() {
        // cursor is at (500, 400); the screen crop starts at (340, 350)
        let out = OcrOutput {
            lines: vec![line("오늘은 날씨가 좋아요", 20.0, 40.0, 300.0), line("산책하러 갈까요", 20.0, 64.0, 250.0), line("멀리 있는 글", 20.0, 280.0, 200.0)],
        };
        let ocr: Arc<dyn OcrEngine> = Arc::new(FakeOcr::new(vec![(7, out)]));
        let screen = Arc::new(FakeScreen::new(vec![page(7)]));
        let c = capturer(Arc::new(FakeSelection::default()), screen.clone(), Some(ocr));
        let cap = c.selection().await.unwrap();
        assert_eq!(cap.text, "오늘은 날씨가 좋아요 산책하러 갈까요");
        assert_eq!(cap.source, CaptureSource::Ocr);
        assert!(cap.confidence.unwrap() > 0.9);
        // boxes were moved into screen coordinates: crop origin = (500 - 320, 400 - 160) = (180, 240)
        let r = cap.rect.unwrap();
        assert_eq!((r.x, r.y), (200.0, 280.0), "{r:?}");
        let requested = screen.requests.lock().unwrap()[0];
        assert_eq!((requested.w, requested.h), (640.0, 320.0));
        assert_eq!(cap.words.len(), 5);
    }

    #[tokio::test]
    async fn nothing_anywhere_explains_what_was_tried() {
        let c = capturer(Arc::new(FakeSelection::default()), Arc::new(FakeScreen::new(vec![])), None);
        let err = c.selection().await.unwrap_err().to_string();
        assert!(err.contains("accessibility") && err.contains("clipboard") && err.contains("ocr"), "{err}");
    }

    #[tokio::test]
    async fn drawn_region_returns_all_text_with_word_boxes() {
        let out = OcrOutput { lines: vec![line("저는 학생입니다", 10.0, 10.0, 200.0), line("만나서 반갑습니다", 10.0, 40.0, 220.0)] };
        let ocr: Arc<dyn OcrEngine> = Arc::new(FakeOcr::new(vec![(3, out)]));
        let c = capturer(Arc::new(FakeSelection::default()), Arc::new(FakeScreen::new(vec![page(3)])), Some(ocr));
        let cap = c.region(Rect::new(300.0, 200.0, 400.0, 100.0)).await.unwrap();
        assert_eq!(cap.text, "저는 학생입니다 만나서 반갑습니다");
        assert_eq!(cap.words.len(), 4);
        assert!(cap.words[0].rect.x >= 310.0 - 0.1, "word boxes are in screen coordinates");
    }

    #[tokio::test]
    async fn empty_region_is_reported() {
        let ocr: Arc<dyn OcrEngine> = Arc::new(FakeOcr::new(vec![]));
        let c = capturer(Arc::new(FakeSelection::default()), Arc::new(FakeScreen::new(vec![page(0)])), Some(ocr));
        assert!(matches!(c.region(Rect::new(0.0, 0.0, 100.0, 50.0)).await, Err(CaptureError::NothingFound(_))));
        assert!(matches!(c.region(Rect::new(5000.0, 5000.0, 10.0, 10.0)).await, Err(CaptureError::Failed(_))));
    }

    #[tokio::test]
    async fn missing_ocr_engine_gives_actionable_error() {
        let c = capturer(Arc::new(FakeSelection::default()), Arc::new(FakeScreen::new(vec![])), None);
        assert!(matches!(c.region(Rect::new(0.0, 0.0, 10.0, 10.0)).await, Err(CaptureError::NoOcr)));
    }

    #[tokio::test]
    async fn hover_prefers_accessibility_then_picks_the_word_under_the_pointer() {
        let sel = Arc::new(FakeSelection::default());
        *sel.word.lock().unwrap() = Some(selection("학교", CaptureSource::Hover));
        let c = capturer(sel, Arc::new(FakeScreen::new(vec![])), None);
        assert_eq!(c.hover(Point { x: 10.0, y: 10.0 }).await.unwrap().text, "학교");

        // OCR path: the crop is centred on the pointer, so pointer (420, 400) → crop origin (240, 370).
        // Line at crop x=100: word 1 at 100..155, word 2 ("학생입니다.") at 160..215 → screen 400..455, y 390..410.
        let near = OcrOutput { lines: vec![line("저는 학생입니다.", 100.0, 20.0, 200.0)] };
        // On the other page every word is at the far left edge of the crop: 130px from the pointer.
        let far = OcrOutput { lines: vec![line("멀어요", 0.0, 20.0, 50.0)] };
        let ocr: Arc<dyn OcrEngine> = Arc::new(FakeOcr::new(vec![(9, near), (8, far)]));
        let screen = Arc::new(FakeScreen::new(vec![page(9), page(8)]));
        let c = capturer(Arc::new(FakeSelection::default()), screen, Some(ocr));

        let cap = c.hover(Point { x: 420.0, y: 400.0 }).await.unwrap();
        assert_eq!((cap.text.as_str(), cap.source), ("학생입니다", CaptureSource::Hover));
        assert_eq!(cap.rect, Some(Rect::new(400.0, 390.0, 55.0, 20.0)));

        let err = c.hover(Point { x: 420.0, y: 400.0 }).await.unwrap_err();
        assert!(matches!(err, CaptureError::NothingFound(_)), "{err}");
    }
}
