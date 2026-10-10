use std::sync::{Arc, Mutex, MutexGuard};

use maru_core::{Capture, Point, Rect};
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
    pub scan: Mutex<ScanState>,
}

impl AppState {
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
