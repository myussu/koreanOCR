use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

use maru_core::{Capture, Config, Point, Rect};
use maru_engine::{ContinuousHandle, Engine, Services};

/// What the overlay is doing right now.
#[derive(Default)]
pub struct PanelState {
    pub visible: bool,
    /// The text the panel is attached to.
    pub anchor: Rect,
    /// Panel size in physical pixels, as last reported by the page.
    pub size: (f32, f32),
    pub pinned: bool,
    /// The panel is reading aloud: it stays up until the reading ends or it is dismissed.
    pub reading: bool,
    /// A text box in the panel has the keyboard (the deck name for Anki): the panel stays up.
    pub editing: bool,
    /// Where the user dragged the panel to (top-left, physical pixels); it stays there while pinned.
    pub custom_pos: Option<(f32, f32)>,
    /// The panel's position when the current drag began.
    pub drag_origin: Option<(f32, f32)>,
    pub hover_mode: bool,
    pub last_capture: Option<Capture>,
    pub last_cursor: Option<Point>,
    /// A region picked while "continuous" was waiting for one.
    pub awaiting_continuous_region: bool,
}

/// The screen-scan overlay: whether it is showing, where, and where its labels are.
#[derive(Default)]
pub struct ScanState {
    pub visible: bool,
    /// Physical pixel of the overlay window's top-left corner.
    pub origin: (f32, f32),
    /// Page pixels per physical pixel, as the page reports it.
    pub dpr: f32,
    /// Label areas in the page's CSS pixels.
    pub rects: Vec<Rect>,
    /// The window currently accepts the mouse (the pointer is over a label).
    pub interactive: bool,
}

pub struct AppState {
    pub services: Arc<Services>,
    pub panel: Mutex<PanelState>,
    pub continuous: Mutex<Option<ContinuousHandle>>,
    /// The area live mode watches: from the config at start-up, replaced whenever a new one is picked.
    pub live_region: Mutex<Option<Rect>>,
    /// The id of the newest live line: the band shows only that one (older lines still reach the history).
    pub live_latest: std::sync::atomic::AtomicU64,
    /// A failed live translation has already been reported this session.
    pub live_error_shown: AtomicBool,
    pub scan: Mutex<ScanState>,
    /// The settings the shell itself reads (shortcuts, panel look): replaced as soon as they are
    /// saved. The engine keeps the settings it started with until Maru restarts.
    pub live: RwLock<Config>,
    /// The file's settings as they were when Maru started, to tell what a restart would change.
    pub booted: Config,
}

impl AppState {
    /// The current settings for shortcuts and the panel.
    pub fn cfg(&self) -> Config {
        self.live.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn set_cfg(&self, cfg: Config) {
        *self.live.write().unwrap_or_else(|p| p.into_inner()) = cfg;
    }

    pub fn engine(&self) -> &Engine {
        &self.services.engine
    }

    pub fn live_region(&self) -> Option<Rect> {
        *self.live_region.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn set_live_region(&self, rect: Rect) {
        *self.live_region.lock().unwrap_or_else(|p| p.into_inner()) = Some(rect);
    }

    /// Stop the live-mode watcher; true if one was running.
    pub fn stop_continuous(&self) -> bool {
        self.continuous.lock().map(|mut c| c.take().is_some()).unwrap_or(false)
    }

    pub fn scan(&self) -> MutexGuard<'_, ScanState> {
        self.scan.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn panel(&self) -> MutexGuard<'_, PanelState> {
        self.panel.lock().unwrap_or_else(|p| p.into_inner())
    }
}
