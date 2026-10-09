//! What happens when the user asks for something: capture → show panel → stream events into it.

use std::sync::Arc;

use maru_core::{Capture, Event, Point, Rect};
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
        p.size = (state.engine().config().ui.max_width_px as f32, 80.0);
    }
    windows::place_and_show(app, state, state.engine().config().ui.placement);
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
    let cursor = state.engine().platform().screen.cursor();
    {
        let mut p = state.panel();
        p.anchor = anchor_of(&capture, cursor);
        p.size = (state.engine().config().ui.max_width_px as f32, 160.0);
        p.last_capture = Some(capture.clone());
    }
    windows::place_and_show(app, state, state.engine().config().ui.placement);
    let (_, rx) = if word_only { state.engine().lookup_word(capture) } else { state.engine().translate(capture) };
    pump(app.clone(), rx);
}

/// The main hotkey: translate whatever is selected (or under the cursor).
pub fn translate_selection(app: AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        match state.engine().capture_selection().await {
            Ok(capture) => show_translation(&app, &state, capture, false),
            Err(e) => notice(&app, &state, state.engine().platform().screen.cursor(), "info", e.to_string()),
        }
    });
}

pub fn translate_region(app: AppHandle, state: Arc<AppState>, rect: Rect) {
    tauri::async_runtime::spawn(async move {
        match state.engine().capture_region(rect).await {
            Ok(capture) => show_translation(&app, &state, capture, false),
            Err(e) => notice(&app, &state, Some(Point { x: rect.x, y: rect.y + rect.h }), "info", e.to_string()),
        }
    });
}

pub fn hover_at(app: AppHandle, state: Arc<AppState>, at: Point) {
    tauri::async_runtime::spawn(async move {
        if let Ok(capture) = state.engine().capture_hover(at).await {
            show_translation(&app, &state, capture, true);
        }
    });
}

pub fn toggle_hover(app: &AppHandle, state: &Arc<AppState>) {
    let on = {
        let mut p = state.panel();
        p.hover_mode = !p.hover_mode;
        p.hover_mode
    };
    let text = if on { "Hover mode on: rest the pointer on a Korean word." } else { "Hover mode off." };
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
    notice(&app, &state, Some(Point { x: region.x, y: region.y + region.h }), "info", "Continuous mode on: new lines in that area are translated automatically.");
    tauri::async_runtime::spawn(async move {
        while let Some(capture) = rx.recv().await {
            {
                let mut p = state.panel();
                p.anchor = region;
                p.size = (state.engine().config().ui.max_width_px as f32, 160.0);
            }
            windows::place_and_show(&app, &state, maru_core::config::Placement::Below);
            let (_, events) = state.engine().translate(capture);
            pump(app.clone(), events);
        }
    });
}

pub fn toggle_continuous(app: &AppHandle, state: &Arc<AppState>) {
    if state.continuous.lock().map(|mut c| c.take().is_some()).unwrap_or(false) {
        state.panel().pinned = false;
        notice(app, state, None, "info", "Continuous mode off.");
        return;
    }
    match state.engine().config().continuous.region {
        Some(region) => start_continuous(app.clone(), state.clone(), region),
        None => {
            state.panel().awaiting_continuous_region = true;
            let _ = windows::open_picker(app);
        }
    }
}

/// The picker reported a rectangle (or was cancelled).
pub fn picked(app: AppHandle, state: Arc<AppState>, rect: Option<Rect>) {
    windows::close_picker(&app);
    let awaiting = std::mem::take(&mut state.panel().awaiting_continuous_region);
    let Some(rect) = rect else { return };
    if awaiting {
        start_continuous(app, state, rect);
    } else {
        translate_region(app, state, rect);
    }
}
