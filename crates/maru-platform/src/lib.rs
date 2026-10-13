//! Operating-system integration behind small traits.
//!
//! The engine only sees [`SelectionSource`] and [`ScreenGrabber`]; Windows
//! implementations live in `windows/` and everything else (including the fakes
//! used by tests) is portable.

pub mod block;
pub mod testing;
pub mod resources;
pub mod network;
pub mod app_window;
#[cfg(windows)]
pub mod windows;

use std::sync::Arc;
use std::time::Duration;

use image::RgbImage;
use maru_core::{AppInfo, CaptureSource, Point, Rect};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PlatformError {
    #[error("not supported on this operating system: {0}")]
    Unsupported(&'static str),
    #[error("{0}")]
    Failed(String),
}

pub type Result<T, E = PlatformError> = std::result::Result<T, E>;

/// Text the user has selected (or pointed at), and where it is on screen.
#[derive(Debug, Clone, PartialEq)]
pub struct Selection {
    pub text: String,
    /// Bounding box of the text on screen, when the source knows it.
    pub rect: Option<Rect>,
    pub source: CaptureSource,
}

/// Ways of reading text without looking at pixels.
pub trait SelectionSource: Send + Sync {
    /// Selection exposed through the accessibility API (exact characters + position).
    fn read_accessibility(&self) -> Result<Option<Selection>>;
    /// Selection obtained by copying it (the clipboard is restored afterwards).
    fn read_clipboard(&self, timeout: Duration) -> Result<Option<Selection>>;
    /// The word under `point`, via accessibility.
    fn word_at(&self, point: Point) -> Result<Option<Selection>>;
}

pub trait ScreenGrabber: Send + Sync {
    /// Capture `region` (physical screen pixels, virtual-desktop coordinates).
    fn grab(&self, region: Rect) -> Result<RgbImage>;
    fn cursor(&self) -> Option<Point>;
    fn foreground_app(&self) -> Option<AppInfo>;
    /// The whole virtual desktop.
    fn screen_bounds(&self) -> Rect;
    /// Visible client area for this executable; unsupported sources return no window.
    fn app_window(&self, _exe: &str) -> Option<app_window::AppWindow> {
        None
    }
}

/// Everything the engine needs from the OS.
#[derive(Clone)]
pub struct Platform {
    pub selection: Arc<dyn SelectionSource>,
    pub screen: Arc<dyn ScreenGrabber>,
    /// OS-provided OCR, if any (Windows.Media.Ocr).
    pub os_ocr: Option<Arc<dyn maru_ocr::OcrEngine>>,
}

impl Platform {
    /// The real implementation for this OS (Windows), or inert stand-ins elsewhere.
    pub fn native() -> Self {
        #[cfg(windows)]
        {
            windows::native()
        }
        #[cfg(not(windows))]
        {
            Self { selection: Arc::new(testing::Unsupported), screen: Arc::new(testing::Unsupported), os_ocr: None }
        }
    }
}
