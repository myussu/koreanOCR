//! Temporarily lend keyboard and pointer control to an explicit overlay interaction.

use std::sync::Arc;

use crate::{PlatformError, Result};

/// Capture this before showing a window: activation can release a game's cursor clip.
#[derive(Debug)]
pub struct InputSnapshot {
    thread: u32,
    foreground: u64,
    source: Option<Window>,
    clip: Bounds,
    desktop: Bounds,
}

impl InputSnapshot {
    pub fn foreground(&self) -> u64 {
        self.foreground
    }

    pub fn source(&self) -> Option<u64> {
        self.source.map(|source| source.handle)
    }
}

/// An owned interaction. Dropping it deliberately changes no operating-system state.
#[derive(Debug)]
pub struct InputLease {
    snapshot: InputSnapshot,
    owner: u64,
    released: Bounds,
    returning: bool,
}

impl InputLease {
    pub fn owner(&self) -> u64 {
        self.owner
    }

    pub fn source(&self) -> Option<u64> {
        self.snapshot.source()
    }

    /// An observed switch to another application permanently cancels automatic return.
    pub fn focus_lost(&mut self) {
        self.returning = false;
    }
}

/// Call all methods on the UI thread. A lease can move between Maru windows without
/// returning control to the game between them; finish consumes it at most once.
pub trait InputControl: Send + Sync {
    fn snapshot(&self) -> Result<InputSnapshot>;
    fn acquire(&self, snapshot: InputSnapshot, owner: u64) -> Result<InputLease>;
    fn transfer(&self, lease: &mut InputLease, owner: u64) -> Result<()>;
    fn finish(&self, lease: InputLease) -> Result<()>;
    fn still_owns(&self, lease: &InputLease) -> bool;
}

