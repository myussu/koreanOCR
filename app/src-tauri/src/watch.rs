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
                (p.visible, p.pinned, p.hover_mode)
            };
            let panel = windows::panel_rect(&app);
            let over_panel = visible && panel.is_some_and(|r| r.contains(cursor));

            if hover_mode && !over_panel {
                if let Some(at) = dwell.update(Instant::now(), cursor) {
                    flow::hover_at(app.clone(), state.clone(), at);
                }
            } else {
                dwell.reset();
            }

            if visible && let Some(rect) = panel {
                // Hover tooltips should vanish quickly once the pointer leaves the word.
                let far = if hover_mode { 90.0 } else { 320.0 };
                if hide.update(Instant::now(), cursor, rect, pinned && !hover_mode, far, HIDE_AFTER) {
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
