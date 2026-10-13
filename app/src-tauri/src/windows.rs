//! The three windows: the floating panel, the region picker, and the hub.

use maru_core::config::Placement;
use maru_core::{Point, Rect};
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::logic;
use crate::state::AppState;

pub const PANEL: &str = "panel";
pub const PICKER: &str = "picker";
pub const HUB: &str = "hub";
pub const SCAN: &str = "scan";
pub const LIVE: &str = "live";

/// Create the (hidden) panel at start-up so the page is loaded and composited before first use.
pub fn create_panel(app: &AppHandle, width: f32) -> tauri::Result<WebviewWindow> {
    let builder = WebviewWindowBuilder::new(app, PANEL, WebviewUrl::App("index.html".into()))
        .title("Maru")
        .inner_size(width as f64, 200.0)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        // Never take keyboard focus: clicking the panel must not clear the user's text selection.
        .focusable(false)
        // Keep the overlay out of screenshots, so continuous mode never reads its own output.
        .content_protected(true)
        .visible(false);
    // The panel reads aloud when a hotkey asks it to, with no click inside it; WebView2 would refuse
    // to play sound without one. (Setting the arguments replaces Tauri's defaults, so repeat them.)
    #[cfg(windows)]
    let builder = builder.additional_browser_args("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required");
    builder.build()
}

/// The (hidden) layer that carries the screen-scan labels. Created at start-up so its page is
/// already loaded when the first scan's events arrive; click-through until the pointer is over a label.
pub fn create_scan(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let win = WebviewWindowBuilder::new(app, SCAN, WebviewUrl::App("scan.html".into()))
        .title("Maru — screen labels")
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        // Never take focus from the game, and stay out of screenshots so a scan never reads its own labels.
        .focusable(false)
        .content_protected(true)
        .visible(false)
        .build()?;
    let _ = win.set_ignore_cursor_events(true);
    Ok(win)
}

/// The (hidden) window that carries the live-subtitle translation. Click-through, never focused, and
/// out of screenshots so live mode never reads its own translation back.
pub fn create_live(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let win = WebviewWindowBuilder::new(app, LIVE, WebviewUrl::App("live.html".into()))
        .title("Maru — live subtitles")
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focusable(false)
        .content_protected(true)
        .visible(false)
        .build()?;
    let _ = win.set_ignore_cursor_events(true);
    Ok(win)
}

/// Put the live window over the subtitle area and the room above and below it, where the band is drawn clear of the original (it stays hidden until a translation arrives).
pub fn place_live(app: &AppHandle, window: Rect) {
    let Some(win) = app.get_webview_window(LIVE) else { return };
    let _ = win.set_position(PhysicalPosition::new(window.x as i32, window.y as i32));
    let _ = win.set_size(PhysicalSize::new(window.w as u32, window.h as u32));
}

pub fn show_live(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(LIVE)
        && !win.is_visible().unwrap_or(false)
    {
        let _ = win.set_ignore_cursor_events(true);
        let _ = win.show();
    }
}

pub fn hide_live(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(LIVE) {
        let _ = tauri::Emitter::emit_to(app, LIVE, "maru-live-off", ());
        let _ = win.hide();
    }
}

/// Cover `monitor` with the scan layer and show it (the game keeps the keyboard).
pub fn show_scan(app: &AppHandle, monitor: Rect) {
    let Some(win) = app.get_webview_window(SCAN) else { return };
    let _ = win.set_ignore_cursor_events(true);
    let _ = win.set_position(PhysicalPosition::new(monitor.x as i32, monitor.y as i32));
    let _ = win.set_size(PhysicalSize::new(monitor.w as u32, monitor.h as u32));
    let _ = win.show();
    // Labels sit behind an open panel, not over it.
    if let Some(panel) = app.get_webview_window(PANEL)
        && panel.is_visible().unwrap_or(false)
    {
        let _ = panel.set_always_on_top(true);
    }
}

/// Remove every scan label and hide the layer.
pub fn hide_scan(app: &AppHandle, state: &AppState) {
    let scan_id = state.scan().id;
    let _ = tauri::Emitter::emit_to(app, SCAN, "maru-scan-clear", serde_json::json!({"scanId":scan_id}));
    if let Some(win) = app.get_webview_window(SCAN) {
        let _ = win.set_ignore_cursor_events(true);
        let _ = win.hide();
    }
    *state.scan() = crate::state::ScanState::default();
}

/// Union of all monitors in physical pixels (the virtual desktop).
pub fn virtual_screen(app: &AppHandle) -> Rect {
    let monitors = app.available_monitors().unwrap_or_default();
    monitors
        .iter()
        .map(|m| Rect::new(m.position().x as f32, m.position().y as f32, m.size().width as f32, m.size().height as f32))
        .reduce(|a, b| a.union(&b))
        .unwrap_or(Rect::new(0.0, 0.0, 1920.0, 1080.0))
}

