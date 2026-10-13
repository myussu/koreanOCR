//! Floating tray controls. Only interactive rectangles take clicks; captures exclude this window.

use crate::{flow, logic, state::AppState, windows};
use maru_core::{Point, Rect, live_model::LiveModel};
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, LogicalSize, Manager, PhysicalPosition, State, WebviewUrl, WebviewWindowBuilder};

const LABEL: &str = "launcher";
#[derive(Default)]
pub struct LauncherState {
    rects: Mutex<(f32, Vec<Rect>)>,
    drag_origin: Mutex<Option<(i32, i32)>>,
    anchor: Mutex<Option<(f32, f32)>>,
    offset: Mutex<(f32, f32)>,
    picker_hidden: std::sync::atomic::AtomicBool,
    last_error: Mutex<Option<String>>,
}

pub fn create(app: &AppHandle, state: Arc<AppState>) -> tauri::Result<()> {
    let monitor = app
        .primary_monitor()?
        .map(|m| Rect::new(m.position().x as f32, m.position().y as f32, m.size().width as f32, m.size().height as f32))
        .unwrap_or(windows::virtual_screen(app));
    let position_file = state.services.paths.data_dir.join("launcher-position.json");
    let saved: Option<(f32, f32)> = std::fs::read(&position_file).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let at = saved
        .filter(|(x, y)| x.is_finite() && y.is_finite())
        .map(|(x, y)| Point { x, y })
        .unwrap_or(Point {
            x: monitor.x + monitor.w - 80.0,
            y: monitor.y + 24.0,
        });
    let at = logic::keep_on_screen(at, (64.0, 64.0), windows::monitor_rect(app, at));
    let win = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("launcher.html".into()))
        .title("Maru controls")
        .inner_size(64.0, 64.0)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focusable(false)
        .content_protected(true)
        .visible(false)
        .build()?;
    let size = win.outer_size()?;
    let at = logic::keep_on_screen(at, (size.width as f32, size.height as f32), windows::monitor_rect(app, at));
    win.set_position(PhysicalPosition::new(at.x as i32, at.y as i32))?;
    win.set_ignore_cursor_events(true)?;
    if state.cfg().ui.launcher {
        win.show()?;
    }
    app.manage(LauncherState::default());
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut poll = tokio::time::interval(std::time::Duration::from_millis(12));
        poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut interactive = false;
        loop {
            poll.tick().await;
            let Some(win) = app.get_webview_window(LABEL) else { break };
            let controls = app.state::<LauncherState>();
            let dragging = controls.drag_origin.lock().unwrap_or_else(|p| p.into_inner()).is_some();
            let over = if win.is_visible().unwrap_or(false) {
                let origin = win.outer_position().ok();
                let pointer = state.engine().platform().screen.cursor();
                let rects = controls.rects.lock().unwrap_or_else(|p| p.into_inner());
                match (origin, pointer) {
                    (Some(o), Some(at)) => dragging || logic::over_label(at, (o.x as f32, o.y as f32), rects.0, &rects.1),
                    _ => dragging,
                }
            } else {
                false
            };
            if over != interactive {
                let _ = win.set_ignore_cursor_events(!over);
                interactive = over;
            }
        }
    });
    Ok(())
}

pub fn show(app: &AppHandle, visible: bool) {
    if let Some(win) = app.get_webview_window(LABEL) {
        let _ = if visible { win.show() } else { win.hide() };
    }
}
pub fn toggle(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(LABEL) {
        show(app, !win.is_visible().unwrap_or(false));
    }
}

