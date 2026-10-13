//! Windows implementations. Compile-checked on every build for the Windows
//! target; exercised on real hardware only (they need a desktop session).

mod capture;
mod clipboard;
mod com;
mod keys;
mod ocr;
mod process;
mod uia;
pub(crate) mod resources;
pub(crate) mod network;

use std::sync::Arc;
use std::time::Duration;

use maru_core::{AppInfo, Point, Rect};

use crate::{Platform, PlatformError, Result, ScreenGrabber, Selection, SelectionSource};

pub use ocr::WindowsOcr;

pub(crate) fn fail(context: &str, e: impl std::fmt::Display) -> PlatformError {
    PlatformError::Failed(format!("{context}: {e}"))
}

/// Executables where sending Ctrl+C would interrupt a running program instead of copying.
pub const CLIPBOARD_BLOCKLIST: &[&str] = &[
    "cmd.exe",
    "powershell.exe",
    "pwsh.exe",
    "windowsterminal.exe",
    "conhost.exe",
    "wt.exe",
    "mintty.exe",
    "alacritty.exe",
    "wezterm-gui.exe",
    "kitty.exe",
    "putty.exe",
    "bash.exe",
];

pub struct WinSelection {
    uia: uia::Uia,
    clipboard_blocklist: Vec<String>,
}

impl WinSelection {
    pub fn new() -> Result<Self> {
        Ok(Self { uia: uia::Uia::start()?, clipboard_blocklist: CLIPBOARD_BLOCKLIST.iter().map(|s| s.to_string()).collect() })
    }
}

impl SelectionSource for WinSelection {
    fn read_accessibility(&self) -> Result<Option<Selection>> {
        self.uia.selection()
    }

    fn read_clipboard(&self, timeout: Duration) -> Result<Option<Selection>> {
        if let Some(app) = process::foreground_app()
            && self.clipboard_blocklist.iter().any(|b| b.eq_ignore_ascii_case(&app.exe))
        {
            tracing::debug!("not copying from {}: Ctrl+C would interrupt it", app.exe);
            return Ok(None);
        }
        clipboard::copy_selection(timeout)
    }

    fn word_at(&self, point: Point) -> Result<Option<Selection>> {
        self.uia.word_at(point)
    }
}

pub struct WinScreen;

impl ScreenGrabber for WinScreen {
    fn grab(&self, region: Rect) -> Result<image::RgbImage> {
        capture::grab(region)
    }
    fn cursor(&self) -> Option<Point> {
        capture::cursor()
    }
    fn foreground_app(&self) -> Option<AppInfo> {
        process::foreground_app()
    }
    fn screen_bounds(&self) -> Rect {
        capture::virtual_screen()
    }
}

pub fn native() -> Platform {
    let selection: Arc<dyn SelectionSource> = match WinSelection::new() {
        Ok(s) => Arc::new(s),
        Err(e) => {
            tracing::warn!("UI Automation unavailable ({e}); selection falls back to the clipboard and OCR");
            Arc::new(crate::testing::Unsupported)
        }
    };
    let os_ocr = match WindowsOcr::new("ko") {
        Ok(o) => Some(Arc::new(o) as Arc<dyn maru_ocr::OcrEngine>),
        Err(e) => {
            tracing::info!("Windows OCR for Korean is not available: {e}");
            None
        }
    };
    Platform { selection, screen: Arc::new(WinScreen), os_ocr }
}
