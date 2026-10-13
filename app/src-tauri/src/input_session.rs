//! Temporary pointer/focus ownership for the wheel and region picker.
use maru_platform::input::{InputControl, InputLease, InputSnapshot};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager, WebviewWindow};

pub struct InputState {
    control: Arc<dyn InputControl>,
    lease: Mutex<Option<InputLease>>,
    picker_gate: Arc<tokio::sync::Mutex<()>>,
}
impl Default for InputState {
    fn default() -> Self {
        Self {
            control: maru_platform::input::native(),
            lease: Mutex::new(None),
            picker_gate: Arc::new(tokio::sync::Mutex::new(())),
        }
    }
}

/// Reserve the picker before changing its purpose, including across async window creation.
pub async fn picker_slot(app: &AppHandle) -> tokio::sync::OwnedMutexGuard<()> {
    let gate = app.state::<InputState>().picker_gate.clone();
    gate.lock_owned().await
}
pub async fn reserve_picker(app: &AppHandle) -> Result<tokio::sync::OwnedMutexGuard<()>, String> {
    let slot = picker_slot(app).await;
    if app.get_webview_window(crate::windows::PICKER).is_some() {
        return Err("Finish or cancel the current area selection first.".into());
    }
    Ok(slot)
}

async fn on_ui<T: Send + 'static>(
    app: &AppHandle,
    work: impl FnOnce(AppHandle) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let _ = tx.send(work(handle));
    })
    .map_err(|e| e.to_string())?;
    rx.await
        .map_err(|_| "Maru's window thread stopped responding.".to_string())?
}

/// Capture confinement before any window creation or cursor visibility changes.
pub async fn prepare(app: &AppHandle) -> Result<InputSnapshot, String> {
    on_ui(app, |app| {
        app.state::<InputState>()
            .control
            .snapshot()
            .map_err(|e| e.to_string())
    })
    .await
}

pub fn return_window(app: &AppHandle) -> Option<u64> {
    app.state::<InputState>()
        .lease
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .and_then(InputLease::source)
}

pub async fn show(
    app: &AppHandle,
    window: &WebviewWindow,
    snapshot: InputSnapshot,
    crosshair: bool,
) -> Result<(), String> {
    let window = window.clone();
    on_ui(app, move |app| {
        let state = app.state::<InputState>();
        let mut lease = state.lease.lock().unwrap_or_else(|p| p.into_inner());
        #[cfg(windows)]
        let owner = window.hwnd().map_err(|e| e.to_string())?.0 as usize as u64;
        #[cfg(not(windows))]
        let owner = 0;
        let current = state.control.snapshot().map_err(|e| e.to_string())?;
        let expected = lease
            .as_ref()
            .map(|token| token.owner())
            .unwrap_or(snapshot.foreground());
        if current.foreground() != expected {
            return Err(
                "The active window changed before Maru could open its controls. Try again.".into(),
            );
        }
        window.show().map_err(|e| e.to_string())?;
        let result = if let Some(current) = lease.as_mut() {
            state.control.transfer(current, owner)
        } else {
            state.control.acquire(snapshot, owner).map(|token| {
                *lease = Some(token);
            })
        };
        if let Err(e) = result {
            if lease.as_ref().is_some_and(|token| token.owner() == owner)
                && let Some(token) = lease.take()
            {
                let _ = state.control.finish(token);
            }
            let _ = window.hide();
            return Err(e.to_string());
        }
        window
            .set_cursor_icon(if crosshair {
                tauri::CursorIcon::Crosshair
            } else {
                tauri::CursorIcon::Default
            })
            .map_err(|e| e.to_string())?;
        window.set_cursor_visible(true).map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

/// Consume the token while its window still exists and owns foreground focus.
pub async fn finish(app: &AppHandle, window: &WebviewWindow) -> Result<(), String> {
    let window = window.clone();
    on_ui(app, move |app| {
        let state = app.state::<InputState>();
        let mut lease = state.lease.lock().unwrap_or_else(|p| p.into_inner());
        #[cfg(windows)]
        let owner = window.hwnd().map_err(|e| e.to_string())?.0 as usize as u64;
        #[cfg(not(windows))]
        let owner = 0;
        if lease.as_ref().is_some_and(|token| token.owner() == owner)
            && let Some(token) = lease.take()
        {
            state.control.finish(token).map_err(|e| e.to_string())?;
        }
        Ok(())
    })
    .await
}

pub fn focus_changed(app: &AppHandle) {
    let Some(state) = app.try_state::<InputState>() else {
        return;
    };
    // Activation may synchronously send a focus event while a controlled transfer holds this lock.
    let Ok(mut lease) = state.lease.try_lock() else {
        return;
    };
    if let Some(token) = lease.as_mut()
        && !state.control.still_owns(token)
    {
        token.focus_lost();
    }
}

/// Exit callbacks run on the GUI thread, before owned windows disappear.
pub fn finish_all(app: &AppHandle) {
    let Some(state) = app.try_state::<InputState>() else {
        return;
    };
    let mut lease = state.lease.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(token) = lease.take()
        && let Err(error) = state.control.finish(token)
    {
        tracing::warn!("pointer control return on exit: {error}");
    }
}