pub fn pause_for_picker(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(LABEL).filter(|w| w.is_visible().unwrap_or(false)) {
        if let Some(controls) = app.try_state::<LauncherState>() {
            controls.picker_hidden.store(true, std::sync::atomic::Ordering::Release);
        }
        let _ = win.hide();
    }
}
pub fn resume_after_picker(app: &AppHandle) {
    if let Some(controls) = app.try_state::<LauncherState>()
        && controls.picker_hidden.swap(false, std::sync::atomic::Ordering::AcqRel)
    {
        show(app, true);
    }
}
pub fn translation_error(app: &AppHandle, message: Option<String>) {
    if let Some(controls) = app.try_state::<LauncherState>() {
        *controls.last_error.lock().unwrap_or_else(|p| p.into_inner()) = message;
    }
}

#[derive(Serialize)]
pub struct LauncherLayout {
    up: bool,
    left: bool,
}

struct MenuPlacement {
    at: Point,
    size: (f32, f32),
    offset: (f32, f32),
    layout: LauncherLayout,
}

fn menu_placement(anchor: Point, monitor: Rect, scale: f32) -> MenuPlacement {
    let left = anchor.x < monitor.x + monitor.w / 2.0;
    let up = anchor.y > monitor.y + monitor.h / 2.0;
    // Leave room for the placement helper's screen-edge margin even on high-DPI monitors.
    let size = ((350.0 * scale).min((monitor.w - 16.0).max(1.0)), (620.0 * scale).min((monitor.h - 16.0).max(1.0)));
    let offset = (if left { 0.0 } else { size.0 - 64.0 * scale }, if up { size.1 - 64.0 * scale } else { 0.0 });
    let at = logic::keep_on_screen(
        Point {
            x: anchor.x - offset.0,
            y: anchor.y - offset.1,
        },
        size,
        monitor,
    );
    MenuPlacement {
        at,
        size,
        offset,
        layout: LauncherLayout { up, left },
    }
}

#[tauri::command]
pub fn launcher_expand(app: AppHandle, controls: State<'_, LauncherState>, expanded: bool) -> Result<LauncherLayout, String> {
    let win = app.get_webview_window(LABEL).ok_or("Maru controls are unavailable")?;
    let scale = win.scale_factor().map_err(|e| e.to_string())? as f32;
    let mut anchor = controls.anchor.lock().unwrap_or_else(|p| p.into_inner());
    if !expanded {
        win.set_size(LogicalSize::new(64.0, 64.0)).map_err(|e| e.to_string())?;
        if let Some((x, y)) = anchor.take() {
            win.set_position(PhysicalPosition::new(x as i32, y as i32)).map_err(|e| e.to_string())?;
        }
        *controls.offset.lock().unwrap_or_else(|p| p.into_inner()) = (0.0, 0.0);
        return Ok(LauncherLayout { up: false, left: false });
    }
    let at = win.outer_position().map_err(|e| e.to_string())?;
    let (x, y) = *anchor.get_or_insert((at.x as f32, at.y as f32));
    let monitor = windows::monitor_rect(
        &app,
        Point {
            x: x + 32.0 * scale,
            y: y + 32.0 * scale,
        },
    );
    let placement = menu_placement(Point { x, y }, monitor, scale);
    win.set_size(LogicalSize::new(placement.size.0 / scale, placement.size.1 / scale))
        .map_err(|e| e.to_string())?;
    win.set_position(PhysicalPosition::new(placement.at.x as i32, placement.at.y as i32))
        .map_err(|e| e.to_string())?;
    *controls.offset.lock().unwrap_or_else(|p| p.into_inner()) = placement.offset;
    Ok(placement.layout)
}

#[derive(Serialize)]
pub struct LauncherStatus {
    live: bool,
    hover: bool,
    scan: bool,
    scan_id: Option<u64>,
    scan_join_mode: bool,
    speech: bool,
    model: &'static str,
    model_status: String,
    queued: usize,
    active: usize,
    snapshot: String,
    shortcut: String,
    live_setup: maru_engine::model_controls::LiveSetupStatus,
}

