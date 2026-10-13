//! Thin commands for the engine's local diagnostic sampler.
use maru_engine::resource_monitor::{ResourceService, ResourceStatus};
use tauri::{AppHandle, Manager, State};

#[tauri::command]
pub fn resource_status(window: tauri::WebviewWindow, service: State<'_, ResourceService>) -> ResourceStatus {
    if window.is_visible().unwrap_or(false) {
        service.status()
    } else {
        service.cached_status()
    }
}
#[tauri::command]
pub async fn resource_record(service: State<'_, ResourceService>, enabled: bool) -> Result<(), String> {
    service.record(enabled).await
}
#[tauri::command]
pub async fn resource_enable(service: State<'_, ResourceService>, enabled: bool) -> Result<(), String> {
    service.enable(enabled).await
}

pub fn event(app: &AppHandle, event: &maru_core::Event, source: &str) {
    if let Some(service) = app.try_state::<ResourceService>() {
        service.event(event, source);
    }
}
