//! One selection token per picker; old pages cannot finish a newer selection.
use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering},
};
use tauri::{AppHandle, Manager, State};

#[derive(Default)]
pub struct PickerSessionState {
    next: AtomicU64,
    active: Mutex<Option<Session>>,
}

struct Session {
    id: u64,
    ready: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum Readiness {
    Waiting,
    Ready,
    Replaced,
}

impl PickerSessionState {
    fn begin(&self) -> u64 {
        let mut active = self.active.lock().unwrap_or_else(|p| p.into_inner());
        let id = self.next.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        *active = Some(Session { id, ready: false });
        id
    }

    fn current(&self) -> Option<u64> {
        self.active
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .map(|session| session.id)
    }

    fn accepts(&self, id: Option<u64>) -> bool {
        self.active
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .is_some_and(|session| id.is_none_or(|id| session.id == id))
    }

    fn clear(&self, id: u64) -> bool {
        let mut active = self.active.lock().unwrap_or_else(|p| p.into_inner());
        if active.as_ref().is_some_and(|session| session.id == id) {
            *active = None;
            true
        } else {
            false
        }
    }

    fn ready(&self, id: u64) -> bool {
        let mut active = self.active.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(session) = active.as_mut().filter(|session| session.id == id) {
            session.ready = true;
            true
        } else {
            false
        }
    }

    fn readiness(&self, id: u64) -> Readiness {
        let active = self.active.lock().unwrap_or_else(|p| p.into_inner());
        match active.as_ref().filter(|session| session.id == id) {
            Some(session) if session.ready => Readiness::Ready,
            Some(_) => Readiness::Waiting,
            None => Readiness::Replaced,
        }
    }
}

pub fn begin(app: &AppHandle) -> u64 {
    app.state::<PickerSessionState>().begin()
}
pub fn current(app: &AppHandle) -> Option<u64> {
    app.state::<PickerSessionState>().current()
}
pub fn accepts(app: &AppHandle, session_id: Option<u64>) -> bool {
    app.state::<PickerSessionState>().accepts(session_id)
}
pub fn clear(app: &AppHandle, id: u64) {
    app.state::<PickerSessionState>().clear(id);
}

pub async fn wait_ready(app: &AppHandle, id: u64) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match app.state::<PickerSessionState>().readiness(id) {
            Readiness::Ready => return Ok(()),
            Readiness::Replaced => return Err("This area selection is no longer current.".into()),
            Readiness::Waiting => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("The area selector did not finish loading. Try again.".into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

#[tauri::command]
pub async fn picker_session(app: AppHandle) -> Result<u64, String> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Some(id) = current(&app) {
            return Ok(id);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("The area selector could not start its selection. Try again.".into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

#[tauri::command]
pub fn picker_ready(state: State<'_, PickerSessionState>, session_id: u64) -> Result<(), String> {
    if state.ready(session_id) {
        Ok(())
    } else {
        Err("This area selection is no longer current.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_picker_replaces_its_previous_token_and_starts_unready() {
        let state = PickerSessionState::default();
        let old = state.begin();
        assert!(state.ready(old));
        let new = state.begin();
        assert_ne!(old, new);
        assert_eq!(state.current(), Some(new));
        assert!(!state.accepts(Some(old)));
        assert!(state.accepts(Some(new)));
        assert_eq!(state.readiness(new), Readiness::Waiting);
    }

    #[test]
    fn a_late_ready_acknowledgement_cannot_make_a_new_picker_ready() {
        let state = PickerSessionState::default();
        let old = state.begin();
        let new = state.begin();
        assert!(!state.ready(old));
        assert_eq!(state.readiness(old), Readiness::Replaced);
        assert_eq!(state.readiness(new), Readiness::Waiting);
        assert!(state.ready(new));
        assert_eq!(state.readiness(new), Readiness::Ready);
        assert!(!state.ready(old));
        assert_eq!(state.readiness(new), Readiness::Ready);
    }

    #[test]
    fn an_old_native_close_cannot_clear_a_new_selection() {
        let state = PickerSessionState::default();
        let old = state.begin();
        let new = state.begin();
        assert!(!state.clear(old));
        assert_eq!(state.current(), Some(new));
        assert!(state.accepts(Some(new)));
    }

    #[test]
    fn a_completed_selection_is_not_accepted_again_or_acknowledged_ready() {
        let state = PickerSessionState::default();
        let id = state.begin();
        assert!(state.clear(id));
        assert!(!state.clear(id));
        assert!(!state.accepts(Some(id)));
        assert!(!state.accepts(None));
        assert!(!state.ready(id));
        assert_eq!(state.readiness(id), Readiness::Replaced);
    }

    #[test]
    fn a_legacy_completion_requires_a_current_selection() {
        let state = PickerSessionState::default();
        assert!(!state.accepts(None));
        let id = state.begin();
        assert!(state.accepts(None));
        assert!(state.clear(id));
        assert!(!state.accepts(None));
    }

    #[test]
    fn clearing_a_picker_does_not_reuse_its_previous_token() {
        let state = PickerSessionState::default();
        let old = state.begin();
        state.clear(old);
        let new = state.begin();
        assert_ne!(new, old);
        assert_eq!(state.current(), Some(new));
        assert!(!state.accepts(Some(old)));
    }
}