#[tauri::command]
pub fn launcher_status(app: AppHandle, state: State<'_, Arc<AppState>>) -> LauncherStatus {
    let cfg = &state.engine().config().continuous.translation;
    let live = state.continuous.lock().unwrap_or_else(|p| p.into_inner()).is_some();
    let model = if live {
        match cfg.model {
            LiveModel::Main => "Gemma / main model",
            LiveModel::HyMt2 => "HY-MT2 7B Q4",
            LiveModel::Milmmt => "MiLMMT 4B Q6",
            LiveModel::HyMt2Small => "HY-MT2 1.8B Q4",
            LiveModel::Lfm => "LFM2 KoEn 1.2B",
        }
    } else {
        "Gemma / main model"
    };
    let status = if live && cfg.model != LiveModel::Main { state.services.live_llm_status() } else { state.services.llm_status() };
    let mut model_status = match status {
        maru_llm::LlmStatus::Starting => "Loading".into(),
        maru_llm::LlmStatus::Ready => "Ready".into(),
        maru_llm::LlmStatus::Failed(e) => format!("Error: {e}"),
        maru_llm::LlmStatus::Disabled => "Loads on demand".into(),
    };
    if !live && state.engine().config().llm.endpoint.is_none() && state.engine().loaded_translation_role() == Some(maru_llm::model_runtime::ModelRole::Live) {
        model_status = "Gemma loads on the next snapshot".into();
    }
    if let Some(controls) = app.try_state::<LauncherState>()
        && let Some(error) = controls.last_error.lock().unwrap_or_else(|p| p.into_inner()).clone()
    {
        model_status = format!("Error: {error}");
    }
    let snapshot = if !live {
        "Normal Gemma setup".into()
    } else if cfg.snapshot_endpoint.is_some() {
        "Snapshot server; local translation if unavailable".into()
    } else if cfg.model != LiveModel::Main {
        "Live model + offline meanings".into()
    } else {
        "Normal Gemma setup".into()
    };
    let (queued, active) = state.engine().live_work();
    let (hover, speech) = {
        let panel = state.panel();
        (panel.hover_mode, panel.reading)
    };
    let (scan, scan_id, scan_join_mode) = {
        let scan = state.scan();
        (scan.visible, scan.id, scan.visible && scan.join_mode)
    };
    LauncherStatus {
        live,
        hover,
        scan,
        scan_id,
        scan_join_mode,
        speech,
        model,
        model_status,
        queued,
        active,
        snapshot,
        shortcut: state.cfg().hotkeys.launcher,
        live_setup: state.services.live_setup_status(),
    }
}

pub fn over_controls(app: &AppHandle, at: Point) -> bool {
    let Some(win) = app.get_webview_window(LABEL).filter(|w| w.is_visible().unwrap_or(false)) else {
        return false;
    };
    let Some(controls) = app.try_state::<LauncherState>() else { return false };
    let Ok(origin) = win.outer_position() else { return false };
    let rects = controls.rects.lock().unwrap_or_else(|p| p.into_inner());
    logic::over_label(at, (origin.x as f32, origin.y as f32), rects.0, &rects.1)
}

#[tauri::command]
pub async fn launcher_action(app: AppHandle, state: State<'_, Arc<AppState>>, action: String) -> Result<(), String> {
    match action.as_str() {
        "prepare_live" => { state.services.prepare_live_session().await?; },
        "release_models" => { state.services.release_idle_models().await?; },
        "translate" => flow::translate_selection(app, state.inner().clone()),
        "region" => windows::open_picker(&app).map(|_| ()).map_err(|e| e.to_string())?,
        "hover" => flow::toggle_hover(&app, &state),
        "continuous" => flow::toggle_continuous(&app, &state),
        "continuous_area" => flow::choose_continuous_area(&app, &state),
        "scan" => crate::scan_controls::toggle(app, state.inner().clone()),
        "scan_refresh" => {
            if !state.scan().visible { return Err("Turn Label screen on first.".into()); }
            flow::scan_screen(app, state.inner().clone());
        },
        "scan_join_mode" => {
            let (id, enabled) = { let scan = state.scan(); (scan.id, !scan.join_mode) };
            let id = id.ok_or("Turn Label screen on first.")?;
            crate::scan_controls::set_join_mode(&app, &state, id, enabled)?;
        },
        "speak" => flow::speak_selection(app, state.inner().clone()),
        "review" | "history" | "status" | "settings" | "resources" => windows::open_hub(&app, &action).map_err(|e| e.to_string())?,
        "hide" => show(&app, false),
        "quit" => app.exit(0),
        _ => return Err("Unknown Maru control".into()),
    }
    Ok(())
}

