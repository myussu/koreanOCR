//! The three windows: the floating panel, the region picker, and the hub.

use maru_core::config::Placement;
use maru_core::{Point, Rect};
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::logic;
use crate::state::AppState;

pub const PANEL: &str = "panel";
pub const PICKER: &str = "picker";
pub const HUB: &str = "hub";

/// Create the (hidden) panel at start-up so the page is loaded and composited before first use.
pub fn create_panel(app: &AppHandle, width: f32) -> tauri::Result<WebviewWindow> {
    WebviewWindowBuilder::new(app, PANEL, WebviewUrl::App("index.html".into()))
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
        .visible(false)
        .build()
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

fn monitor_rect(app: &AppHandle, at: Point) -> Rect {
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
    let at = logic::place_panel(anchor, size, monitor, placement);
    let _ = win.set_size(PhysicalSize::new(size.0.max(1.0) as u32, size.1.max(1.0) as u32));
    let _ = win.set_position(PhysicalPosition::new(at.x as i32, at.y as i32));
    if !state.panel().visible {
        let _ = win.show();
        state.panel().visible = true;
    }
}

pub fn hide_panel(app: &AppHandle, state: &AppState) {
    if let Some(win) = app.get_webview_window(PANEL) {
        let _ = win.hide();
    }
    state.panel().visible = false;
}

/// Where the panel currently is, in physical screen pixels.
pub fn panel_rect(app: &AppHandle) -> Option<Rect> {
    let win = app.get_webview_window(PANEL)?;
    let (p, s) = (win.outer_position().ok()?, win.outer_size().ok()?);
    Some(Rect::new(p.x as f32, p.y as f32, s.width as f32, s.height as f32))
}

/// Full-desktop transparent window for drawing a rectangle.
pub fn open_picker(app: &AppHandle) -> tauri::Result<()> {
    if let Some(existing) = app.get_webview_window(PICKER) {
        let _ = existing.close();
    }
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
        .focused(true)
        .visible(false)
        .build()?;
    win.set_position(PhysicalPosition::new(v.x as i32, v.y as i32))?;
    win.set_size(PhysicalSize::new(v.w as u32, v.h as u32))?;
    win.show()?;
    win.set_focus()?;
    Ok(())
}

pub fn close_picker(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(PICKER) {
        let _ = w.close();
    }
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