pub fn monitor_rect(app: &AppHandle, at: Point) -> Rect {
    match app.monitor_from_point(at.x as f64, at.y as f64) {
        Ok(Some(m)) => Rect::new(m.position().x as f32, m.position().y as f32, m.size().width as f32, m.size().height as f32),
        _ => virtual_screen(app),
    }
}

/// Move/resize the panel next to its anchor and make sure it is visible.
pub fn place_and_show(app: &AppHandle, state: &AppState, placement: Placement) {
    let Some(win) = app.get_webview_window(PANEL) else { return };
    let (anchor, size) = {
        let p = state.panel();
        (p.anchor, p.size)
    };
    let monitor = monitor_rect(app, Point { x: anchor.x + anchor.w / 2.0, y: anchor.y + anchor.h / 2.0 });
    let mut at = logic::place_panel(anchor, size, monitor, placement);
    // A panel the user dragged stays where it was put, however its content grows.
    if let Some((x, y)) = state.panel().custom_pos {
        let there = Point { x, y };
        at = logic::keep_on_screen(there, size, monitor_rect(app, Point { x: x + size.0 / 2.0, y: y + size.1 / 2.0 }));
    }
    let _ = win.set_size(PhysicalSize::new(size.0.max(1.0) as u32, size.1.max(1.0) as u32));
    let _ = win.set_position(PhysicalPosition::new(at.x as i32, at.y as i32));
    if !state.panel().visible {
        let _ = win.show();
        state.panel().visible = true;
    }
    // Two always-on-top windows: whichever was raised last is in front, so the panel claims it
    // back every time it is placed (the scan layer may have been shown after it).
    let _ = win.set_always_on_top(true);
}

pub fn hide_panel(app: &AppHandle, state: &AppState) {
    if let Some(win) = app.get_webview_window(PANEL) {
        let _ = win.set_focusable(false);
        let _ = win.hide();
    }
    {
        let mut p = state.panel();
        p.visible = false;
        p.reading = false;
        p.editing = false;
    }
    // whatever was being read aloud stops with it
    let _ = tauri::Emitter::emit_to(app, PANEL, "maru-hidden", ());
}

/// Where the panel currently is, in physical screen pixels.
pub fn panel_rect(app: &AppHandle) -> Option<Rect> {
    let win = app.get_webview_window(PANEL)?;
    let (p, s) = (win.outer_position().ok()?, win.outer_size().ok()?);
    Some(Rect::new(p.x as f32, p.y as f32, s.width as f32, s.height as f32))
}

/// Full-desktop transparent window for drawing a rectangle.
pub async fn open_picker(app: &AppHandle) -> Result<(), String> {
    let slot=crate::input_session::reserve_picker(app).await?;
    open_reserved_picker(app,&slot).await
}

pub async fn open_reserved_picker(app: &AppHandle, _slot: &tokio::sync::OwnedMutexGuard<()>) -> Result<(),String> {
    let snapshot = crate::input_session::prepare(app).await?;
    let v = virtual_screen(app);
    let win = WebviewWindowBuilder::new(app, PICKER, WebviewUrl::App("picker.html".into()))
        .title("Maru — pick a region")
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .content_protected(true)
        .focused(false)
        .visible(false)
        .build().map_err(|e| e.to_string())?;
    let mut session_id=None;
    let opened = async {
        win.set_position(PhysicalPosition::new(v.x as i32, v.y as i32)).map_err(|e| e.to_string())?;
        win.set_size(PhysicalSize::new(v.w as u32, v.h as u32)).map_err(|e| e.to_string())?;
        let id=crate::picker_session::begin(app);
        session_id=Some(id);
        crate::picker_session::wait_ready(app,id).await?;
        crate::launcher::pause_for_picker(app);
        crate::input_session::show(app, &win, snapshot, true).await
    }.await;
    if let Err(error) = opened {
        if let Some(id)=session_id { crate::picker_session::clear(app,id); }
        let _ = crate::input_session::finish(app, &win).await;
        let _ = win.destroy();
        crate::launcher::resume_after_picker(app);
        tracing::warn!("region picker: {error}");
        return Err(error);
    }
    Ok(())
}

pub async fn close_picker(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(PICKER) {
        if let Err(error) = crate::input_session::finish(app, &w).await { tracing::warn!("region picker return: {error}"); }
        let _ = w.destroy();
    }
    crate::launcher::resume_after_picker(app);
    if let Some(id)=crate::picker_session::current(app) { crate::picker_session::clear(app,id); }
}

/// Review / history / status window (a normal, focusable window).
pub fn open_hub(app: &AppHandle, tab: &str) -> tauri::Result<()> {
    if let Some(w) = app.get_webview_window(HUB) {
        let _ = tauri::Emitter::emit_to(app, HUB, "maru-tab", tab);
        w.show()?;
        w.set_focus()?;
        return Ok(());
    }
    let url = format!("hub.html#{tab}");
    WebviewWindowBuilder::new(app, HUB, WebviewUrl::App(url.into())).title("Maru").inner_size(940.0, 720.0).min_inner_size(640.0, 480.0).build()?;
    Ok(())
}
