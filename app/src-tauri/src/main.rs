//! Maru overlay: a thin Tauri shell around `maru-engine`.
//!
//! All language, OCR and learning logic lives in the engine crates; this file wires
//! it to windows, hotkeys and the tray.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod flow;
mod hotkeys;
mod logic;
mod state;
mod watch;
mod windows;

use std::sync::{Arc, Mutex};

use maru_core::{Config, Paths};
use maru_engine::{BootOptions, boot};
use state::AppState;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Manager, RunEvent};

/// Log to `maru-app.log` in the data folder (the release build has no console), and record
/// panics there. A panic inside a native callback aborts the process with no message anywhere
/// else, which made the first crash impossible to diagnose.
fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    let paths = Paths::discover();
    let _ = std::fs::create_dir_all(&paths.data_dir);
    let log = paths.data_dir.join("maru-app.log");
    let _ = std::fs::rename(&log, paths.data_dir.join("maru-app.previous.log"));
    match std::fs::File::create(&log) {
        Ok(file) => tracing_subscriber::fmt().with_env_filter(filter).with_ansi(false).with_writer(Mutex::new(file)).init(),
        Err(_) => tracing_subscriber::fmt().with_env_filter(filter).init(),
    }
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("PANIC: {info}\n{}", std::backtrace::Backtrace::force_capture());
        default_hook(info);
    }));
    tracing::info!("log file: {}", log.display());
}

fn build_tray(app: &tauri::AppHandle, state: &Arc<AppState>) -> tauri::Result<()> {
    let keys = state.engine().config().hotkeys.clone();
    let item = |id: &str, label: String| MenuItem::with_id(app, id, label, true, None::<&str>);
    let menu = Menu::with_items(
        app,
        &[
            &item("translate", format!("Translate selection\t{}", keys.translate))?,
            &item("region", format!("Pick a region…\t{}", keys.region))?,
            &item("hover", format!("Toggle hover mode\t{}", keys.hover))?,
            &item("continuous", format!("Toggle continuous mode\t{}", keys.continuous))?,
            &item("continuous_area", format!("Continuous mode: choose area…\t{}", keys.continuous_area))?,
            &item("scan", format!("Label all Korean text on screen\t{}", keys.scan))?,
            &PredefinedMenuItem::separator(app)?,
            &item("review", "Review & history…".into())?,
            &item("status", "Status…".into())?,
            &PredefinedMenuItem::separator(app)?,
            &item("quit", "Quit".into())?,
        ],
    )?;
    let state = state.clone();
    let mut tray = TrayIconBuilder::with_id("maru").tooltip("Maru").menu(&menu).on_menu_event(move |app, event| {
        let (app, state) = (app.clone(), state.clone());
        match event.id.as_ref() {
            "translate" => flow::translate_selection(app, state),
            "region" => {
                let _ = windows::open_picker(&app);
            }
            "hover" => flow::toggle_hover(&app, &state),
            "continuous" => flow::toggle_continuous(&app, &state),
            "continuous_area" => flow::choose_continuous_area(&app, &state),
            "scan" => flow::scan_screen(app, state),
            "review" => {
                let _ = windows::open_hub(&app, "review");
            }
            "status" => {
                let _ = windows::open_hub(&app, "status");
            }
            "quit" => app.exit(0),
            _ => {}
        }
    });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

fn main() {
    // Start-up self-test of the Kiwi library runs in a child process of this very program.
    maru_engine::bootstrap::run_probe_if_requested();
    init_logging();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            let paths = Paths::discover();
            let cfg = Config::load(&paths.config_file).unwrap_or_else(|e| {
                tracing::error!("{e}; using defaults");
                Config::default()
            });
            // Engines start in the background (the language model loads while you work).
            let services = tauri::async_runtime::block_on(boot(paths, cfg, BootOptions::default()));
            for note in &services.notes {
                tracing::info!("{note}");
            }
            let live_region = Mutex::new(services.engine.config().continuous.region);
            let state = Arc::new(AppState { services: Arc::new(services), panel: Mutex::default(), continuous: Mutex::new(None), live_region, scan: Mutex::default() });

            let width = state.engine().config().ui.max_width_px as f32;
            windows::create_panel(app.handle(), width)?;
            windows::create_scan(app.handle())?;
            build_tray(app.handle(), &state)?;
            for problem in hotkeys::register(app.handle(), &state) {
                tracing::warn!("{problem}");
            }
            watch::spawn(app.handle().clone(), state.clone());
            watch::spawn_scan_hits(app.handle().clone(), state.clone());
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::ui_config,
            commands::deep_dive,
            commands::save_word,
            commands::unsave_word,
            commands::set_known,
            commands::export_anki,
            commands::review_word,
            commands::due_words,
            commands::recent_words,
            commands::history,
            commands::status,
            commands::resize_panel,
            commands::dismiss_panel,
            commands::set_pinned,
            commands::picker_done,
            commands::scan_labels,
            commands::scan_open,
            commands::word_contexts,
            commands::open_hub,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build the Maru application");

    app.run(|_handle, event| {
        // Closing the hub window must not quit: Maru lives in the tray.
        if let RunEvent::ExitRequested { api, code, .. } = event
            && code.is_none()
        {
            api.prevent_exit();
        }
    });
}
