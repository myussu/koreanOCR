//! Visible acknowledgement of manual saved-area reads, including time spent in OCR.
use crate::{logic, state::AppState, windows};
use maru_core::{Point, Rect};
use maru_engine::reading_areas::ReadingStatus;
use serde::Serialize;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::{Duration, Instant};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, State, WebviewUrl,
    WebviewWindowBuilder,
};

const LABEL: &str = "reading-progress";
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Progress {
    id: u64,
    area: String,
    phase: &'static str,
    message: String,
}
struct Operation {
    value: Progress,
    area_id: Option<i64>,
    starting: bool,
    expires: Option<Instant>,
    anchor: Rect,
}
#[derive(Default)]
pub struct FeedbackState {
    next: AtomicU64,
    revision: AtomicU64,
    operation: Mutex<Option<Operation>>,
    ready: AtomicBool,
    opening: tokio::sync::Mutex<()>,
    pub capture: tokio::sync::Mutex<()>,
}

fn message(phase: &str) -> &str {
    match phase {
        "capturing" => "Capturing and reading the Korean text…",
        "queued" => "Text captured · waiting for translation…",
        "reading" => "Translating and preparing word explanations…",
        "done" => "Reading ready · the translation panel is available.",
        _ => "Read again to retry.",
    }
}
fn progress_phase(status: &ReadingStatus) -> &'static str {
    match status.phase.as_str() {
        "capturing" => "capturing",
        "queued" => "queued",
        "reading" => "reading",
        "error" | "paused" => "error",
        _ => {
            if status.pending {
                "queued"
            } else if status.busy {
                "reading"
            } else {
                "done"
            }
        }
    }
}
fn placement(anchor: Rect, monitor: Rect, scale: f32) -> (Point, (f32, f32)) {
    let size = (
        (420.0 * scale).min((monitor.w - 16.0).max(1.0)),
        (96.0 * scale).min((monitor.h - 16.0).max(1.0)),
    );
    (
        logic::place_panel(anchor, size, monitor, maru_core::config::Placement::Auto),
        size,
    )
}

pub fn begin(app: &AppHandle, state: &AppState) -> u64 {
    let status = state.engine().reading_status();
    let area = status
        .areas
        .iter()
        .find(|area| Some(area.id) == status.selected)
        .map(|area| area.name.clone())
        .unwrap_or_else(|| "Saved reading area".into());
    let anchor = state.engine().reading_area_rect().unwrap_or_else(|| {
        logic::around_point(
            state
                .engine()
                .platform()
                .screen
                .cursor()
                .unwrap_or(Point { x: 20.0, y: 20.0 }),
        )
    });
    let feedback = app.state::<FeedbackState>();
    let id = feedback.next.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
    let value = Progress {
        id,
        area,
        phase: "capturing",
        message: message("capturing").into(),
    };
    *feedback.operation.lock().unwrap_or_else(|p| p.into_inner()) = Some(Operation {
        value: value.clone(),
        area_id: status.selected,
        starting: true,
        expires: None,
        anchor,
    });
    feedback.revision.fetch_add(1, Ordering::AcqRel);
    tracing::info!(operation=id,area_id=?status.selected,"saved reading requested");
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = present(&app, id).await {
            tracing::warn!(%error,"reading feedback window");
        }
    });
    id
}

pub fn capture_finished(app: &AppHandle, id: u64, status: &ReadingStatus, error: Option<&str>) {
    let feedback = app.state::<FeedbackState>();
    let mut operation = feedback.operation.lock().unwrap_or_else(|p| p.into_inner());
    let Some(value) = finish_capture(&mut operation, id, status, error) else {
        return;
    };
    feedback.revision.fetch_add(1, Ordering::AcqRel);
    drop(operation);
    let _ = app.emit_to(LABEL, "maru-reading-progress", value);
}
fn finish_capture(
    operation: &mut Option<Operation>,
    id: u64,
    status: &ReadingStatus,
    error: Option<&str>,
) -> Option<Progress> {
    let op = operation.as_mut().filter(|op| op.value.id == id)?;
    op.starting = false;
    set_phase(op, status, error);
    Some(op.value.clone())
}
fn observe(op: &mut Operation, status: &ReadingStatus) -> bool {
    if op.starting || matches!(op.value.phase, "error" | "done") {
        return false;
    }
    let before = op.value.clone();
    if op.area_id != status.selected {
        set_phase(
            op,
            status,
            Some("The selected area changed. Read again to capture it."),
        );
    } else {
        set_phase(op, status, None);
    }
    before != op.value
}
fn set_phase(op: &mut Operation, status: &ReadingStatus, error: Option<&str>) {
    if let Some(error) = error {
        op.value.phase = "error";
        op.value.message = error.into();
        op.expires = None;
        return;
    }
    let phase = progress_phase(status);
    op.value.phase = phase;
    op.value.message = status
        .message
        .clone()
        .unwrap_or_else(|| message(phase).into());
    op.expires = if phase == "done" {
        Some(Instant::now() + Duration::from_secs(6))
    } else {
        None
    };
}

