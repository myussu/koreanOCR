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
        if !p.pinned {
            p.custom_pos = None; // a new answer goes next to its text again, unless it was pinned in place
        }
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
    show_capture(app, state, capture, word_only, false, false);
}

/// Show `capture` in the panel. With `auto_deep` the panel also opens the deep dive for its single
/// content word as soon as the breakdown arrives (a click on a scan label that is one word). With
/// `auto_speak` the panel reads the text aloud as soon as it has it.
pub fn show_capture(app: &AppHandle, state: &Arc<AppState>, capture: Capture, word_only: bool, auto_deep: bool, auto_speak: bool) {
    let cursor = state.engine().platform().screen.cursor();
    {
        let mut p = state.panel();
        p.anchor = anchor_of(&capture, cursor);
        p.size = (state.cfg().ui.max_width_px as f32, 160.0);
        if !p.pinned {
            p.custom_pos = None;
        }
        p.last_capture = Some(capture.clone());
    }
    windows::place_and_show(app, state, state.cfg().ui.placement);
    let (id, rx) = if word_only { state.engine().lookup_word(capture) } else { state.engine().translate(capture) };
    if auto_deep {
        let _ = app.emit_to(PANEL, "maru-auto-deep", id);
    }
    if auto_speak {
        let _ = app.emit_to(PANEL, "maru-auto-speak", id);
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

/// The speak hotkey: translate the selection and read it aloud. Pressed while reading, it stops.
pub fn speak_selection(app: AppHandle, state: Arc<AppState>) {
    if state.services.tts.is_none() {
        return notice(&app, &state, state.engine().platform().screen.cursor(), "info", "Speech is off. Turn it on in Settings → Speech (or run `maru setup tts`).".to_string());
    }
    if state.panel().reading {
        let _ = app.emit_to(PANEL, "maru-tts-control", "stop");
        return;
    }
    tauri::async_runtime::spawn(async move {
        match state.engine().capture_selection().await {
            Ok(capture) => {
                tracing::info!("speak: {:?} capture, {} characters", capture.source, capture.text.chars().count());
                show_capture(&app, &state, capture, false, false, true)
            }
            Err(e) => {
                tracing::info!("speak: nothing captured: {e}");
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

/// Labels the person joined: the engine builds the one item and its label, and the events go to the same overlay as the scan's.
pub fn scan_join(app: AppHandle, state: Arc<AppState>, scan_id: u64, item: u32, pieces: Vec<maru_engine::joins::JoinPiece>) {
    tracing::info!("scan labels joined: {:?}", pieces.iter().map(|p| p.text.as_str()).collect::<Vec<_>>());
    let mut rx = state.engine().scan_join(scan_id, item, pieces);
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
        show_capture(&app, &state, capture, single, single, false);
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

/// What the live window needs to draw one translation over the original subtitle (positions in
/// physical pixels, relative to the window).
#[derive(Serialize, Clone)]
struct LiveBand {
    id: u64,
    /// The translation so far.
    text: String,
    done: bool,
    cover: Rect,
    /// Height of one line of the original text.
    line_h: f32,
    anchor_bottom: bool,
    linger_ms: u64,
}

/// Where one live line's translation goes.
#[derive(Clone, Copy)]
struct BandSpot {
    cover: Rect,
    line_h: f32,
    anchor_bottom: bool,
}

/// Height of a line of text: the middle word box, or the whole box when OCR gave no words.
fn line_height(capture: &Capture, cover: Rect) -> f32 {
    let mut heights: Vec<f32> = capture.words.iter().map(|w| w.rect.h).filter(|h| *h > 1.0).collect();
    if heights.is_empty() {
        return cover.h;
    }
    heights.sort_by(|a, b| a.total_cmp(b));
    heights[heights.len() / 2]
}

/// Forward one live line's translation to the band window while it streams. Only the newest line is
/// drawn; an older one still runs to the end so that it reaches the history.
fn pump_live(app: AppHandle, state: Arc<AppState>, id: u64, mut rx: EventRx, spot: BandSpot) {
    tauri::async_runtime::spawn(async move {
        let mut text = String::new();
        let send = |text: &str, done: bool| {
            if state.live_latest.load(std::sync::atomic::Ordering::Relaxed) != id || text.is_empty() {
                return;
            }
            windows::show_live(&app);
            let band = LiveBand { id, text: text.to_string(), done, cover: spot.cover, line_h: spot.line_h, anchor_bottom: spot.anchor_bottom, linger_ms: state.cfg().continuous.linger_ms };
            let _ = app.emit_to(windows::LIVE, "maru-live", band);
        };
        while let Some(event) = rx.recv().await {
            match event {
                Event::Translation { delta, .. } => {
                    text.push_str(&delta);
                    send(&text, false);
                }
                Event::TranslationDone { text: full, .. } => {
                    text = full;
                    send(&text, true);
                }
                Event::Error { message, .. } => {
                    tracing::warn!("live translation: {message}");
                    if !state.live_error_shown.swap(true, std::sync::atomic::Ordering::Relaxed) {
                        notice(&app, &state, None, "error", format!("Live subtitles could not be translated: {message}"));
                    }
                }
                Event::Done { .. } => break,
                _ => {}
            }
        }
    });
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
    state.live_error_shown.store(false, std::sync::atomic::Ordering::Relaxed);
    let monitor = windows::monitor_rect(&app, Point { x: region.x + region.w / 2.0, y: region.y + region.h / 2.0 });
    let (window, anchor_bottom) = logic::band_window(region, monitor);
    windows::place_live(&app, window);
    refresh_tray(&app, &state);
    let keys = state.cfg().hotkeys;
    let text = format!(
        "Live subtitles on: new lines in the {}×{} area at ({}, {}) are translated over the original. {} turns it off; {} chooses a different area. The history ({}) keeps every line.",
        region.w.round(),
        region.h.round(),
        region.x.round(),
        region.y.round(),
        keys.continuous,
        keys.continuous_area,
        keys.history
    );
    notice(&app, &state, Some(Point { x: region.x, y: region.y + region.h }), "info", text);
    tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            match event {
                maru_engine::LiveEvent::Line(line) => {
                    let cover = logic::band_cover(line.capture.rect, region, window);
                    let spot = BandSpot { cover, line_h: line_height(&line.capture, cover), anchor_bottom };
                    let (id, events) = state.engine().translate_live(*line);
                    state.live_latest.store(id, std::sync::atomic::Ordering::Relaxed);
                    pump_live(app.clone(), state.clone(), id, events, spot);
                }
                maru_engine::LiveEvent::Cleared => {
                    let _ = app.emit_to(windows::LIVE, "maru-live-clear", state.live_latest.load(std::sync::atomic::Ordering::Relaxed));
                }
            }
        }
    });
}

pub fn toggle_continuous(app: &AppHandle, state: &Arc<AppState>) {
    if state.stop_continuous() {
        windows::hide_live(app);
        refresh_tray(app, state);
        notice(app, state, None, "info", "Live subtitles off.");
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
        windows::hide_live(app);
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