#[tauri::command]
pub fn launcher_rects(controls: State<'_, LauncherState>, dpr: f32, rects: Vec<Rect>) {
    if dpr.is_finite() && dpr > 0.0 && rects.len() <= 32 {
        *controls.rects.lock().unwrap_or_else(|p| p.into_inner()) = (dpr, rects);
    }
}

#[tauri::command]
pub fn launcher_drag(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    controls: State<'_, LauncherState>,
    phase: String,
    dx: f32,
    dy: f32,
) -> Result<(), String> {
    if !dx.is_finite() || !dy.is_finite() {
        return Err("Invalid drag position".into());
    }
    let win = app.get_webview_window(LABEL).ok_or("Maru controls are unavailable")?;
    let mut origin = controls.drag_origin.lock().unwrap_or_else(|p| p.into_inner());
    if phase == "start" {
        let at = win.outer_position().map_err(|e| e.to_string())?;
        *origin = Some((at.x, at.y));
    }
    if let Some((x, y)) = *origin {
        let dpr = win.scale_factor().map_err(|e| e.to_string())? as f32;
        let at = Point {
            x: x as f32 + dx * dpr,
            y: y as f32 + dy * dpr,
        };
        let size = win.outer_size().map_err(|e| e.to_string())?;
        let at = logic::keep_on_screen(
            at,
            (size.width as f32, size.height as f32),
            windows::monitor_rect(
                &app,
                Point {
                    x: at.x + size.width as f32 / 2.0,
                    y: at.y + size.height as f32 / 2.0,
                },
            ),
        );
        win.set_position(PhysicalPosition::new(at.x as i32, at.y as i32)).map_err(|e| e.to_string())?;
        if phase == "end" {
            *origin = None;
            let offset = *controls.offset.lock().unwrap_or_else(|p| p.into_inner());
            let saved = (at.x + offset.0, at.y + offset.1);
            let mut anchor = controls.anchor.lock().unwrap_or_else(|p| p.into_inner());
            if anchor.is_some() {
                *anchor = Some(saved);
            }
            let path = state.services.paths.data_dir.join("launcher-position.json");
            let bytes = serde_json::to_vec(&saved).map_err(|e| e.to_string())?;
            std::fs::write(path, bytes).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn floating_menus_open_into_the_monitor_and_fit_small_or_scaled_screens() {
        for (monitor, anchor, scale) in [
            (Rect::new(0.0, 0.0, 1920.0, 1080.0), Point { x: 1830.0, y: 990.0 }, 1.0),
            (Rect::new(-1280.0, 0.0, 1280.0, 720.0), Point { x: -1270.0, y: 10.0 }, 1.25),
            (Rect::new(0.0, 0.0, 800.0, 600.0), Point { x: 600.0, y: 400.0 }, 2.0),
        ] {
            let p = menu_placement(anchor, monitor, scale);
            assert!(p.at.x >= monitor.x && p.at.y >= monitor.y);
            assert!(p.at.x + p.size.0 <= monitor.x + monitor.w && p.at.y + p.size.1 <= monitor.y + monitor.h);
            assert_eq!(p.layout.up, anchor.y > monitor.y + monitor.h / 2.0);
            assert_eq!(p.layout.left, anchor.x < monitor.x + monitor.w / 2.0);
        }
    }
}
