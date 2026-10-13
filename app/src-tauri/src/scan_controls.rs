//! Transient screen-label controls shared by the floating menu and scan page.
use crate::{flow, state::AppState, windows};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

pub fn toggle(app: AppHandle, state: Arc<AppState>) {
    if state.scan().visible {
        windows::hide_scan(&app, &state);
        flow::refresh_tray(&app, &state);
    } else {
        flow::scan_screen(app, state);
    }
}

pub fn set_join_mode(
    app: &AppHandle,
    state: &AppState,
    scan_id: u64,
    enabled: bool,
) -> Result<bool, String> {
    let previous = {
        let mut scan = state.scan();
        let previous = scan.join_mode;
        if !scan.set_join_mode(scan_id, enabled) {
            return Err("The screen labels changed. Turn Label screen on and try again.".into());
        }
        previous
    };
    if let Err(error) = app.emit_to(
        windows::SCAN,
        "maru-scan-join-mode",
        serde_json::json!({"scanId":scan_id,"enabled":enabled}),
    ) {
        state.scan().set_join_mode(scan_id, previous);
        return Err(error.to_string());
    }
    Ok(enabled)
}

#[tauri::command]
pub fn scan_join_mode(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    scan_id: u64,
    enabled: bool,
) -> Result<bool, String> {
    set_join_mode(&app, &state, scan_id, enabled)
}
