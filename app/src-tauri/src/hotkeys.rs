use std::sync::Arc;

use tauri::AppHandle;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::state::AppState;
use crate::{flow, windows};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Translate,
    Region,
    Hover,
    Continuous,
    ContinuousArea,
    History,
    Dismiss,
}

/// Register every configured hotkey. Returns human-readable problems (bad syntax, already taken).
pub fn register(app: &AppHandle, state: &Arc<AppState>) -> Vec<String> {
    let keys = state.engine().config().hotkeys.clone();
    let table = [
        (keys.translate, Action::Translate),
        (keys.region, Action::Region),
        (keys.hover, Action::Hover),
        (keys.continuous, Action::Continuous),
        (keys.continuous_area, Action::ContinuousArea),
        (keys.history, Action::History),
        (keys.dismiss, Action::Dismiss),
    ];
    let mut problems = Vec::new();
    for (text, action) in table {
        if text.trim().is_empty() {
            continue;
        }
        let shortcut: Shortcut = match text.parse() {
            Ok(s) => s,
            Err(e) => {
                problems.push(format!("hotkey {text:?} is not valid: {e}"));
                continue;
            }
        };
        let state = state.clone();
        let result = app.global_shortcut().on_shortcut(shortcut, move |app, _, event| {
            // Fire on press only, not again on release.
            if event.state != ShortcutState::Pressed {
                return;
            }
            let (app, state) = (app.clone(), state.clone());
            match action {
                Action::Translate => flow::translate_selection(app, state),
                Action::Region => {
                    let _ = windows::open_picker(&app);
                }
                Action::Hover => flow::toggle_hover(&app, &state),
                Action::Continuous => flow::toggle_continuous(&app, &state),
                Action::ContinuousArea => flow::choose_continuous_area(&app, &state),
                Action::History => {
                    let _ = windows::open_hub(&app, "history");
                }
                Action::Dismiss => windows::hide_panel(&app, &state),
            }
        });
        if let Err(e) = result {
            problems.push(format!("could not register {text}: {e} (another program may be using it; change it in config.toml)"));
        }
    }
    problems
}
