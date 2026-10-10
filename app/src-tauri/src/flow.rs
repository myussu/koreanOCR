//! What happens when the user asks for something: capture → show panel → stream events into it.

use std::sync::Arc;

use maru_core::{Capture, Config, Event, Point, Rect};
use maru_engine::EventRx;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::logic;
use crate::state::AppState;
use crate::windows::{self, PANEL};

/// A message for the user that is not part of a translation (nothing selected, OCR missing…).
#[derive(Serialize, Clone)]
pub struct Notice {
    pub level: &'static str,
    pub text: String,
}

pub fn notice(app: &AppHandle, state: &AppState, anchor: Option<Point>, level: &'static str, text: impl Into<String>) {
    let anchor = anchor.map(logic::around_point).unwrap_or(state.panel().anchor);
    {
        let mut p = state.panel();
        p.anchor = anchor;
        p.size = (state.cfg().ui.max_width_px as f32, 80.0);
    }
    windows::place_and_show(app, state, state.cfg().ui.placement);
    let _ = app.emit_to(PANEL, "maru-notice", Notice { level, text: text.into() });
}

/// Forward engine events to the panel until the request ends.
pub fn pump(app: AppHandle, mut rx: EventRx) {
    tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            let done = matches!(event, Event::Done { .. });
            let _ = app.emit_to(PANEL, "maru-event", &event);
            if done {
                break;
            }
        }
    });
}

fn anchor_of(capture: &Capture, fallback: Option<Point>) -> Rect {
    capture
        .rect
        .or_else(|| capture.cursor.or(fallback).map(logic::around_point))
        .unwrap_or(Rect::new(100.0, 100.0, 200.0, 24.0))
}

pub fn show_translation(app: &AppHandle, state: &Arc<AppState>, capture: Capture, word_only: bool) {
    show_capture(app, state, capture, word_only, false);
}

/// Show `capture` in the panel. With `auto_deep` the panel also opens the deep dive for its single
/// content word as soon as the breakdown arrives (a click on a scan label that is one word).
pub fn show_capture(app: &AppHandle, state: &Arc<AppState>, capture: Capture, word_only: bool, auto_deep: bool) {
    let cursor = state.engine().platform().screen.cursor();
    {
        let mut p = state.panel();
        p.anchor = anchor_of(&capture, cursor);
        p.size = (state.cfg().ui.max_width_px as f32, 160.0);
        p.last_capture = Some(capture.clone());
    }
    windows::place_and_show(app, state, state.cfg().ui.placement);
    let (id, rx) = if word_only { state.engine().lookup_word(capture) } else { state.engine().translate(capture) };
    if auto_deep {
        let _ = app.emit_to(PANEL, "maru-auto-deep", id);
    }
    pump(app.clone(), rx);
}

/// The main hotkey: translate whatever is selected (or under the cursor).
pub fn translate_selection(app: AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        match state.engine().capture_selection().await {
            Ok(capture) => {
                tracing::info!("translate: {:?} capture, {} characters, confidence {:?}", capture.source, capture.text.chars().count(), capture.confidence);
                show_translation(&app, &state, capture, false)
            }
            Err(e) => {
                tracing::info!("translate: nothing captured: {e}");
                notice(&app, &state, state.engine().platform().screen.cursor(), "info", e.to_string())
            }
        }
    });
}

/// Read the whole screen the pointer is on and label every Korean text on it.
/// Pressing it again rescans (an older scan still in flight is cancelled).
pub fn scan_screen(app: AppHandle, state: Arc<AppState>) {
    let cursor = state.engine().platform().screen.cursor().unwrap_or_default();
    let monitor = windows::monitor_rect(&app, cursor);
    {
        let mut s = state.scan();
        s.visible = true;
        s.origin = (monitor.x, monitor.y);
        s.rects.clear();
        s.interactive = false;
    }
    windows::show_scan(&app, monitor);
    refresh_tray(&app, &state);
    let (_, mut rx) = state.engine().scan(monitor);
    tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            let done = matches!(event, Event::Done { .. });
            let _ = app.emit_to(windows::SCAN, "maru-scan", &event);
            if done {
                break;
            }
        }
    });
}

/// A scan label was clicked: explain its text in the panel. One word goes straight to its deep dive.
pub fn open_scan_item(app: AppHandle, state: Arc<AppState>, text: String, rect: Rect) {
    tracing::info!("scan label clicked: {text:?}");
    tauri::async_runtime::spawn(async move {
        let eojeols = state.engine().analyze(&text);
        let single = eojeols.iter().filter(|e| e.content).count() == 1 && eojeols.len() <= 2;
        let screen = state.engine().platform().screen.clone();
        let capture = Capture { text, source: maru_core::CaptureSource::Ocr, rect: Some(rect), words: Vec::new(), cursor: screen.cursor(), app: screen.foreground_app(), confidence: None };
        show_capture(&app, &state, capture, single, single);
    });
}

/// Hide the panel and every screen label.
pub fn dismiss_all(app: &AppHandle, state: &Arc<AppState>) {
    windows::hide_panel(app, state);
    if state.scan().visible {
        windows::hide_scan(app, state);
        refresh_tray(app, state);
    }
}

