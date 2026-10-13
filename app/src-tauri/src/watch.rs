//! Background loop: hover dwell detection and hiding an ignored panel.

use std::sync::Arc;
use std::time::{Duration, Instant};

use maru_core::Point;
use tauri::AppHandle;

use crate::logic::{AutoHide, Dwell};
use crate::state::AppState;
use crate::{flow, windows};

const POLL: Duration = Duration::from_millis(60);
const HOVER_DWELL: Duration = Duration::from_millis(380);
const HOVER_RADIUS: f32 = 5.0;
const HIDE_AFTER: Duration = Duration::from_millis(1800);

pub fn spawn(app: AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        let screen = state.engine().platform().screen.clone();
        let mut dwell = Dwell::new(HOVER_RADIUS, HOVER_DWELL);
        let mut hide = AutoHide::default();
        let mut ticker = tokio::time::interval(POLL);
        loop {
            ticker.tick().await;
            let Some(cursor) = screen.cursor() else { continue };
            let (visible, pinned, hover_mode) = {
                let p = state.panel();
                // reading aloud keeps the panel up like a pin does, even in hover mode
                (p.visible, p.pinned || p.reading || p.editing, p.hover_mode)
            };
            let panel = windows::panel_rect(&app);
            let over_panel = visible && panel.is_some_and(|r| r.contains(cursor));

            if hover_mode && !over_panel && !crate::launcher::over_controls(&app, cursor) {
                if let Some(at) = dwell.update(Instant::now(), cursor) {
                    flow::hover_at(app.clone(), state.clone(), at);
                }
            } else {
                dwell.reset();
            }

            if visible && let Some(rect) = panel {
                // Hover tooltips should vanish quickly once the pointer leaves the word.
                let far = if hover_mode { 90.0 } else { 320.0 };
                if hide.update(Instant::now(), cursor, rect, pinned && (!hover_mode || state.panel().reading || state.panel().editing), far, HIDE_AFTER) {
                    windows::hide_panel(&app, &state);
                    hide.reset();
                }
            } else {
                hide.reset();
            }
            state.panel().last_cursor = Some(Point { x: cursor.x, y: cursor.y });
        }
    });
}

/// How often the scan overlay checks whether the pointer is over one of its labels. Quick enough
/// that a click made right after moving onto a label is not lost to the game underneath.
const SCAN_POLL: Duration = Duration::from_millis(12);

/// The scan overlay is click-through, so the game stays playable under it; this switches it to
/// accept the mouse only while the pointer is over a label, its action menu or the join chip.
pub fn spawn_scan_hits(app: AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        let screen = state.engine().platform().screen.clone();
        let mut ticker = tokio::time::interval(SCAN_POLL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            let (visible, origin, dpr, interactive) = {
                let s = state.scan();
                (s.visible, s.origin, s.dpr, s.interactive)
            };
            if !visible {
                continue;
            }
            let Some(cursor) = screen.cursor() else { continue };
            let at = Point { x: cursor.x, y: cursor.y };
            // The panel opens over the label that was clicked; where the two overlap the panel wins,
            // so its words and buttons stay clickable.
            let on_panel = state.panel().visible && windows::panel_rect(&app).is_some_and(|r| r.contains(at));
            let over = !on_panel && crate::logic::over_label(at, origin, dpr, &state.scan().rects);
            if over != interactive {
                tracing::debug!("scan overlay {} the pointer", if over { "takes" } else { "releases" });
                if let Some(win) = tauri::Manager::get_webview_window(&app, windows::SCAN) {
                    let _ = win.set_ignore_cursor_events(!over);
                }
                state.scan().interactive = over;
            }
        }
    });
}

/// Read how your cards stand in Anki: once at start-up (waiting for Anki to be open) and then every
/// `refresh_minutes`. Words you hold for weeks become known, so their glosses fade.
pub fn spawn_anki_refresh(state: Arc<AppState>) {
    let cfg = state.engine().config().learning.anki.clone();
    if !cfg.enabled {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let mut waited = 0u32;
        loop {
            match state.engine().anki_refresh().await {
                Ok(n) => {
                    tracing::info!("Anki: {n} of your words are there");
                    waited = 0;
                }
                // Anki is usually closed at start-up: try again soon, then settle into the normal rhythm.
                Err(e) => {
                    tracing::debug!("Anki refresh: {e}");
                    if cfg.refresh_minutes == 0 {
                        return;
                    }
                    waited += 1;
                }
            }
            if cfg.refresh_minutes == 0 {
                return;
            }
            let wait = if waited > 0 && waited < 6 { Duration::from_secs(60) } else { Duration::from_secs(u64::from(cfg.refresh_minutes) * 60) };
            tokio::time::sleep(wait).await;
        }
    });
}
