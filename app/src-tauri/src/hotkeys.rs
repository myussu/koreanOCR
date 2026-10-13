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
    Scan,
    Speak,
    History,
    Dismiss,
    Launcher,
}

/// One action may have several keys: `"Ctrl+Alt+T | F6"` (a comma followed by a space also works;
/// a bare `,` is the comma *key*, as in `Ctrl+,`). Empty parts are ignored.
pub fn alternatives(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let (mut parts, mut current) = (Vec::new(), String::new());
    for (i, &c) in chars.iter().enumerate() {
        let separator = c == '|' || (c == ',' && chars.get(i + 1).is_some_and(|n| n.is_whitespace()));
        if separator {
            parts.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
    }
    parts.push(current);
    parts.into_iter().map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect()
}

/// Register every configured hotkey. Returns human-readable problems (bad syntax, already taken).
pub fn register(app: &AppHandle, state: &Arc<AppState>) -> Vec<String> {
    // Saving new shortcuts registers them again from scratch.
    let _ = app.global_shortcut().unregister_all();
    let keys = state.cfg().hotkeys;
    let table = [
        (keys.translate, Action::Translate),
        (keys.region, Action::Region),
        (keys.hover, Action::Hover),
        (keys.continuous, Action::Continuous),
        (keys.continuous_area, Action::ContinuousArea),
        (keys.scan, Action::Scan),
        (keys.speak, Action::Speak),
        (keys.history, Action::History),
        (keys.dismiss, Action::Dismiss),
        (keys.launcher, Action::Launcher),
    ];
    let mut problems = Vec::new();
    for (text, action) in table.into_iter().flat_map(|(text, action)| alternatives(&text).into_iter().map(move |t| (t, action))) {
        let shortcut: Shortcut = match text.parse() {
            Ok(s) => s,
            Err(e) => {
                problems.push(format!("hotkey {text:?} is not valid: {e} (separate alternatives with | )"));
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
            // A shortcut can open the hub or picker; WebView2 needs the event thread free.
            tauri::async_runtime::spawn(async move { match action {
                Action::Translate => flow::translate_selection(app, state),
                Action::Region => {
                    let _ = windows::open_picker(&app);
                }
                Action::Hover => flow::toggle_hover(&app, &state),
                Action::Continuous => flow::toggle_continuous(&app, &state),
                Action::ContinuousArea => flow::choose_continuous_area(&app, &state),
                Action::Scan => flow::scan_screen(app, state),
                Action::Speak => flow::speak_selection(app, state),
                Action::History => {
                    let _ = windows::open_hub(&app, "history");
                }
                Action::Dismiss => flow::dismiss_all(&app, &state),
                Action::Launcher => crate::launcher::toggle(&app),
            }});
        });
        if let Err(e) = result {
            problems.push(format!("could not register {text}: {e} (another program may be using it; change it in config.toml)"));
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn several_keys_per_action() {
        assert_eq!(alternatives("Ctrl+Alt+T"), ["Ctrl+Alt+T"]);
        assert_eq!(alternatives("Ctrl+Alt+T | F6"), ["Ctrl+Alt+T", "F6"]);
        assert_eq!(alternatives("Ctrl+Alt+T, F6 |Pause"), ["Ctrl+Alt+T", "F6", "Pause"]);
        assert_eq!(alternatives("Ctrl+,"), ["Ctrl+,"], "the comma key itself is not a separator");
        assert_eq!(alternatives(" | "), Vec::<String>::new());
        assert_eq!(alternatives(""), Vec::<String>::new());
    }

    #[test]
    fn the_keys_we_suggest_for_games_all_parse() {
        for key in ["F6", "F7", "F8", "Pause", "ScrollLock", "Numpad5", "Insert", "Ctrl+Alt+T", "Ctrl+Alt+Shift+C", "Ctrl+Alt+Escape"] {
            assert!(key.parse::<Shortcut>().is_ok(), "{key} should be a valid hotkey");
        }
    }
}
