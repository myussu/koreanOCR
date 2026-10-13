//! Saved-region controls and forwarding; capture scheduling belongs to the engine.
use crate::{flow, logic, state::AppState, windows};
use maru_core::Event;
use maru_engine::reading_areas::ReadingStatus;
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};

type S<'a> = State<'a, Arc<AppState>>;
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> maru_engine::engine::Result<T> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

pub fn picked(app: AppHandle, state: Arc<AppState>, rect: Option<maru_core::Rect>) {
    tauri::async_runtime::spawn(async move {
        let engine = state.engine().clone();
        match blocking(move || engine.finish_reading_pick(rect)).await {
            Ok(Some(_)) => {
                let _ = windows::open_hub(&app, "reading");
            }
            Ok(None) => {}
            Err(e) => flow::notice(&app, &state, None, "info", e),
        }
    });
}

pub fn spawn(app: AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(300));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            state.engine().observe_reading().await;
            let Some(job) = state.engine().take_reading_job() else {
                continue;
            };
            let capture = job.capture;
            {
                let mut panel = state.panel();
                panel.anchor = capture
                    .rect
                    .or_else(|| capture.cursor.map(logic::around_point))
                    .unwrap_or_default();
                panel.size = (state.cfg().ui.max_width_px as f32, 160.0);
                if !panel.pinned {
                    panel.custom_pos = None;
                }
                panel.last_capture = Some(capture.clone());
            }
            windows::place_and_show(&app, &state, state.cfg().ui.placement);
            let (id, mut events) = state.engine().translate(capture);
            let (app, state) = (app.clone(), state.clone());
            tauri::async_runtime::spawn(async move {
                let mut failure = None;
                let mut finished = false;
                while let Some(event) = events.recv().await {
                    let done = matches!(event, Event::Done { .. });
                    if let Event::Error { message, .. } = &event {
                        failure = Some(message.clone());
                    }
                    crate::resources::event(&app, &event, "panel");
                    if state.engine().accepts_reading(job.serial) {
                        let _ = app.emit_to(windows::PANEL, "maru-event", &event);
                    }
                    if done {
                        finished = true;
                        break;
                    }
                }
                if !finished
                    && state.engine().accepts_reading(job.serial)
                    && state
                        .engine()
                        .ocr_correction_view(maru_engine::ocr_corrections::Target::Snapshot { id })
                        .is_ok()
                {
                    let message =
                        "The reading stopped before completion; try Read again.".to_string();
                    let _ = app.emit_to(
                        windows::PANEL,
                        "maru-event",
                        Event::Error {
                            id,
                            stage: maru_core::Stage::Llm,
                            message: message.clone(),
                        },
                    );
                    let _ = app.emit_to(windows::PANEL, "maru-event", Event::Done { id });
                    failure = Some(message);
                }
                state.engine().finish_reading(job.serial, failure);
            });
        }
    });
}

#[tauri::command]
pub fn reading_status(state: S<'_>) -> ReadingStatus {
    state.engine().reading_context();
    state.engine().reading_status()
}
#[derive(Serialize)]
pub struct PickerHint {
    name: String,
    rect: Option<maru_core::Rect>,
    origin: maru_core::Point,
}
#[tauri::command]
pub fn reading_picker_hint(app: AppHandle, state: S<'_>) -> Option<PickerHint> {
    let (name, rect) = state.engine().reading_picker_hint()?;
    let origin = app
        .get_webview_window(windows::PICKER)?
        .outer_position()
        .ok()?;
    Some(PickerHint {
        name,
        rect,
        origin: maru_core::Point {
            x: origin.x as f32,
            y: origin.y as f32,
        },
    })
}

#[tauri::command]
pub async fn reading_create(
    app: AppHandle,
    state: S<'_>,
    name: String,
    area_id: Option<i64>,
) -> Result<(), String> {
    if app.get_webview_window(windows::PICKER).is_some() {
        return Err("Finish or cancel the current area selection first.".into());
    }
    let engine = state.engine().clone();
    blocking(move || engine.begin_reading_pick(&name, area_id)).await?;
    if let Some(hub) = app.get_webview_window(windows::HUB) {
        let _ = hub.hide();
    }
    if let Err(e) = windows::open_picker(&app) {
        let _ = state.engine().finish_reading_pick(None);
        return Err(e.to_string());
    }
    Ok(())
}

#[tauri::command]
pub async fn reading_select(state: S<'_>, area_id: i64) -> Result<(), String> {
    let engine = state.engine().clone();
    blocking(move || engine.select_reading_area(area_id)).await
}
#[tauri::command]
pub async fn reading_rename(state: S<'_>, area_id: i64, name: String) -> Result<(), String> {
    let engine = state.engine().clone();
    blocking(move || engine.rename_reading_area(area_id, &name)).await
}
#[tauri::command]
pub async fn reading_remove(state: S<'_>, area_id: i64) -> Result<(), String> {
    let engine = state.engine().clone();
    blocking(move || engine.remove_reading_area(area_id)).await
}
#[tauri::command]
pub fn reading_auto(state: S<'_>, enabled: bool) -> Result<(), String> {
    state
        .engine()
        .set_reading_auto(enabled)
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn reading_hold(state: S<'_>, held: bool) {
    state.engine().set_reading_hold(held);
}
#[tauri::command]
pub async fn reading_confirm_position(state: S<'_>) -> Result<(), String> {
    let engine = state.engine().clone();
    blocking(move || engine.confirm_reading_position()).await
}

pub async fn read_again(app: AppHandle, state: Arc<AppState>) -> Result<(), String> {
    if let Some(hub) = app
        .get_webview_window(windows::HUB)
        .filter(|w| w.is_visible().unwrap_or(false))
    {
        hub.hide().map_err(|e| e.to_string())?;
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
    state
        .engine()
        .request_reading()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn reading_read(app: AppHandle, state: S<'_>) -> Result<(), String> {
    read_again(app, state.inner().clone()).await
}

pub fn hotkey(app: AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        if let Err(e) = read_again(app.clone(), state.clone()).await {
            flow::notice(&app, &state, None, "info", e);
        }
    });
}
