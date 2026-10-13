//! The OCR editor calls engine correction methods and forwards their existing event streams.
use crate::{flow, state::AppState, windows};
use maru_engine::ocr_corrections::{CorrectionView, Target};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};

const LABEL: &str = "correction";
#[derive(Default)]
pub struct CorrectionState {
    target: Mutex<Option<Target>>,
}
type S<'a> = State<'a, Arc<AppState>>;

fn open(app: &AppHandle, state: &AppState, target: Target) -> Result<(), String> {
    let view = state
        .engine()
        .ocr_correction_view(target)
        .map_err(|e| e.to_string())?;
    *app.state::<CorrectionState>()
        .target
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = Some(target);
    let window = match app.get_webview_window(LABEL) {
        Some(window) => window,
        None => WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("correction.html".into()))
            .title("Maru — correct OCR text")
            .inner_size(640.0, 600.0)
            .min_inner_size(420.0, 360.0)
            .always_on_top(true)
            .content_protected(true)
            .visible(false)
            .build()
            .map_err(|e| e.to_string())?,
    };
    let _ = app.emit_to(LABEL, "maru-correction", view);
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn correction_open_snapshot(
    app: AppHandle,
    state: S<'_>,
    request_id: u64,
) -> Result<(), String> {
    open(&app, &state, Target::Snapshot { id: request_id })
}
#[tauri::command]
pub async fn correction_open_scan(
    app: AppHandle,
    state: S<'_>,
    scan_id: u64,
    item: u32,
) -> Result<(), String> {
    open(&app, &state, Target::Scan { id: scan_id, item })
}
#[tauri::command]
pub fn correction_view(
    state: S<'_>,
    editor: State<'_, CorrectionState>,
) -> Result<CorrectionView, String> {
    let target = (*editor.target.lock().unwrap_or_else(|p| p.into_inner()))
        .ok_or("Open a correction from a snapshot first.")?;
    state
        .engine()
        .ocr_correction_view(target)
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn correction_apply(
    app: AppHandle,
    state: S<'_>,
    editor: State<'_, CorrectionState>,
    target: Target,
    text: String,
    remember: bool,
) -> Result<(), String> {
    let mut current = editor.target.lock().unwrap_or_else(|p| p.into_inner());
    if *current != Some(target) {
        return Err("The editor changed; reopen the correction from the snapshot.".into());
    }
    match target {
        Target::Snapshot { id } => {
            let (capture, _, rx) = state
                .engine()
                .correct_snapshot(id, &text, remember)
                .map_err(|e| e.to_string())?;
            state.panel().last_capture = Some(capture);
            windows::place_and_show(&app, &state, state.cfg().ui.placement);
            flow::pump(app.clone(), rx);
        }
        Target::Scan { id, item } => {
            let mut rx = state
                .engine()
                .correct_scan(id, item, &text, remember)
                .map_err(|e| e.to_string())?;
            let handle = app.clone();
            tauri::async_runtime::spawn(async move {
                while let Some(event) = rx.recv().await {
                    let done = matches!(event, maru_core::Event::Done { .. });
                    let _ = handle.emit_to(windows::SCAN, "maru-scan", event);
                    if done {
                        break;
                    }
                }
            });
        }
    }
    *current = None;
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.hide();
    }
    Ok(())
}
#[tauri::command]
pub fn correction_forget(state: S<'_>, target: Target) -> Result<(), String> {
    state
        .engine()
        .forget_ocr_fix(target)
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn correction_cancel(app: AppHandle) {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.hide();
    }
}
