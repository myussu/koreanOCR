use crate::{flow, state::AppState, windows};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

fn pump(app: AppHandle, state: Arc<AppState>, id: u64, mut rx: maru_engine::EventRx) {
    tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            if !state.scan().accepts_events(id) {
                break;
            }
            let done = matches!(event, maru_core::Event::Done { .. });
            let _ = app.emit_to(windows::SCAN, "maru-scan", event);
            if done {
                break;
            }
        }
    });
}

#[tauri::command]
pub async fn scan_join_items(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    scan_id: u64,
    item: u32,
    items: Vec<u32>,
    as_word: bool,
) -> Result<(), String> {
    if !state.scan().accepts_events(scan_id) {
        return Err("Use the current screen labels to join words.".into());
    }
    let rx = state
        .engine()
        .join_scan_items(scan_id, item, items, as_word)
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!(scan_id, item, as_word, "scan labels joined");
    pump(app, state.inner().clone(), scan_id, rx);
    Ok(())
}

#[tauri::command]
pub async fn scan_separate(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    scan_id: u64,
    item: u32,
) -> Result<(), String> {
    if !state.scan().accepts_events(scan_id) {
        return Err("Use the current screen labels to separate a sentence.".into());
    }
    let rx = state
        .engine()
        .separate_scan_item(scan_id, item)
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!(scan_id, item, "scan joined label separated");
    pump(app, state.inner().clone(), scan_id, rx);
    Ok(())
}

#[tauri::command]
pub fn scan_open_item(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    scan_id: u64,
    item: u32,
) -> Result<(), String> {
    if !state.scan().accepts_events(scan_id) {
        return Err("Click a label from the current screen scan.".into());
    }
    let (capture, single) = state
        .engine()
        .capture_scan_item(scan_id, item)
        .map_err(|e| e.to_string())?;
    tracing::info!(scan_id, item, "scan label clicked: {:?}", capture.text);
    flow::show_capture(&app, state.inner(), capture, single, single, false);
    Ok(())
}
