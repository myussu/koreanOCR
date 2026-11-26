//! Temporary pointer/focus ownership for the wheel and region picker.
use maru_platform::input::{InputControl, InputLease, InputSnapshot};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager, WebviewWindow};

pub struct InputState {
    control: Arc<dyn InputControl>,
    lease: Mutex<Option<InputLease>>,
    picker_gate: Arc<tokio::sync::Mutex<()>>,
    hub_raised: std::sync::atomic::AtomicBool,
}
impl Default for InputState {
    fn default() -> Self {
        Self {
            control: maru_platform::input::native(),
            lease: Mutex::new(None),
            picker_gate: Arc::new(tokio::sync::Mutex::new(())),
            hub_raised: std::sync::atomic::AtomicBool::new(false),
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
    show_window(app, window, snapshot, crosshair, false).await
}

/// Show a window that works with the mouse alone, without taking keyboard control or confining anything.
/// Windows refuses to bring Maru forward while another program keeps receiving input; the first click
/// on the window then activates it.
pub async fn show_without_control(app: &AppHandle, window: &WebviewWindow) -> Result<(), String> {
    let window = window.clone();
    on_ui(app, move |_| {
        window.show().map_err(|e| e.to_string())?;
        window.set_cursor_icon(tauri::CursorIcon::Crosshair).map_err(|e| e.to_string())?;
        window.set_cursor_visible(true).map_err(|e| e.to_string())?;
        let _ = window.set_focus();
        Ok(())
    })
    .await
}

/// Keep Maru foreground during an explicit wheel-to-manager action.
pub async fn handoff(
    app: &AppHandle,
    window: &WebviewWindow,
    snapshot: InputSnapshot,
) -> Result<(), String> {
    show_window(app, window, snapshot, false, true).await
}

async fn show_window(
    app: &AppHandle,
    window: &WebviewWindow,
    snapshot: InputSnapshot,
    crosshair: bool,
    normal: bool,
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
        if let Some(stale) = lease.as_ref().map(InputLease::owner)
            && lease_was_abandoned(stale, current.foreground(), owner)
            && let Some(token) = lease.take()
        {
            // The user clicked back into the game while the wheel was open. That wheel is finished:
            // put it away and open the new control from the window that is in front now.
            let _ = state.control.finish(token);
            hide_owner(&app, stale);
        }
        let expected = lease
            .as_ref()
            .map(|token| token.owner())
            .unwrap_or(snapshot.foreground());
        if current.foreground() != expected {
            return Err(
                "The active window changed before Maru could open its controls. Try again.".into(),
            );
        }
        let result = (|| -> Result<(), String> {
            if normal {
                window.unminimize().map_err(|e| e.to_string())?;
                window.set_always_on_top(true).map_err(|e| e.to_string())?;
                state
                    .hub_raised
                    .store(true, std::sync::atomic::Ordering::Release);
            }
            window.show().map_err(|e| e.to_string())?;
            if let Some(current) = lease.as_mut() {
                state.control.transfer(current, owner)
            } else {
                state.control.acquire(snapshot, owner).map(|token| {
                    *lease = Some(token);
                })
            }
            .map_err(|e| e.to_string())?;
            window
                .set_cursor_icon(if crosshair {
                    tauri::CursorIcon::Crosshair
                } else {
                    tauri::CursorIcon::Default
                })
                .map_err(|e| e.to_string())?;
            window.set_cursor_visible(true).map_err(|e| e.to_string())?;
            Ok(())
        })();
        if let Err(e) = result {
            if lease.as_ref().is_some_and(|token| token.owner() == owner)
                && let Some(token) = lease.take()
            {
                let _ = state.control.finish(token);
            }
            let _ = window.hide();
            if normal {
                state
                    .hub_raised
                    .store(false, std::sync::atomic::Ordering::Release);
                let _ = window.set_always_on_top(false);
            }
            return Err(e);
        }
        if normal {
            let _ = lease.take();
        }
        Ok(())
    })
    .await
}

/// A lease belongs to a Maru window in front; once another application is in front, nothing will hand it on.
fn lease_was_abandoned(lease_owner: u64, foreground: u64, opening: u64) -> bool {
    lease_owner != foreground && opening != foreground
}

fn hide_owner(app: &AppHandle, owner: u64) {
    #[cfg(windows)]
    for window in app.webview_windows().into_values() {
        if window.hwnd().is_ok_and(|handle| handle.0 as usize as u64 == owner) {
            let _ = window.hide();
        }
    }
    #[cfg(not(windows))]
    let _ = (app, owner);
}

pub fn release_hub_raise(window: &WebviewWindow) {
    if let Some(state) = window.app_handle().try_state::<InputState>()
        && state
            .hub_raised
            .swap(false, std::sync::atomic::Ordering::AcqRel)
    {
        let _ = window.set_always_on_top(false);
    }
}

pub fn hub_focus_lost(window: &WebviewWindow) {
    let Some(state) = window.app_handle().try_state::<InputState>() else {
        return;
    };
    #[cfg(windows)]
    if let Ok(handle) = window.hwnd()
        && state
            .control
            .snapshot()
            .is_ok_and(|snapshot| snapshot.foreground() == handle.0 as usize as u64)
    {
        return;
    }
    #[cfg(not(windows))]
    let _ = &state;
    release_hub_raise(window);
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
    // Focus moving between Maru's own windows (the selector hiding, the controls coming back) is not the user leaving.
    if let Some(token) = lease.as_mut()
        && !state.control.maru_has_focus(token)
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

#[cfg(test)]
mod tests {
    use super::lease_was_abandoned;

    const WHEEL: u64 = 1;
    const GAME: u64 = 2;
    const PICKER: u64 = 3;

    #[test]
    fn a_wheel_left_for_the_game_no_longer_decides_who_opens_next() {
        assert!(lease_was_abandoned(WHEEL, GAME, PICKER));
    }

    #[test]
    fn a_wheel_still_in_front_hands_its_lease_to_the_next_control() {
        assert!(!lease_was_abandoned(WHEEL, WHEEL, PICKER));
        // Activation can already have moved to the window being opened.
        assert!(!lease_was_abandoned(WHEEL, PICKER, PICKER));
    }
}
