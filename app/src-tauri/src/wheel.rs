//! Focused shortcut controls for games that hide or confine the pointer.
use crate::{flow, input_session, launcher, logic, reading, state::AppState, windows};
use maru_core::{Point, Rect};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, State, WebviewUrl,
    WebviewWindowBuilder,
};

const LABEL: &str = "wheel";
#[derive(Default)]
pub struct WheelState {
    ready: AtomicBool,
    opening: tokio::sync::Mutex<()>,
    revision: AtomicU64,
}

fn placement(pointer: Point, monitor: Rect, scale: f32) -> (Point, f32) {
    let side = (440.0 * scale)
        .min((monitor.w - 16.0).max(1.0))
        .min((monitor.h - 16.0).max(1.0));
    (
        logic::keep_on_screen(
            Point {
                x: pointer.x - side / 2.0,
                y: pointer.y - side / 2.0,
            },
            (side, side),
            monitor,
        ),
        side,
    )
}

async fn show(
    app: &AppHandle,
    state: &Arc<AppState>,
    reset: bool,
    expected: Option<u64>,
    expected_foreground: Option<u64>,
) -> Result<(), String> {
    let controls = app.state::<WheelState>();
    let _opening = controls.opening.lock().await;
    if expected.is_some_and(|id| controls.revision.load(Ordering::Acquire) != id) {
        return Ok(());
    }
    let revision = if reset {
        controls.revision.fetch_add(1, Ordering::AcqRel) + 1
    } else {
        controls.revision.load(Ordering::Acquire)
    };
    if app.get_webview_window(windows::PICKER).is_some() {
        return Err("Finish or cancel the current area selection first.".into());
    }
    state.engine().reading_context();
    let snapshot = input_session::prepare(app).await?;
    if expected_foreground.is_some_and(|id| snapshot.foreground() != id) {
        return Ok(());
    }
    let pointer = state
        .engine()
        .platform()
        .screen
        .cursor()
        .unwrap_or_else(|| {
            let r = windows::virtual_screen(app);
            Point {
                x: r.x + r.w / 2.0,
                y: r.y + r.h / 2.0,
            }
        });
    let window = if let Some(window) = app.get_webview_window(LABEL) {
        window
    } else {
        controls.ready.store(false, Ordering::Release);
        WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("wheel.html".into()))
            .title("Maru controls wheel")
            .inner_size(440.0, 440.0)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .resizable(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .content_protected(true)
            .focused(false)
            .visible(false)
            .build()
            .map_err(|e| e.to_string())?
    };
    // The page installs its listener before acknowledging readiness, including first opening.
    let ready = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !controls.ready.load(Ordering::Acquire)
            && controls.revision.load(Ordering::Acquire) == revision
        {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await;
    if ready.is_err() {
        return Err("The controls wheel did not finish loading. Try again.".into());
    }
    if controls.revision.load(Ordering::Acquire) != revision {
        return Ok(());
    }
    let scale = app
        .monitor_from_point(pointer.x as f64, pointer.y as f64)
        .map_err(|e| e.to_string())?
        .map(|monitor| monitor.scale_factor())
        .unwrap_or(window.scale_factor().map_err(|e| e.to_string())?) as f32;
    let (at, side) = placement(pointer, windows::monitor_rect(app, pointer), scale);
    window
        .set_size(PhysicalSize::new(side as u32, side as u32))
        .map_err(|e| e.to_string())?;
    window
        .set_position(PhysicalPosition::new(at.x as i32, at.y as i32))
        .map_err(|e| e.to_string())?;
    window
        .set_ignore_cursor_events(false)
        .map_err(|e| e.to_string())?;
    if let Err(e) = input_session::show(app, &window, snapshot, false).await {
        let _ = input_session::finish(app, &window).await;
        let _ = window.hide();
        return Err(e);
    }
    if reset {
        app.emit_to(LABEL, "maru-wheel-show", ())
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn wheel_ready(controls: State<'_, WheelState>) {
    controls.ready.store(true, Ordering::Release);
}
#[tauri::command]
pub fn wheel_status(app: AppHandle, state: State<'_, Arc<AppState>>) -> launcher::LauncherStatus {
    launcher::launcher_status(app, state)
}
#[tauri::command]
pub async fn wheel_close(app: AppHandle) -> Result<(), String> {
    let controls = app.state::<WheelState>();
    controls.revision.fetch_add(1, Ordering::AcqRel);
    let _opening = controls.opening.lock().await;
    if let Some(window) = app.get_webview_window(LABEL) {
        let result = input_session::finish(&app, &window).await;
        window.hide().map_err(|e| e.to_string())?;
        if let Err(error) = result {
            tracing::warn!("controls wheel return: {error}");
        }
    }
    Ok(())
}
pub fn hotkey(app: AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        let result = if app
            .get_webview_window(LABEL)
            .is_some_and(|w| w.is_visible().unwrap_or(false))
        {
            wheel_close(app.clone()).await
        } else {
            show(&app, &state, true, None, None).await
        };
        if let Err(e) = result {
            tracing::warn!("controls wheel: {e}");
            flow::notice(&app, &state, None, "info", e);
        }
    });
}

pub fn invalidate_recovery(app: &AppHandle) {
    if let Some(controls) = app.try_state::<WheelState>() {
        controls.revision.fetch_add(1, Ordering::AcqRel);
    }
}

#[tauri::command]
pub async fn wheel_action(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    action: String,
) -> Result<(), String> {
    let shared = state.inner().clone();
    let picker = matches!(action.as_str(), "region" | "continuous_area");
    let hub = hub_action(&action);
    let source = input_session::return_window(&app);
    if !picker && hub.is_none() {
        wheel_close(app.clone()).await?;
    }
    let revision = app.state::<WheelState>().revision.load(Ordering::Acquire);
    let started = std::time::Instant::now();
    tracing::info!(%action,"controls wheel action started");
    let result = if let Some(tab) = hub {
        shared.engine().reading_context();
        windows::open_hub_from_controls(&app, tab).await
    } else {
        match action.as_str() {
            "reading_read" => {
                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                reading::read_again(app.clone(), shared.clone()).await
            }
            "reading_auto" => shared
                .engine()
                .set_reading_auto(!shared.engine().reading_status().automatic)
                .map_err(|e| e.to_string()),
            "reading_hold" => {
                shared
                    .engine()
                    .set_reading_hold(!shared.engine().reading_status().held);
                Ok(())
            }
            _ => launcher::launcher_action(app.clone(), state, action.clone()).await,
        }
    };
    if result.is_ok() {
        if (picker || hub.is_some())
            && app.state::<WheelState>().revision.load(Ordering::Acquire) == revision
            && let Some(window) = app.get_webview_window(LABEL)
        {
            window.hide().map_err(|e| e.to_string())?;
        }
    } else if let Some(source) = source {
        let _ = show(&app, &shared, false, Some(revision), Some(source)).await;
    }
    if let Err(error) = &result {
        tracing::warn!(%action,%error,elapsed_ms=started.elapsed().as_millis(),"controls wheel action failed");
    } else {
        tracing::info!(%action,elapsed_ms=started.elapsed().as_millis(),"controls wheel action accepted");
    }
    result
}

fn hub_action(action: &str) -> Option<&str> {
    match action {
        "reading" | "reading_save" => Some("reading"),
        "settings" | "history" | "review" | "resources" | "status" => Some(action),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manager_actions_keep_maru_focus_and_game_actions_return_to_the_game() {
        for action in [
            "reading",
            "reading_save",
            "settings",
            "history",
            "review",
            "resources",
            "status",
        ] {
            assert!(hub_action(action).is_some());
        }
        for action in [
            "reading_read",
            "region",
            "continuous",
            "scan",
            "speak",
            "hover",
        ] {
            assert!(hub_action(action).is_none());
        }
        assert_eq!(hub_action("reading_save"), Some("reading"));
    }
    #[test]
    fn the_wheel_fits_the_pointer_monitor_including_negative_origins_and_scaled_edges() {
        for (monitor, pointer, scale) in [
            (
                Rect::new(0.0, 0.0, 1920.0, 1080.0),
                Point {
                    x: 1910.0,
                    y: 1070.0,
                },
                1.0,
            ),
            (
                Rect::new(-1920.0, -1080.0, 1920.0, 1080.0),
                Point {
                    x: -1910.0,
                    y: -1070.0,
                },
                2.0,
            ),
            (
                Rect::new(0.0, 0.0, 800.0, 600.0),
                Point { x: 795.0, y: 595.0 },
                2.0,
            ),
        ] {
            let (at, side) = placement(pointer, monitor, scale);
            assert!(at.x >= monitor.x && at.y >= monitor.y);
            assert!(at.x + side <= monitor.x + monitor.w && at.y + side <= monitor.y + monitor.h);
        }
    }
}
