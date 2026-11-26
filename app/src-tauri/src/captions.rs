//! YouTube captions in the shell: make the code the extension needs, start the engine's listener at launch when
//! `[captions]` is switched on, hand every line to the engine, and tell the Status page what the listener is doing.
//! The protocol and every rule about a request are `maru_engine::captions`; nothing is decided here.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use maru_core::Event;
use maru_core::settings;
use maru_engine::captions::{Counters, new_token};
use serde::Serialize;
use serde_json::{Map, Value};
use tauri::{AppHandle, Manager, State};

use crate::state::AppState;

enum Listener {
    Off,
    /// Switched on, but not listening, and why.
    Failed(String),
    On { port: u16, counters: Arc<Counters> },
}

pub struct CaptionsState {
    listener: Mutex<Listener>,
    /// A caption that could not be translated has already been reported this session.
    error_shown: AtomicBool,
}

/// What the Status page shows (`app/ui/captions-core.js`). The code is not part of it: Settings shows that.
#[derive(Serialize)]
pub struct CaptionsStatus {
    enabled: bool,
    listening: bool,
    port: u16,
    problem: Option<String>,
    accepted: u64,
    dropped: u64,
    refused: u64,
}

#[tauri::command]
pub fn captions_status(state: State<'_, CaptionsState>) -> CaptionsStatus {
    let off = CaptionsStatus { enabled: false, listening: false, port: 0, problem: None, accepted: 0, dropped: 0, refused: 0 };
    match &*state.listener.lock().unwrap_or_else(|p| p.into_inner()) {
        Listener::Off => off,
        Listener::Failed(why) => CaptionsStatus { enabled: true, problem: Some(why.clone()), ..off },
        Listener::On { port, counters } => {
            let counts = counters.counts();
            CaptionsStatus { enabled: true, listening: true, port: *port, accepted: counts.accepted, dropped: counts.dropped, refused: counts.refused, ..off }
        }
    }
}

/// Captions switched on without a code: add a new one to what is about to be saved, so the settings page can show
/// it to copy as soon as the switch is saved. `text` is the file now, `changes` what the save will apply to it.
pub fn add_code_when_switched_on(text: &str, changes: &mut Map<String, Value>) {
    // A save that is not valid is reported by the save itself; nothing is added to it.
    let Ok(after) = settings::patch(text, changes) else { return };
    if after.config.captions.enabled && after.config.captions.token().is_none() {
        changes.insert("captions.token".into(), Value::String(new_token()));
    }
}

/// At launch, before the settings are read: a file that switches captions on by hand gets its code written too.
/// Comments and everything else in the file are kept. If the file cannot be written the listener stays off and
/// the Status page says why.
pub fn write_code_if_missing(path: &Path) {
    let Ok(text) = std::fs::read_to_string(path) else { return }; // no file: the defaults, with captions off
    let mut changes = Map::new();
    add_code_when_switched_on(&text, &mut changes);
    if changes.is_empty() {
        return;
    }
    let written = settings::patch(&text, &changes).map_err(|e| e.to_string()).and_then(|patched| {
        // Beside the file, then renamed over it: a crash never leaves half a config.
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, &patched.text).and_then(|()| std::fs::rename(&tmp, path)).map_err(|e| e.to_string())
    });
    match written {
        Ok(()) => tracing::info!("YouTube captions: wrote a new code for the extension to config.toml (Settings shows it)"),
        Err(e) => tracing::warn!("YouTube captions: the code for the extension could not be saved to config.toml ({e})"),
    }
}

/// Start listening when `[captions]` says so, and pass every line on to the engine for as long as Maru runs.
pub fn start(app: &AppHandle, state: Arc<AppState>) {
    let cfg = state.engine().config().captions.clone();
    let listener = if !cfg.enabled {
        Listener::Off
    } else {
        match tauri::async_runtime::block_on(maru_engine::captions::listen(&cfg)) {
            Ok(mut server) => {
                let on = Listener::On { port: server.addr().port(), counters: server.counters() };
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    while let Some(line) = server.next().await {
                        let (id, events) = state.engine().translate_caption(line);
                        follow(app.clone(), state.clone(), id, events);
                    }
                });
                on
            }
            Err(why) => {
                tracing::warn!("{why}");
                Listener::Failed(why)
            }
        }
    };
    app.manage(CaptionsState { listener: Mutex::new(listener), error_shown: AtomicBool::new(false) });
}

