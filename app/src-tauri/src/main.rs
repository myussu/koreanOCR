//! Maru overlay: a thin Tauri shell around `maru-engine`.
//!
//! All language, OCR and learning logic lives in the engine crates; this file wires
//! it to windows, hotkeys and the tray.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod flow;
mod hotkeys;
mod logic;
mod settings;
mod server_check;
mod state;
mod watch;
mod windows;
mod launcher;
mod resources;
mod correction;
mod scan_controls;
mod scan_actions;
mod reading;
mod wheel;
mod input_session;
mod picker_session;
mod reading_feedback;
mod mining;
mod captions;

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

/// The tray menu, with the shortcuts as currently configured.
pub fn tray_menu(app: &tauri::AppHandle, state: &AppState) -> tauri::Result<Menu<tauri::Wry>> {
    let keys = state.cfg().hotkeys;
    // Grouped like the floating controls: live subtitles, captures, windows, the controls themselves. A shortcut
    // shows only when one is set, so an unassigned action does not end in an empty tab.
    let item = |id: &str, label: &str, key: &str| {
        let text = if key.trim().is_empty() { label.to_string() } else { format!("{label}\t{}", key.replace('|', " or ")) };
        MenuItem::with_id(app, id, text, true, None::<&str>)
    };
    Menu::with_items(
        app,
        &[
            &item("continuous", "Live subtitles on / off", &keys.continuous)?,
            &item("continuous_area", "Live subtitles: choose the area…", &keys.continuous_area)?,
            &item("star", "Star the live subtitle line", &keys.star)?,
            &PredefinedMenuItem::separator(app)?,
            &item("region", "Snapshot a region…", &keys.region)?,
            &item("translate", "Translate the selection", &keys.translate)?,
            &item("hover", "Hover lookup on / off", &keys.hover)?,
            &item("scan", "Label all Korean on screen", &keys.scan)?,
            &item("speak", "Translate and read aloud", &keys.speak)?,
            &item("reading_again", "Read the saved game area again", &keys.reading_again)?,
            &PredefinedMenuItem::separator(app)?,
            &item("history", "History and cards…", &keys.history)?,
            &item("review", "Review…", "")?,
            &item("reading", "Saved game areas…", "")?,
            &item("settings", "Settings…", "")?,
            &item("status", "Status…", "")?,
            &PredefinedMenuItem::separator(app)?,
            &item("launcher", "Show / hide the floating controls", &keys.launcher)?,
            &item("wheel", "Controls wheel", &keys.wheel)?,
            &PredefinedMenuItem::separator(app)?,
            &item("quit", "Quit Maru", "")?,
        ],
    )
}