async fn present(app: &AppHandle, id: u64) -> Result<(), String> {
    let feedback = app.state::<FeedbackState>();
    let _opening = feedback.opening.lock().await;
    let window = if let Some(window) = app.get_webview_window(LABEL) {
        window
    } else {
        feedback.ready.store(false, Ordering::Release);
        WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("reading-progress.html".into()))
            .title("Maru reading progress")
            .inner_size(420.0, 96.0)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .resizable(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .focusable(false)
            .focused(false)
            .content_protected(true)
            .visible(false)
            .build()
            .map_err(|e| e.to_string())?
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        while !feedback.ready.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .map_err(|_| "The reading indicator did not finish loading.".to_string())?;
    let (value, anchor) = {
        let operation = feedback.operation.lock().unwrap_or_else(|p| p.into_inner());
        let Some(op) = operation.as_ref().filter(|op| op.value.id == id) else {
            return Ok(());
        };
        (op.value.clone(), op.anchor)
    };
    let point = Point {
        x: anchor.x + anchor.w / 2.0,
        y: anchor.y + anchor.h / 2.0,
    };
    let scale = app
        .monitor_from_point(point.x as f64, point.y as f64)
        .map_err(|e| e.to_string())?
        .map(|m| m.scale_factor())
        .unwrap_or(1.0) as f32;
    let (at, size) = placement(anchor, windows::monitor_rect(app, point), scale);
    let (tx, rx) = tokio::sync::oneshot::channel();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let feedback = handle.state::<FeedbackState>();
        let operation = feedback.operation.lock().unwrap_or_else(|p| p.into_inner());
        let result = (|| {
            let Some(op) = operation.as_ref().filter(|op| op.value.id == value.id) else {
                return Ok(());
            };
            window
                .set_position(PhysicalPosition::new(at.x as i32, at.y as i32))
                .map_err(|e| e.to_string())?;
            window
                .set_size(PhysicalSize::new(size.0 as u32, size.1 as u32))
                .map_err(|e| e.to_string())?;
            window
                .set_ignore_cursor_events(true)
                .map_err(|e| e.to_string())?;
            window.show().map_err(|e| e.to_string())?;
            window.set_always_on_top(true).map_err(|e| e.to_string())?;
            handle
                .emit_to(LABEL, "maru-reading-progress", &op.value)
                .map_err(|e| e.to_string())
        })();
        let _ = tx.send(result);
    })
    .map_err(|e| e.to_string())?;
    rx.await
        .map_err(|_| "Maru's reading indicator stopped responding.".to_string())?
}

pub fn dismiss(app: &AppHandle) {
    crate::wheel::invalidate_recovery(app);
    if let Some(feedback) = app.try_state::<FeedbackState>() {
        *feedback.operation.lock().unwrap_or_else(|p| p.into_inner()) = None;
        feedback.revision.fetch_add(1, Ordering::AcqRel);
    }
    hide_if_idle(app);
}
fn hide_if_idle(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let feedback = handle.state::<FeedbackState>();
        let operation = feedback.operation.lock().unwrap_or_else(|p| p.into_inner());
        if operation.is_none()
            && let Some(window) = handle.get_webview_window(LABEL)
        {
            let _ = window.hide();
        }
    });
}
pub fn protects_answer(app: &AppHandle) -> bool {
    app.try_state::<FeedbackState>().is_some_and(|feedback| {
        feedback
            .operation
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .is_some_and(|op| {
                op.value.phase != "error" && op.expires.is_none_or(|until| Instant::now() < until)
            })
    })
}