/// See one caption line through: the engine lists it in the history; here it becomes the line a star is meant
/// for, and the first failure of a session is reported like a failed live subtitle.
fn follow(app: AppHandle, state: Arc<AppState>, id: u64, mut events: maru_engine::EventRx) {
    tauri::async_runtime::spawn(async move {
        while let Some(event) = events.recv().await {
            crate::resources::event(&app, &event, "captions");
            match event {
                // No band draws a caption line, so "the line on screen" for the star shortcut is the newest one translated.
                Event::TranslationDone { .. } => state.live_shown.store(id, Ordering::Relaxed),
                Event::Error { message, .. } => {
                    tracing::warn!("caption translation: {message}");
                    let first = app.try_state::<CaptionsState>().is_some_and(|s| !s.error_shown.swap(true, Ordering::Relaxed));
                    if first {
                        crate::flow::notice(&app, &state, None, "error", format!("A YouTube caption could not be translated: {message}"));
                    }
                }
                Event::Done { .. } => break,
                _ => {}
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use maru_core::Config;
    use serde_json::json;

    fn changes(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap_or_default()
    }

    #[test]
    fn switching_captions_on_adds_a_code_once_and_never_replaces_one() {
        let mut save = changes(json!({ "captions.enabled": true }));
        add_code_when_switched_on("# my notes\n[ui]\nfont_size_px = 20\n", &mut save);
        let code = save["captions.token"].as_str().expect("a code is saved with the switch").to_string();
        assert_eq!(code.len(), 32);

        // The file already has one: the switch alone is saved.
        let file = format!("[captions]\nenabled = false\ntoken = \"{code}\"\n");
        let mut again = changes(json!({ "captions.enabled": true }));
        add_code_when_switched_on(&file, &mut again);
        assert_eq!(again.len(), 1);

        // Emptying the code while captions stay on asks for a new one.
        let on = format!("[captions]\nenabled = true\ntoken = \"{code}\"\n");
        let mut cleared = changes(json!({ "captions.token": null }));
        add_code_when_switched_on(&on, &mut cleared);
        let fresh = cleared["captions.token"].as_str().unwrap();
        assert!(fresh.len() == 32 && fresh != code);

        // Off, or a save about something else, or a save that is not valid: nothing is added.
        for (text, save) in [("", json!({ "captions.port": 19000 })), ("", json!({ "ui.font_size_px": 20 })), ("", json!({ "captions.enabled": true, "captions.port": 80 }))] {
            let mut save = changes(save);
            let before = save.clone();
            add_code_when_switched_on(text, &mut save);
            assert_eq!(save, before);
        }
    }

    #[test]
    fn a_file_that_switches_captions_on_by_hand_gets_its_code_and_keeps_its_comments() {
        let dir = std::env::temp_dir().join(format!("maru-captions-code-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "# my notes\n[captions]\nenabled = true   # from the browser\n").unwrap();
        write_code_if_missing(&path);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# my notes") && text.contains("# from the browser"), "{text}");
        let cfg = Config::load(&path).unwrap();
        let code = cfg.captions.token().expect("a code was written").to_string();
        // The next launch leaves the file alone.
        write_code_if_missing(&path);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        assert_eq!(Config::load(&path).unwrap().captions.token(), Some(code.as_str()));

        // Captions off, or no file at all: nothing is written.
        let off = dir.join("off.toml");
        std::fs::write(&off, "[captions]\nenabled = false\n").unwrap();
        write_code_if_missing(&off);
        assert_eq!(std::fs::read_to_string(&off).unwrap(), "[captions]\nenabled = false\n");
        write_code_if_missing(&dir.join("missing.toml"));
        assert!(!dir.join("missing.toml").exists());
        std::fs::remove_dir_all(dir).ok();
    }
}
