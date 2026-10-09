//! Fakes: used by the engine's tests, and as the non-Windows implementation.

use std::sync::Mutex;
use std::time::Duration;

use image::{Rgb, RgbImage};
use maru_core::{AppInfo, Point, Rect};

use crate::{PlatformError, Result, ScreenGrabber, Selection, SelectionSource};

/// Does nothing; every capture reports "unsupported".
pub struct Unsupported;

impl SelectionSource for Unsupported {
    fn read_accessibility(&self) -> Result<Option<Selection>> {
        Err(PlatformError::Unsupported("selection reading"))
    }
    fn read_clipboard(&self, _: Duration) -> Result<Option<Selection>> {
        Err(PlatformError::Unsupported("clipboard capture"))
    }
    fn word_at(&self, _: Point) -> Result<Option<Selection>> {
        Err(PlatformError::Unsupported("word lookup"))
    }
}

impl ScreenGrabber for Unsupported {
    fn grab(&self, _: Rect) -> Result<RgbImage> {
        Err(PlatformError::Unsupported("screen capture"))
    }
    fn cursor(&self) -> Option<Point> {
        None
    }
    fn foreground_app(&self) -> Option<AppInfo> {
        None
    }
    fn screen_bounds(&self) -> Rect {
        Rect::new(0.0, 0.0, 1920.0, 1080.0)
    }
}

/// Scriptable selection source.
#[derive(Default)]
pub struct FakeSelection {
    pub accessibility: Mutex<Option<Selection>>,
    pub clipboard: Mutex<Option<Selection>>,
    pub word: Mutex<Option<Selection>>,
    /// Records which methods were called, in order.
    pub calls: Mutex<Vec<&'static str>>,
}

impl SelectionSource for FakeSelection {
    fn read_accessibility(&self) -> Result<Option<Selection>> {
        self.calls.lock().unwrap().push("accessibility");
        Ok(self.accessibility.lock().unwrap().clone())
    }
    fn read_clipboard(&self, _: Duration) -> Result<Option<Selection>> {
        self.calls.lock().unwrap().push("clipboard");
        Ok(self.clipboard.lock().unwrap().clone())
    }
    fn word_at(&self, _: Point) -> Result<Option<Selection>> {
        self.calls.lock().unwrap().push("word_at");
        Ok(self.word.lock().unwrap().clone())
    }
}

/// Serves a queue of images (the last one repeats) and records requested regions.
pub struct FakeScreen {
    pub frames: Mutex<Vec<RgbImage>>,
    pub requests: Mutex<Vec<Rect>>,
    pub cursor: Option<Point>,
    pub app: Option<AppInfo>,
}

impl FakeScreen {
    pub fn new(frames: Vec<RgbImage>) -> Self {
        Self { frames: Mutex::new(frames), requests: Mutex::new(Vec::new()), cursor: Some(Point { x: 500.0, y: 400.0 }), app: None }
    }

    pub fn blank(w: u32, h: u32) -> RgbImage {
        RgbImage::from_pixel(w, h, Rgb([255, 255, 255]))
    }
}

impl ScreenGrabber for FakeScreen {
    fn grab(&self, region: Rect) -> Result<RgbImage> {
        self.requests.lock().unwrap().push(region);
        let mut frames = self.frames.lock().unwrap();
        let img = if frames.len() > 1 { frames.remove(0) } else { frames.first().cloned().unwrap_or_else(|| Self::blank(8, 8)) };
        Ok(img)
    }
    fn cursor(&self) -> Option<Point> {
        self.cursor
    }
    fn foreground_app(&self) -> Option<AppInfo> {
        self.app.clone()
    }
    fn screen_bounds(&self) -> Rect {
        Rect::new(0.0, 0.0, 1920.0, 1080.0)
    }
}