pub fn native() -> Arc<dyn InputControl> {
    #[cfg(windows)]
    {
        Arc::new(Control::new(crate::windows::input::WindowsInput))
    }
    #[cfg(not(windows))]
    {
        Arc::new(Control::new(Unsupported))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Bounds {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Window {
    pub handle: u64,
    pub process: u32,
    pub thread: u32,
    pub client: Bounds,
}

pub(crate) trait Backend: Send + Sync {
    fn thread(&self) -> u32;
    fn process(&self) -> u32;
    fn foreground(&self) -> u64;
    fn window(&self, handle: u64) -> Option<Window>;
    fn desktop(&self) -> Result<Bounds>;
    fn clip(&self) -> Result<Bounds>;
    fn activate(&self, handle: u64) -> Result<()>;
    fn release_clip(&self) -> Result<()>;
    fn confine(&self, bounds: Bounds) -> Result<()>;
}

pub(crate) struct Control<B>(B);

impl<B: Backend> Control<B> {
    pub(crate) fn new(backend: B) -> Self {
        Self(backend)
    }

    fn check_thread(&self, snapshot: &InputSnapshot) -> Result<()> {
        if snapshot.thread != self.0.thread() {
            return Err(PlatformError::Failed(
                "Maru's pointer controls must be closed on the same UI thread that opened them."
                    .into(),
            ));
        }
        Ok(())
    }

    fn check_owner(&self, owner: u64) -> Result<()> {
        if self
            .0
            .window(owner)
            .is_none_or(|window| window.process != self.0.process())
        {
            return Err(PlatformError::Failed(
                "Maru's control window is no longer available.".into(),
            ));
        }
        Ok(())
    }

    fn restore(&self, lease: &InputLease) -> Result<()> {
        self.check_thread(&lease.snapshot)?;
        if !lease.returning || self.0.foreground() != lease.owner {
            return Ok(());
        }
        let Some(source) = lease.snapshot.source else {
            return Ok(());
        };
        if self.0.window(source.handle) != Some(source)
            || self.0.desktop()? != lease.snapshot.desktop
        {
            return Ok(());
        }
        // Another application (or the game itself) may have changed the shared clip.
        let restore_clip =
            self.0.clip()? == lease.released && lease.snapshot.clip != lease.snapshot.desktop;
        self.0.activate(source.handle)?;
        if self.0.foreground() != source.handle {
            return Ok(());
        }
        if restore_clip
            && self.0.clip()? == lease.released
            && self.0.window(source.handle) == Some(source)
            && self.0.desktop()? == lease.snapshot.desktop
        {
            self.0.confine(lease.snapshot.clip)?;
        }
        Ok(())
    }

    fn rollback(&self, lease: &InputLease) {
        if let Err(error) = self.restore(lease) {
            tracing::debug!(%error, "could not return pointer control after an unsuccessful interaction");
        }
    }
}

impl<B: Backend> InputControl for Control<B> {
    fn snapshot(&self) -> Result<InputSnapshot> {
        let foreground = self.0.foreground();
        Ok(InputSnapshot {
            thread: self.0.thread(),
            foreground,
            source: self.0.window(foreground),
            clip: self.0.clip()?,
            desktop: self.0.desktop()?,
        })
    }

    fn acquire(&self, mut snapshot: InputSnapshot, owner: u64) -> Result<InputLease> {
        self.check_thread(&snapshot)?;
        self.check_owner(owner)?;
        let foreground = self.0.foreground();
        if foreground != snapshot.foreground && foreground != owner {
            return Err(PlatformError::Failed(
                "The active window changed before Maru could open its controls. Try again.".into(),
            ));
        }
        if let Some(source) = snapshot.source
            && self.0.window(source.handle).is_none_or(|current| {
                current.process != source.process || current.thread != source.thread
            })
        {
            return Err(PlatformError::Failed(
                "The previous window closed or changed before Maru could open its controls.".into(),
            ));
        }
        // Loading a first-use WebView may take time. Refresh only before activation:
        // once our owner is foreground, the game may already have released its clip.
        if foreground == snapshot.foreground && foreground != owner {
            snapshot.source = self.0.window(foreground);
            snapshot.clip = self.0.clip()?;
            snapshot.desktop = self.0.desktop()?;
        }
        let mut lease = InputLease {
            released: snapshot.desktop,
            snapshot,
            owner,
            returning: true,
        };
        let result = (|| {
            self.0.activate(owner)?;
            if self.0.foreground() != owner {
                return Err(PlatformError::Failed(
                    "Windows did not give Maru keyboard and pointer control. Try the shortcut again."
                        .into(),
                ));
            }
            self.0.release_clip()?;
            lease.released = self.0.clip()?;
            Ok(())
        })();
        if let Err(error) = result {
            self.rollback(&lease);
            return Err(error);
        }
        Ok(lease)
    }

    fn transfer(&self, lease: &mut InputLease, owner: u64) -> Result<()> {
        self.check_thread(&lease.snapshot)?;
        self.check_owner(owner)?;
        if !lease.returning || (self.0.foreground() != lease.owner && self.0.foreground() != owner)
        {
            return Err(PlatformError::Failed(
                "The active window changed. Open Maru's controls again before selecting an area."
                    .into(),
            ));
        }
        let previous_owner = lease.owner;
        // If activation partly succeeds, finish must still recognize our new owner.
        lease.owner = owner;
        let result = (|| {
            self.0.activate(owner)?;
            if self.0.foreground() != owner {
                return Err(PlatformError::Failed(
                    "Windows did not give Maru's area selector keyboard and pointer control."
                        .into(),
                ));
            }
            self.0.release_clip()?;
            lease.released = self.0.clip()?;
            Ok(())
        })();
        if result.is_err() && self.0.foreground() == previous_owner {
            lease.owner = previous_owner;
        }
        result
    }

    fn finish(&self, lease: InputLease) -> Result<()> {
        self.restore(&lease)
    }

    fn still_owns(&self, lease: &InputLease) -> bool {
        self.0.foreground() == lease.owner
    }
}

#[cfg(not(windows))]
struct Unsupported;

#[cfg(not(windows))]
impl Backend for Unsupported {
    fn thread(&self) -> u32 {
        0
    }
    fn process(&self) -> u32 {
        0
    }
    fn foreground(&self) -> u64 {
        0
    }
    fn window(&self, _handle: u64) -> Option<Window> {
        None
    }
    fn desktop(&self) -> Result<Bounds> {
        Err(PlatformError::Unsupported("interactive overlay input"))
    }
    fn clip(&self) -> Result<Bounds> {
        Err(PlatformError::Unsupported("interactive overlay input"))
    }
    fn activate(&self, _handle: u64) -> Result<()> {
        Err(PlatformError::Unsupported("interactive overlay input"))
    }
    fn release_clip(&self) -> Result<()> {
        Err(PlatformError::Unsupported("interactive overlay input"))
    }
    fn confine(&self, _bounds: Bounds) -> Result<()> {
        Err(PlatformError::Unsupported("interactive overlay input"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    const GAME: u64 = 7;
    const WHEEL: u64 = 99;
    const PICKER: u64 = 100;
    const OTHER: u64 = 8;
    const DESKTOP: Bounds = Bounds {
        left: -1920,
        top: 0,
        right: 1920,
        bottom: 1080,
    };
    const GAME_CLIP: Bounds = Bounds {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1080,
    };

    #[derive(Debug, PartialEq)]
    enum Action {
        Activate(u64),
        Release,
        Confine(Bounds),
    }

    struct Desktop {
        foreground: u64,
        thread: u32,
        windows: BTreeMap<u64, Window>,
        desktop: Bounds,
        clip: Bounds,
        actions: Vec<Action>,
        clip_on_game_focus: Option<Bounds>,
        failing_activation: Option<(u64, bool)>,
        failing_release: bool,
        failing_clip_after_release: bool,
    }

    #[derive(Clone)]
    struct Fake(Arc<Mutex<Desktop>>);

    impl Fake {
        fn new() -> Self {
            let windows = [
                (GAME, 22, 220),
                (WHEEL, 11, 110),
                (PICKER, 11, 110),
                (OTHER, 33, 330),
            ]
            .into_iter()
            .map(|(handle, process, thread)| {
                (
                    handle,
                    Window {
                        handle,
                        process,
                        thread,
                        client: GAME_CLIP,
                    },
                )
            })
            .collect();
            Self(Arc::new(Mutex::new(Desktop {
                foreground: GAME,
                thread: 110,
                windows,
                desktop: DESKTOP,
                clip: GAME_CLIP,
                actions: Vec::new(),
                clip_on_game_focus: None,
                failing_activation: None,
                failing_release: false,
                failing_clip_after_release: false,
            })))
        }

        fn state(&self) -> std::sync::MutexGuard<'_, Desktop> {
            self.0.lock().unwrap()
        }

        fn control(&self) -> Control<Self> {
            Control::new(self.clone())
        }
    }

    impl Backend for Fake {
        fn thread(&self) -> u32 {
            self.state().thread
        }
        fn process(&self) -> u32 {
            11
        }
        fn foreground(&self) -> u64 {
            self.state().foreground
        }
        fn window(&self, handle: u64) -> Option<Window> {
            self.state().windows.get(&handle).copied()
        }
        fn desktop(&self) -> Result<Bounds> {
            Ok(self.state().desktop)
        }
        fn clip(&self) -> Result<Bounds> {
            let mut state = self.state();
            if state.failing_clip_after_release && state.actions.last() == Some(&Action::Release) {
                state.failing_clip_after_release = false;
                return Err(PlatformError::Failed("clip query failed".into()));
            }
            Ok(state.clip)
        }
        fn activate(&self, handle: u64) -> Result<()> {
            let mut state = self.state();
            state.actions.push(Action::Activate(handle));
            if state.failing_activation == Some((handle, false)) {
                return Err(PlatformError::Failed("activation failed".into()));
            }
            state.foreground = handle;
            if handle == GAME
                && let Some(clip) = state.clip_on_game_focus
            {
                state.clip = clip;
            }
            if state.failing_activation == Some((handle, true)) {
                return Err(PlatformError::Failed("activation partly failed".into()));
            }
            Ok(())
        }
        fn release_clip(&self) -> Result<()> {
            let mut state = self.state();
            state.actions.push(Action::Release);
            if state.failing_release {
                return Err(PlatformError::Failed("release failed".into()));
            }
            state.clip = state.desktop;
            Ok(())
        }
        fn confine(&self, bounds: Bounds) -> Result<()> {
            let mut state = self.state();
            state.actions.push(Action::Confine(bounds));
            state.clip = bounds;
            Ok(())
        }
    }

    fn open(fake: &Fake, owner: u64) -> InputLease {
        let control = fake.control();
        control.acquire(control.snapshot().unwrap(), owner).unwrap()
    }

    #[test]
    fn opening_controls_releases_a_game_clip_and_closing_returns_the_original_state() {
        let fake = Fake::new();
        let lease = open(&fake, WHEEL);
        assert_eq!(lease.source(), Some(GAME));
        assert_eq!(lease.owner(), WHEEL);
        assert!(fake.control().still_owns(&lease));
        assert_eq!(fake.state().clip, DESKTOP);
        fake.control().finish(lease).unwrap();
        let state = fake.state();
        assert_eq!(state.foreground, GAME);
        assert_eq!(state.clip, GAME_CLIP);
        assert_eq!(
            state.actions,
            [
                Action::Activate(WHEEL),
                Action::Release,
                Action::Activate(GAME),
                Action::Confine(GAME_CLIP),
            ]
        );
    }

    #[test]
    fn an_unconfined_game_is_not_confined_to_the_old_desktop_when_controls_close() {
        let fake = Fake::new();
        fake.state().clip = DESKTOP;
        let lease = open(&fake, WHEEL);
        fake.control().finish(lease).unwrap();
        assert_eq!(fake.state().foreground, GAME);
        assert!(
            !fake
                .state()
                .actions
                .iter()
                .any(|a| matches!(a, Action::Confine(_)))
        );
    }

    #[test]
    fn an_unrelated_foreground_window_is_never_replaced_when_controls_close() {
        let fake = Fake::new();
        let lease = open(&fake, WHEEL);
        fake.state().foreground = OTHER;
        fake.control().finish(lease).unwrap();
        assert_eq!(fake.state().foreground, OTHER);
        assert_eq!(fake.state().actions.len(), 2);
    }

    #[test]
    fn an_observed_switch_away_prevents_return_even_if_controls_are_focused_again() {
        let fake = Fake::new();
        let mut lease = open(&fake, WHEEL);
        lease.focus_lost();
        fake.control().finish(lease).unwrap();
        assert_eq!(fake.state().foreground, WHEEL);
        assert_eq!(fake.state().actions.len(), 2);
    }

    #[test]
    fn a_game_that_replaces_its_clip_on_focus_keeps_that_new_clip() {
        let fake = Fake::new();
        let lease = open(&fake, WHEEL);
        let new_clip = Bounds {
            right: 1280,
            bottom: 720,
            ..GAME_CLIP
        };
        fake.state().clip_on_game_focus = Some(new_clip);
        fake.control().finish(lease).unwrap();
        assert_eq!(fake.state().foreground, GAME);
        assert_eq!(fake.state().clip, new_clip);
        assert_eq!(fake.state().actions.len(), 3);
    }

    #[test]
    fn a_clip_changed_during_the_interaction_is_not_overwritten_on_close() {
        let fake = Fake::new();
        let lease = open(&fake, WHEEL);
        let external_clip = Bounds {
            right: 1200,
            ..GAME_CLIP
        };
        fake.state().clip = external_clip;
        fake.control().finish(lease).unwrap();
        assert_eq!(fake.state().clip, external_clip);
        assert_eq!(fake.state().actions.len(), 3);
    }

    #[test]
    fn a_closed_or_reused_game_handle_is_not_activated_on_close() {
        for reuse in [false, true] {
            let fake = Fake::new();
            let lease = open(&fake, WHEEL);
            if reuse {
                fake.state().windows.get_mut(&GAME).unwrap().process = 999;
            } else {
                fake.state().windows.remove(&GAME);
            }
            fake.control().finish(lease).unwrap();
            assert_eq!(fake.state().actions.len(), 2);
        }
    }

    #[test]
    fn changed_game_geometry_or_desktop_layout_prevents_restoring_an_old_clip() {
        for desktop in [false, true] {
            let fake = Fake::new();
            let lease = open(&fake, WHEEL);
            if desktop {
                fake.state().desktop.right = 2560;
            } else {
                fake.state().windows.get_mut(&GAME).unwrap().client.right = 1280;
            }
            fake.control().finish(lease).unwrap();
            assert_eq!(fake.state().actions.len(), 2);
        }
    }

    #[test]
    fn transferring_the_wheel_to_the_picker_does_not_focus_the_game_between_them() {
        let fake = Fake::new();
        let mut lease = open(&fake, WHEEL);
        fake.control().transfer(&mut lease, PICKER).unwrap();
        assert_eq!(lease.source(), Some(GAME));
        assert_eq!(lease.owner(), PICKER);
        assert_eq!(fake.state().foreground, PICKER);
        assert_eq!(
            fake.state().actions,
            [
                Action::Activate(WHEEL),
                Action::Release,
                Action::Activate(PICKER),
                Action::Release
            ]
        );
        fake.control().finish(lease).unwrap();
        assert_eq!(fake.state().foreground, GAME);
        assert_eq!(fake.state().clip, GAME_CLIP);
    }

    #[test]
    fn a_transfer_after_an_observed_external_switch_is_refused() {
        let fake = Fake::new();
        let mut lease = open(&fake, WHEEL);
        lease.focus_lost();
        assert!(fake.control().transfer(&mut lease, PICKER).is_err());
        assert_eq!(fake.state().actions.len(), 2);
    }

    #[test]
    fn a_window_switch_between_snapshot_and_activation_is_refused_without_mutations() {
        let fake = Fake::new();
        let snapshot = fake.control().snapshot().unwrap();
        fake.state().foreground = OTHER;
        assert!(fake.control().acquire(snapshot, WHEEL).is_err());
        assert!(fake.state().actions.is_empty());
    }

    #[test]
    fn a_game_clip_changed_while_the_controls_load_is_refreshed_before_activation() {
        let fake = Fake::new();
        let snapshot = fake.control().snapshot().unwrap();
        let new_clip = Bounds {
            right: 1280,
            bottom: 720,
            ..GAME_CLIP
        };
        fake.state().clip = new_clip;
        let lease = fake.control().acquire(snapshot, WHEEL).unwrap();
        fake.control().finish(lease).unwrap();
        assert_eq!(fake.state().clip, new_clip);
    }

    #[test]
    fn a_reused_source_handle_before_activation_is_refused_without_mutations() {
        let fake = Fake::new();
        let snapshot = fake.control().snapshot().unwrap();
        fake.state().windows.get_mut(&GAME).unwrap().process = 999;
        assert!(fake.control().acquire(snapshot, WHEEL).is_err());
        assert!(fake.state().actions.is_empty());
    }

    #[test]
    fn a_foreign_or_missing_owner_is_refused_without_activation() {
        for owner in [OTHER, 12345] {
            let fake = Fake::new();
            assert!(
                fake.control()
                    .acquire(fake.control().snapshot().unwrap(), owner)
                    .is_err()
            );
            assert!(fake.state().actions.is_empty());
        }
    }

    #[test]
    fn a_lease_is_only_opened_and_closed_on_its_original_ui_thread() {
        let fake = Fake::new();
        let snapshot = fake.control().snapshot().unwrap();
        fake.state().thread = 111;
        assert!(fake.control().acquire(snapshot, WHEEL).is_err());
        assert!(fake.state().actions.is_empty());

        fake.state().thread = 110;
        let lease = open(&fake, WHEEL);
        fake.state().thread = 111;
        assert!(fake.control().finish(lease).is_err());
        assert_eq!(fake.state().actions.len(), 2);
    }

    #[test]
    fn a_failed_activation_cannot_release_another_windows_pointer_clip() {
        let fake = Fake::new();
        fake.state().failing_activation = Some((WHEEL, false));
        assert!(
            fake.control()
                .acquire(fake.control().snapshot().unwrap(), WHEEL)
                .is_err()
        );
        assert_eq!(fake.state().foreground, GAME);
        assert_eq!(fake.state().clip, GAME_CLIP);
        assert_eq!(fake.state().actions, [Action::Activate(WHEEL)]);
    }

    #[test]
    fn partial_activation_and_release_failures_return_to_the_game_when_still_owned() {
        for activation in [false, true] {
            let fake = Fake::new();
            if activation {
                fake.state().failing_activation = Some((WHEEL, true));
            } else {
                fake.state().failing_release = true;
            }
            assert!(
                fake.control()
                    .acquire(fake.control().snapshot().unwrap(), WHEEL)
                    .is_err()
            );
            assert_eq!(fake.state().foreground, GAME);
            assert_eq!(fake.state().clip, GAME_CLIP);
        }
    }

    #[test]
    fn a_failed_clip_query_after_release_restores_the_previous_game_and_clip() {
        let fake = Fake::new();
        fake.state().failing_clip_after_release = true;
        assert!(
            fake.control()
                .acquire(fake.control().snapshot().unwrap(), WHEEL)
                .is_err()
        );
        assert_eq!(fake.state().foreground, GAME);
        assert_eq!(fake.state().clip, GAME_CLIP);
    }

    #[test]
    fn a_partly_failed_transfer_retains_the_current_owner_for_safe_cleanup() {
        let fake = Fake::new();
        let mut lease = open(&fake, WHEEL);
        fake.state().failing_activation = Some((PICKER, true));
        assert!(fake.control().transfer(&mut lease, PICKER).is_err());
        assert_eq!(lease.owner(), PICKER);
        fake.control().finish(lease).unwrap();
        assert_eq!(fake.state().foreground, GAME);
        assert_eq!(fake.state().clip, GAME_CLIP);
    }

    #[test]
    fn a_failed_transfer_retains_the_previous_owner_when_it_still_has_focus() {
        let fake = Fake::new();
        let mut lease = open(&fake, WHEEL);
        fake.state().failing_activation = Some((PICKER, false));
        assert!(fake.control().transfer(&mut lease, PICKER).is_err());
        assert_eq!(lease.owner(), WHEEL);
        fake.control().finish(lease).unwrap();
        assert_eq!(fake.state().foreground, GAME);
    }

    #[test]
    fn dropping_a_lease_does_not_change_focus_or_pointer_confinement() {
        let fake = Fake::new();
        {
            let _lease = open(&fake, WHEEL);
        }
        assert_eq!(fake.state().foreground, WHEEL);
        assert_eq!(fake.state().clip, DESKTOP);
        assert_eq!(fake.state().actions.len(), 2);
    }
}
