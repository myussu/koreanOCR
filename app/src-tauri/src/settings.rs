//! The settings page's backend: show `config.toml`, change it safely, say what needs a restart.

use std::str::FromStr;
use std::sync::Arc;

use maru_core::Config;
use maru_core::settings::{self, applies_live};
use serde::Serialize;
use serde_json::{Map, Value};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_global_shortcut::Shortcut;

use crate::hotkeys::alternatives;
use crate::state::AppState;
use crate::{flow, hotkeys, windows};

type S<'a> = State<'a, Arc<AppState>>;
type Res<T> = Result<T, String>;

#[derive(Serialize)]
pub struct SettingsView {
    /// What the file says now.
    values: Value,
    defaults: Value,
    path: String,
    /// Settings saved since Maru started that only take effect after a restart.
    pending_restart: Vec<String>,
    /// The lite profile is on because of the environment variable, whatever the file says.
    lite_from_env: bool,
    live_models: Vec<maru_engine::live_models::ModelFile>,
}

fn read_text(state: &AppState) -> Res<String> {
    match std::fs::read_to_string(&state.services.paths.config_file) {
        Ok(t) => Ok(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e.to_string()),
    }
}

fn pending_restart(state: &AppState, now: &Config) -> Vec<String> {
    settings::changed_paths(&state.booted, now).into_iter().filter(|p| !applies_live(p)).collect()
}

#[tauri::command]
pub fn get_settings(state: S) -> Res<SettingsView> {
    let path = state.services.paths.config_file.clone();
    let now = Config::load(&path).map_err(|e| e.to_string())?;
    Ok(SettingsView {
        values: settings::snapshot(&now),
        defaults: settings::snapshot(&Config::default()),
        pending_restart: pending_restart(&state, &now),
        lite_from_env: std::env::var("MARU_LITE").is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")),
        path: path.display().to_string(),
        live_models: maru_engine::live_models::available(&state.services.paths),
    })
}

#[derive(Serialize)]
pub struct SaveResult {
    /// Settings whose value changed in this save.
    changed: Vec<String>,
    /// Everything saved since start-up that still waits for a restart (this save's included).
    restart: Vec<String>,
    /// Problems with shortcuts that were saved anyway (taken by another program, ...).
    warnings: Vec<String>,
}

/// Shortcuts must parse and must not be given to two actions.
fn check_hotkeys(cfg: &Config) -> Res<()> {
    let k = &cfg.hotkeys;
    let table = [
        ("translate", &k.translate),
        ("region", &k.region),
        ("hover", &k.hover),
        ("continuous", &k.continuous),
        ("continuous_area", &k.continuous_area),
        ("scan", &k.scan),
        ("speak", &k.speak),
        ("history", &k.history),
        ("dismiss", &k.dismiss),
        ("launcher", &k.launcher),
        ("reading_again", &k.reading_again),
        ("wheel", &k.wheel),
        ("star", &k.star),
    ];
    let mut seen: Vec<(Shortcut, &str)> = Vec::new();
    for (action, text) in table {
        for alt in alternatives(text) {
            let parsed = Shortcut::from_str(&alt).map_err(|e| format!("{action}: {alt:?} is not a valid shortcut ({e})"))?;
            if let Some((_, other)) = seen.iter().find(|(s, _)| *s == parsed) {
                return Err(format!("{alt} is used for both {other} and {action}"));
            }
            seen.push((parsed, action));
        }
    }
    Ok(())
}

/// The backup must be a local live model whose file is set and readable.
fn check_backup(live: &maru_core::live_model::LiveTranslationConfig, backup: maru_core::live_model::LiveModel) -> Res<()> {
    if backup == maru_core::live_model::LiveModel::Main {
        return Err("The backup translator must be a local live model, not the main model.".into());
    }
    let path = live.path_for(backup).ok_or("Set the backup translator's GGUF file first: pick its card once in Live subtitles.")?;
    maru_engine::live_models::validate_file(path)
}