pub fn spawn(app: AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        let mut poll = tokio::time::interval(Duration::from_millis(250));
        poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last_status = String::new();
        loop {
            poll.tick().await;
            let feedback = app.state::<FeedbackState>();
            let revision = feedback.revision.load(Ordering::Acquire);
            let status = state.engine().reading_status();
            if let Ok(encoded) = serde_json::to_string(&status)
                && encoded != last_status
            {
                last_status = encoded;
                let _ = app.emit("maru-reading-status", &status);
            }
            let mut operation = feedback.operation.lock().unwrap_or_else(|p| p.into_inner());
            if revision != feedback.revision.load(Ordering::Acquire) {
                continue;
            }
            let Some(op) = operation.as_mut() else {
                continue;
            };
            if op.expires.is_some_and(|until| Instant::now() >= until) {
                *operation = None;
                drop(operation);
                hide_if_idle(&app);
                continue;
            }
            if observe(op, &status) {
                let value = op.value.clone();
                drop(operation);
                let _ = app.emit_to(LABEL, "maru-reading-progress", &value);
                tracing::info!(
                    operation = value.id,
                    phase = value.phase,
                    "saved reading progress"
                );
            }
        }
    });
}

#[tauri::command]
pub fn reading_progress_ready(feedback: State<'_, FeedbackState>) {
    feedback.ready.store(true, Ordering::Release);
}
#[tauri::command]
pub fn reading_progress_status(feedback: State<'_, FeedbackState>) -> Option<Progress> {
    feedback
        .operation
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .map(|op| op.value.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn status(phase: &str) -> ReadingStatus {
        ReadingStatus {
            areas: Vec::new(),
            selected: Some(1),
            game: None,
            exe: None,
            automatic: false,
            held: false,
            phase: phase.into(),
            message: None,
            pending: phase == "queued",
            busy: phase == "reading",
            can_create: true,
            position_check: false,
        }
    }
    fn operation(id: u64) -> Operation {
        Operation {
            value: Progress {
                id,
                area: "Dialogue".into(),
                phase: "capturing",
                message: message("capturing").into(),
            },
            area_id: Some(1),
            starting: true,
            expires: None,
            anchor: Rect::default(),
        }
    }
    #[test]
    fn a_pre_capture_ready_status_cannot_complete_a_reading_that_has_just_been_requested() {
        let mut op = operation(1);
        assert!(!observe(&mut op, &status("ready")));
        assert_eq!(op.value.phase, "capturing");
        let mut active = Some(op);
        assert_eq!(
            finish_capture(&mut active, 1, &status("queued"), None)
                .unwrap()
                .phase,
            "queued"
        );
        assert!(observe(active.as_mut().unwrap(), &status("reading")));
    }
    #[test]
    fn an_old_capture_error_cannot_replace_the_feedback_for_a_newer_reading() {
        let mut active = Some(operation(2));
        assert!(finish_capture(&mut active, 1, &status("error"), Some("Old error")).is_none());
        assert_eq!(active.as_ref().unwrap().value.id, 2);
        assert_eq!(active.as_ref().unwrap().value.phase, "capturing");
    }
    #[test]
    fn a_reading_failure_remains_visible_when_later_status_polls_are_ready() {
        let mut active = Some(operation(1));
        finish_capture(
            &mut active,
            1,
            &status("ready"),
            Some("No text was detected"),
        );
        let op = active.as_mut().unwrap();
        assert!(!observe(op, &status("ready")));
        assert_eq!(op.value.message, "No text was detected");
        assert!(op.expires.is_none());
    }
    #[test]
    fn a_completed_reading_expires_but_a_changed_area_refuses_the_old_reading() {
        let mut active = Some(operation(1));
        finish_capture(&mut active, 1, &status("queued"), None);
        assert!(observe(active.as_mut().unwrap(), &status("ready")));
        assert!(active.as_ref().unwrap().expires.is_some());
        let mut active = Some(operation(2));
        finish_capture(&mut active, 2, &status("queued"), None);
        let mut changed = status("queued");
        changed.selected = Some(2);
        assert!(observe(active.as_mut().unwrap(), &changed));
        assert_eq!(active.as_ref().unwrap().value.phase, "error");
    }
    #[test]
    fn reading_feedback_stays_on_its_monitor_at_negative_origins_and_large_scales() {
        for (monitor, anchor, scale) in [
            (
                Rect::new(-1920.0, 0.0, 1920.0, 1080.0),
                Rect::new(-500.0, 900.0, 450.0, 100.0),
                1.5,
            ),
            (
                Rect::new(0.0, 0.0, 800.0, 600.0),
                Rect::new(10.0, 20.0, 700.0, 400.0),
                2.0,
            ),
        ] {
            let (at, size) = placement(anchor, monitor, scale);
            assert!(
                at.x >= monitor.x
                    && at.y >= monitor.y
                    && at.x + size.0 <= monitor.right()
                    && at.y + size.1 <= monitor.bottom()
            );
        }
    }
}