pub fn translate_region(app: AppHandle, state: Arc<AppState>, rect: Rect) {
    tauri::async_runtime::spawn(async move {
        match state.engine().capture_region(rect).await {
            Ok(capture) => {
                tracing::info!("region: {} characters, confidence {:?}", capture.text.chars().count(), capture.confidence);
                show_translation(&app, &state, capture, false)
            }
            Err(e) => {
                tracing::info!("region: nothing captured: {e}");
                notice(&app, &state, Some(Point { x: rect.x, y: rect.y + rect.h }), "info", e.to_string())
            }
        }
    });
}

pub fn hover_at(app: AppHandle, state: Arc<AppState>, at: Point) {
    tauri::async_runtime::spawn(async move {
        match state.engine().capture_hover(at).await {
            Ok(capture) => {
                tracing::debug!("hover: {:?} capture {:?}", capture.source, capture.text);
                show_translation(&app, &state, capture, true);
            }
            Err(e) => tracing::debug!("hover: {e}"),
        }
    });
}

/// What is switched on right now, e.g. `["hover mode", "live subtitles"]`.
pub fn active_features(state: &AppState) -> Vec<&'static str> {
    let mut on = Vec::new();
    if state.panel().hover_mode {
        on.push("hover mode");
    }
    if state.continuous.lock().map(|c| c.is_some()).unwrap_or(false) {
        on.push("live subtitles");
    }
    if state.scan().visible {
        on.push("screen labels");
    }
    on
}

/// The tray icon's tooltip says what is on, so a forgotten toggle is visible at a glance.
pub fn refresh_tray(app: &AppHandle, state: &AppState) {
    let on = active_features(state);
    let tip = if on.is_empty() { "Maru".to_string() } else { format!("Maru — on: {}", on.join(", ")) };
    if let Some(tray) = app.tray_by_id("maru") {
        let _ = tray.set_tooltip(Some(tip));
    }
}

pub fn toggle_hover(app: &AppHandle, state: &Arc<AppState>) {
    let on = {
        let mut p = state.panel();
        p.hover_mode = !p.hover_mode;
        p.hover_mode
    };
    let text = if on { "Hover mode on: rest the pointer on a Korean word." } else { "Hover mode off." };
    refresh_tray(app, state);
    notice(app, state, state.engine().platform().screen.cursor(), "info", text);
    if !on {
        windows::hide_panel(app, state);
    }
}

fn start_continuous(app: AppHandle, state: Arc<AppState>, region: Rect) {
    let mut handle = match state.engine().start_continuous(region) {
        Ok(h) => h,
        Err(e) => return notice(&app, &state, None, "error", e.to_string()),
    };
    let mut rx = handle.take_receiver();
    // The handle owns the stop switch: keep it in state; dropping it ends the watcher.
    if let Ok(mut slot) = state.continuous.lock() {
        *slot = Some(handle);
    }
    state.panel().pinned = true;
    refresh_tray(&app, &state);
    let keys = state.cfg().hotkeys;
    let text = format!(
        "Continuous mode on: new lines in the {}×{} area at ({}, {}) are translated automatically. {} turns it off; {} chooses a different area.",
        region.w.round(),
        region.h.round(),
        region.x.round(),
        region.y.round(),
        keys.continuous,
        keys.continuous_area
    );
    notice(&app, &state, Some(Point { x: region.x, y: region.y + region.h }), "info", text);
    tauri::async_runtime::spawn(async move {
        while let Some(capture) = rx.recv().await {
            {
                let mut p = state.panel();
                p.anchor = region;
                p.size = (state.cfg().ui.max_width_px as f32, 160.0);
            }
            windows::place_and_show(&app, &state, maru_core::config::Placement::Below);
            let (_, events) = state.engine().translate(capture);
            pump(app.clone(), events);
        }
    });
}

pub fn toggle_continuous(app: &AppHandle, state: &Arc<AppState>) {
    if state.stop_continuous() {
        state.panel().pinned = false;
        refresh_tray(app, state);
        notice(app, state, None, "info", "Continuous mode off.");
        return;
    }
    match state.live_region() {
        Some(region) => start_continuous(app.clone(), state.clone(), region),
        None => choose_continuous_area(app, state),
    }
}

/// Let the user drag a new area for continuous mode; it starts watching there as soon as it is picked.
pub fn choose_continuous_area(app: &AppHandle, state: &Arc<AppState>) {
    if state.stop_continuous() {
        state.panel().pinned = false;
        refresh_tray(app, state);
    }
    state.panel().awaiting_continuous_region = true;
    if windows::open_picker(app).is_err() {
        state.panel().awaiting_continuous_region = false;
    }
}

/// Keep the chosen live-mode area for the next launch (this session already uses it).
fn remember_region(state: &AppState, rect: Rect) {
    let paths = &state.services.paths;
    let saved = Config::load(&paths.config_file).and_then(|mut cfg| {
        cfg.continuous.region = Some(rect);
        cfg.save(&paths.config_file)
    });
    if let Err(e) = saved {
        tracing::warn!("could not save the live-mode region: {e}");
    }
}

/// The picker reported a rectangle (or was cancelled).
pub fn picked(app: AppHandle, state: Arc<AppState>, rect: Option<Rect>) {
    windows::close_picker(&app);
    let awaiting = std::mem::take(&mut state.panel().awaiting_continuous_region);
    let Some(rect) = rect else { return };
    if awaiting {
        state.set_live_region(rect);
        remember_region(&state, rect);
        start_continuous(app, state, rect);
    } else {
        translate_region(app, state, rect);
    }
}