fn check_url(what: &str, url: &str) -> Res<()> {
    if url.starts_with("http://") || url.starts_with("https://") {
        return Ok(());
    }
    Err(format!("The {what} address must start with http:// or https://. For an IP or hostname, press Detect server to find its port."))
}

#[tauri::command]
pub fn save_settings(app: AppHandle, state: S, changes: Map<String, Value>) -> Res<SaveResult> {
    let path = state.services.paths.config_file.clone();
    let patched = settings::patch(&read_text(&state)?, &changes).map_err(|e| e.to_string())?;
    let new = patched.config.clone();
    check_hotkeys(&new)?;
    // A bare IP or hostname is only a starting point for Detect server; saved as is, every request would fail.
    if patched.changed.iter().any(|p| p == "llm.endpoint") && let Some(endpoint) = &new.llm.endpoint {
        check_url("language model server", endpoint)?;
    }
    if patched.changed.iter().any(|p| p.starts_with("continuous.translation.")) {
        if let Some(path) = new.continuous.translation.selected_path() {
            maru_engine::live_models::validate_file(path)?;
        }
        if let Some(backup) = new.continuous.translation.backup_model {
            check_backup(&new.continuous.translation, backup)?;
        }
        if let Some(endpoint) = &new.continuous.translation.snapshot_endpoint {
            check_url("snapshot server", endpoint)?;
            if new.continuous.translation.snapshot_model_name.trim().is_empty() {
                return Err("Enter the model name your snapshot server expects.".into());
            }
        }
    }

    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    // Write beside the file and rename, so a crash never leaves half a config.
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, &patched.text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    tracing::info!("settings saved: {}", patched.changed.join(", "));

    let live_changed = patched.changed.iter().any(|p| applies_live(p));
    let mut warnings = Vec::new();
    if live_changed {
        state.set_cfg(new.clone());
        if patched.changed.iter().any(|p| p.starts_with("hotkeys.")) {
            warnings = hotkeys::register(&app, &state);
            if let Ok(menu) = crate::tray_menu(&app, &state)
                && let Some(tray) = app.tray_by_id("maru")
            {
                let _ = tray.set_menu(Some(menu));
            }
            flow::refresh_tray(&app, &state);
        }
        let _ = app.emit_to(windows::PANEL, "maru-config", ());
        if patched.changed.iter().any(|p| p == "ui.launcher") {
            crate::launcher::show(&app, new.ui.launcher);
        }
    }
    Ok(SaveResult { changed: patched.changed, restart: pending_restart(&state, &new), warnings })
}

/// Start Maru again so the settings that are read at start-up take effect.
#[tauri::command]
pub fn restart_app(app: AppHandle) {
    tracing::info!("restarting to apply settings");
    app.restart();
}