fn build_tray(app: &tauri::AppHandle, state: &Arc<AppState>) -> tauri::Result<()> {
    let menu = tray_menu(app, state)?;
    let state = state.clone();
    let mut tray = TrayIconBuilder::with_id("maru").tooltip("Maru").menu(&menu).on_menu_event(move |app, event| {
        let (app, state) = (app.clone(), state.clone());
        let id = event.id.as_ref().to_string();
        // WebView2 creation must not block the thread delivering this menu event.
        tauri::async_runtime::spawn(async move { match id.as_str() {
            "translate" => flow::translate_selection(app, state),
            "region" => {
                if let Err(e)=windows::open_picker(&app).await { flow::notice(&app,&state,None,"info",e); }
            }
            "reading_again" => reading::hotkey(app, state),
            "reading" => {
                state.engine().reading_context();
                let _ = windows::open_hub(&app, "reading");
            },
            "hover" => flow::toggle_hover(&app, &state),
            "continuous" => flow::toggle_continuous(&app, &state),
            "continuous_area" => flow::choose_continuous_area(&app, &state),
            "star" => flow::star_live_line(&app, &state),
            "scan" => flow::scan_screen(app, state),
            "speak" => flow::speak_selection(app, state),
            "review" => {
                let _ = windows::open_hub(&app, "review");
            }
            "history" => {
                let _ = windows::open_hub(&app, "history");
            }
            "status" => {
                let _ = windows::open_hub(&app, "status");
            }
            "settings" => {
                let _ = windows::open_hub(&app, "settings");
            }
            "quit" => app.exit(0),
            "launcher" => launcher::toggle(&app),
            "wheel" => wheel::hotkey(app,state),
            _ => {}
        }});
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
            // An older config gets "one model per task" written into it once; the file as it was is kept beside it.
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
            match maru_core::tasks::migrate_file(&paths.config_file, now) {
                Ok(Some(done)) => tracing::info!("config.toml now says which model each task uses; the file as it was is {}", done.backup.display()),
                Ok(None) => {}
                Err(e) => tracing::warn!("config.toml could not be updated with a model per task ({e}); the older settings decide"),
            }
            // The same for how each task reads the screen (`[reading]`).
            match maru_core::reading::migrate_file(&paths.config_file, now) {
                Ok(Some(done)) => tracing::info!("config.toml now says how each task reads the screen; the file as it was is {}", done.backup.display()),
                Ok(None) => {}
                Err(e) => tracing::warn!("config.toml could not be updated with how each task reads the screen ({e}); the older settings decide"),
            }
            // YouTube captions switched on in the file by hand: the code the extension must send is written before the settings are read.
            captions::write_code_if_missing(&paths.config_file);
            let cfg = Config::load(&paths.config_file).unwrap_or_else(|e| {
                tracing::error!("{e}; using defaults");
                Config::default()
            });
            let booted = cfg.clone();
            // Settings and controls stay available before any local GPU model is requested.
            let services = tauri::async_runtime::block_on(boot(paths, cfg, BootOptions { defer_models: true, ..Default::default() }));
            for note in &services.notes {
                tracing::info!("{note}");
            }
            let live_region = Mutex::new(services.engine.config().continuous.region);
            let state = Arc::new(AppState { services: Arc::new(services), panel: Mutex::default(), continuous: Mutex::new(None), live_region, live_latest: Default::default(), live_shown: Default::default(), live_error_shown: Default::default(), scan: Mutex::default(), live: std::sync::RwLock::new(booted.clone()), booted });
            app.manage(input_session::InputState::default());
            app.manage(picker_session::PickerSessionState::default());
            app.manage(reading_feedback::FeedbackState::default());
            app.manage(wheel::WheelState::default());

            let width = state.cfg().ui.max_width_px as f32;
            windows::create_panel(app.handle(), width)?;
            windows::create_scan(app.handle())?;
            windows::create_live(app.handle())?;
            launcher::create(app.handle(), state.clone())?;
            app.manage(maru_engine::resource_monitor::ResourceService::start(state.services.clone()));
            app.manage(maru_engine::server_discovery::DiscoveryService::default());
            app.manage(correction::CorrectionState::default());
            build_tray(app.handle(), &state)?;
            for problem in hotkeys::register(app.handle(), &state) {
                tracing::warn!("{problem}");
            }
            watch::spawn(app.handle().clone(), state.clone());
            watch::spawn_anki_refresh(state.clone());
            watch::spawn_scan_hits(app.handle().clone(), state.clone());
            reading::spawn(app.handle().clone(), state.clone());
            reading_feedback::spawn(app.handle().clone(),state.clone());
            captions::start(app.handle(), state.clone());
            // A model on another computer is loaded now rather than at the first subtitle ([llm] server_keep_minutes).
            state.engine().hold_server_models(false);
            app.manage(state);
            Ok(())
        })
        .on_window_event(|window,event| {
            if window.label()==windows::HUB && matches!(event,tauri::WindowEvent::Focused(false)) && let Some(hub)=window.app_handle().get_webview_window(windows::HUB) { input_session::hub_focus_lost(&hub); }
            if !matches!(window.label(),"wheel"|"picker") { return; }
            if matches!(event,tauri::WindowEvent::Focused(false)) { input_session::focus_changed(window.app_handle()); }
            if let tauri::WindowEvent::CloseRequested { api,.. } = event {
                api.prevent_close();
                let app=window.app_handle().clone();
                if window.label()=="wheel" {
                    tauri::async_runtime::spawn(async move { if let Err(e)=wheel::wheel_close(app).await { tracing::warn!("controls wheel return: {e}"); } });
                } else if let Some(state)=app.try_state::<Arc<AppState>>() {
                    let state=state.inner().clone();
                    let session_id=picker_session::current(&app);
                    tauri::async_runtime::spawn(async move { flow::picked(app,state,None,session_id).await; });
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            reading_feedback::reading_progress_ready,
            reading_feedback::reading_progress_status,
            picker_session::picker_session,
            picker_session::picker_ready,
            wheel::wheel_status,
            wheel::wheel_action,
            wheel::wheel_close,
            wheel::wheel_ready,
            reading::reading_status,
            reading::reading_picker_hint,
            reading::reading_create,
            reading::reading_select,
            reading::reading_rename,
            reading::reading_remove,
            reading::reading_auto,
            reading::reading_hold,
            reading::reading_confirm_position,
            reading::reading_read,
            correction::correction_open_snapshot,
            correction::correction_open_scan,
            correction::correction_view,
            correction::correction_apply,
            correction::correction_forget,
            correction::correction_cancel,
            commands::ui_config,
            commands::live_idle,
            commands::history_open,
            commands::deep_dive,
            commands::save_word,
            commands::unsave_word,
            commands::set_known,
            commands::anki_overview,
            commands::anki_add,
            commands::anki_add_example,
            commands::anki_sync,
            commands::anki_check,
            commands::anki_refresh,
            commands::anki_due,
            commands::review_word,
            commands::due_words,
            commands::recent_words,
            commands::history,
            commands::history_star,
            commands::history_starred,
            mining::mine_sheet,
            mining::mine_sentence_translation,
            mining::mine_queue,
            mining::task_status,
            mining::task_test,
            mining::connection_test,
            mining::mine_add,
            mining::anki_fill_sounds,
            mining::open_cards,
            mining::take_pending_cards,
            commands::status,
            commands::resize_panel,
            commands::dismiss_panel,
            commands::set_pinned,
            commands::panel_drag,
            commands::panel_typing,
            commands::picker_done,
            commands::scan_labels,
            commands::scan_open,
            commands::scan_join,
            commands::scan_unjoin,
            scan_controls::scan_join_mode,
            scan_actions::scan_join_items,
            scan_actions::scan_separate,
            scan_actions::scan_open_item,
            commands::word_contexts,
            commands::open_hub,
            commands::tts_test,
            commands::tts_plan,
            commands::tts_clip,
            commands::tts_reading,
            settings::get_settings,
            server_check::test_model_connection,
            server_check::connection_check,
            server_check::discovery_networks,
            server_check::discovery_start,
            server_check::discovery_status,
            server_check::discovery_cancel,
            settings::save_settings,
            settings::restart_app,
            settings::open_config_file,
            launcher::launcher_status,
            launcher::launcher_action,
            launcher::launcher_rects,
            launcher::launcher_drag,
            launcher::launcher_expand,
            resources::resource_status,
            resources::resource_record,
            resources::resource_enable,
            captions::captions_status,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build the Maru application");

    app.run(|handle, event| {
        if matches!(event,RunEvent::ExitRequested {code:Some(_),..}) { input_session::finish_all(handle); }
        if let RunEvent::Exit = event && let Some(service) = handle.try_state::<maru_engine::resource_monitor::ResourceService>() {
            service.shutdown();
        }
        // Give the other computer its GPU back; an unreachable server must not hold up quitting.
        if let RunEvent::Exit = event && let Some(state) = handle.try_state::<Arc<AppState>>() {
            let engine = state.engine().clone();
            // The timer must be created inside the runtime, not on this (main) thread.
            tauri::async_runtime::block_on(async move {
                let _ = tokio::time::timeout(std::time::Duration::from_secs(2), engine.release_server_models()).await;
            });
        }
        // Closing the hub window must not quit: Maru lives in the tray.
        if let RunEvent::ExitRequested { api, code, .. } = event
            && code.is_none()
        {
            api.prevent_exit();
        }
    });
}