/// Show `config.toml` in the file explorer for people who prefer editing it by hand.
#[tauri::command]
pub fn open_config_file(state: S) -> Res<()> {
    let path = state.services.paths.config_file.clone();
    if !path.exists() {
        Config::default().save(&path).map_err(|e| e.to_string())?;
    }
    #[cfg(windows)]
    let result = std::process::Command::new("explorer").arg(format!("/select,{}", path.display())).spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg("-R").arg(&path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open").arg(path.parent().unwrap_or(&path)).spawn();
    result.map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(f: impl FnOnce(&mut Config)) -> Config {
        let mut c = Config::default();
        f(&mut c);
        c
    }

    #[test]
    fn default_shortcuts_are_valid_and_distinct() {
        check_hotkeys(&Config::default()).unwrap();
    }

    /// Every name the page's key recorder can produce (settings-core.js `keyName`) must register.
    #[test]
    fn every_key_the_recorder_can_name_is_a_valid_shortcut() {
        let names = [
            "Ctrl+Alt+T", "Shift+5", "F1", "F24", "Numpad5", "NumpadEnter", "NumpadAdd", "NumpadSubtract", "NumpadMultiply", "NumpadDivide", "NumpadDecimal",
            "Ctrl+Escape", "Ctrl+Space", "Ctrl+Enter", "Ctrl+Tab", "Ctrl+Backspace", "Ctrl+Delete", "Insert", "Ctrl+Home", "Ctrl+End", "Ctrl+PageUp", "Ctrl+PageDown",
            "Ctrl+Up", "Ctrl+Down", "Ctrl+Left", "Ctrl+Right", "Pause", "ScrollLock", "PrintScreen", "CapsLock", "NumLock", "Super+A",
            "Ctrl+`", "Ctrl+-", "Ctrl+=", "Ctrl+[", "Ctrl+]", "Ctrl+\\", "Ctrl+;", "Ctrl+'", "Ctrl+,", "Ctrl+.", "Ctrl+/",
        ];
        let bad: Vec<_> = names.iter().filter(|n| Shortcut::from_str(n).is_err()).collect();
        assert!(bad.is_empty(), "the recorder would offer shortcuts that cannot be registered: {bad:?}");
    }

    #[test]
    fn bad_and_duplicate_shortcuts_are_refused_with_a_reason() {
        let bad = check_hotkeys(&with(|c| c.hotkeys.scan = "Ctrl+Alt+Banana".into())).unwrap_err();
        assert!(bad.contains("scan") && bad.contains("Banana"), "{bad}");
        let dup = check_hotkeys(&with(|c| c.hotkeys.hover = "Ctrl+Alt+T".into())).unwrap_err();
        assert!(dup.contains("hover") && dup.contains("translate"), "{dup}");
        check_hotkeys(&with(|c| c.hotkeys.scan = "Ctrl+Alt+S | F7".into())).unwrap();
        assert!(check_hotkeys(&with(|c| c.hotkeys.scan = "F7 | F7".into())).is_err(), "twice in one action is also a clash");
    }

    #[test]
    fn a_backup_translator_must_be_a_local_model_with_its_file_set() {
        use maru_core::live_model::{LiveModel, LiveTranslationConfig};
        let mut live = LiveTranslationConfig::default();
        assert!(check_backup(&live, LiveModel::Main).is_err());
        let err = check_backup(&live, LiveModel::HyMt2Small).unwrap_err();
        assert!(err.contains("GGUF"), "{err}");
        let file = std::env::temp_dir().join(format!("maru-backup-{}.gguf", std::process::id()));
        std::fs::write(&file, b"GGUFtest").unwrap();
        live.hy_mt2_small_path = Some(file.clone());
        check_backup(&live, LiveModel::HyMt2Small).unwrap();
        let _ = std::fs::remove_file(file);
    }

    #[test]
    fn a_server_address_without_http_is_refused_with_a_pointer_to_detection() {
        let err = check_url("language model server", "192.168.18.217").unwrap_err();
        assert!(err.contains("http://") && err.contains("Detect server"), "{err}");
        assert!(check_url("language model server", "localhost:11434").is_err());
        check_url("language model server", "http://192.168.18.217:11434").unwrap();
        check_url("snapshot server", "https://example.org/v1").unwrap();
    }

    #[test]
    fn the_wheel_and_read_again_shortcuts_are_checked_like_the_others() {
        let dup = check_hotkeys(&with(|c| c.hotkeys.wheel = "Ctrl+Alt+T".into())).unwrap_err();
        assert!(dup.contains("wheel") && dup.contains("translate"), "{dup}");
        let dup = check_hotkeys(&with(|c| c.hotkeys.reading_again = "Ctrl+Alt+Q".into())).unwrap_err();
        assert!(dup.contains("reading_again") && dup.contains("wheel"), "{dup}");
        let bad = check_hotkeys(&with(|c| c.hotkeys.reading_again = "Ctrl+Alt+Banana".into())).unwrap_err();
        assert!(bad.contains("reading_again"), "{bad}");
        // Empty means the shortcut is off, which is the default for reading again.
        check_hotkeys(&with(|c| c.hotkeys.wheel = String::new())).unwrap();
    }
}
