//! Functions the pages can call. Thin: each one forwards to the engine.

use std::sync::Arc;

use maru_core::config::{GlossMode, Placement};
use maru_core::{Rect, WordStatus};
use maru_engine::{DeepDiveRequest, HistoryItem, ReviewResult};
use maru_llm::LlmStatus;
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::state::AppState;
use crate::{flow, windows};

type S<'a> = State<'a, Arc<AppState>>;
type Res<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Serialize)]
pub struct UiConfig {
    pub gloss_mode: GlossMode,
    pub font_size_px: u32,
    pub show_romanization: bool,
    pub show_pronunciation: bool,
    pub show_grammar_formula: bool,
    pub opacity: f32,
    pub max_width_px: u32,
    pub theme: String,
    pub anki: bool,
    /// Speech is set up: the panel offers to read aloud.
    pub tts: bool,
}

#[tauri::command]
pub fn ui_config(state: S) -> UiConfig {
    let cfg = state.cfg();
    let learning = state.engine().config().learning.gloss_mode;
    UiConfig {
        gloss_mode: learning,
        font_size_px: cfg.ui.font_size_px,
        show_romanization: cfg.ui.show_romanization,
        show_pronunciation: cfg.ui.show_pronunciation,
        show_grammar_formula: cfg.ui.show_grammar_formula,
        opacity: cfg.ui.opacity,
        max_width_px: cfg.ui.max_width_px,
        theme: cfg.ui.theme.clone(),
        anki: state.engine().config().learning.anki.enabled,
        tts: state.services.tts.is_some(),
    }
}

/// Start a deep dive; its events arrive on the usual channel tagged with the returned id.
#[tauri::command]
pub fn deep_dive(app: AppHandle, state: S, request: DeepDiveRequest) -> u64 {
    tracing::info!("deep dive: {:?}", request.eojeol.surface);
    let (id, rx) = state.engine().deep_dive(request);
    flow::pump(app, rx);
    id
}

#[tauri::command]
pub fn save_word(state: S, key: String) -> Res<WordStatus> {
    state.engine().save_word(&key).map_err(err)
}

#[tauri::command]
pub fn unsave_word(state: S, key: String) -> Res<()> {
    state.engine().unsave_word(&key).map_err(err)
}

#[tauri::command]
pub fn set_known(state: S, key: String, known: bool) -> Res<WordStatus> {
    state.engine().set_known(&key, known).map_err(err)
}

#[tauri::command]
pub async fn export_anki(state: S<'_>, key: String) -> Res<Option<u64>> {
    state.engine().export_anki(&key).await.map_err(err)
}

#[tauri::command]
pub fn review_word(state: S, key: String, rating: u8) -> Res<ReviewResult> {
    state.engine().review_word(&key, rating).map_err(err)
}

#[tauri::command]
pub fn due_words(state: S, limit: u32) -> Res<Vec<WordStatus>> {
    state.engine().due_words(limit).map_err(err)
}

#[tauri::command]
pub fn recent_words(state: S, limit: u32) -> Res<Vec<WordStatus>> {
    state.engine().recent_words(limit).map_err(err)
}

#[tauri::command]
pub fn history(state: S) -> Vec<HistoryItem> {
    state.engine().history()
}

#[derive(Serialize)]
pub struct Status {
    pub notes: Vec<String>,
    pub llm: String,
    pub analyser: &'static str,
    pub ocr: bool,
    pub dictionary: bool,
    pub hotkeys: maru_core::config::Hotkeys,
    pub vocab: Option<VocabCounts>,
    /// Hover mode is on.
    pub hover_mode: bool,
    /// Live subtitle mode is running, and on which screen area.
    pub live_mode: bool,
    pub live_area: Option<Rect>,
    /// The OCR reading model: `off`, `loading`, `ready` or `failed: …`.
    pub ocr_reader: String,
    /// Screen labels (from a scan) are showing.
    pub scan_visible: bool,
    /// The lite (game) profile is in effect.
    pub lite: bool,
    /// Speech: `off`, `loading`, `ready` or `failed: …`.
    pub tts: String,
    /// Where speech comes from: `local`, `server` or empty when off.
    pub tts_source: String,
}

#[derive(Serialize)]
pub struct VocabCounts {
    pub total: u32,
    pub saved: u32,
    pub known: u32,
    pub due_now: u32,
}

#[tauri::command]
pub fn status(state: S) -> Status {
    let e = state.engine();
    let live_mode = state.continuous.lock().map(|c| c.is_some()).unwrap_or(false);
    let llm = match e.llm_status() {
        LlmStatus::Ready => "ready".to_string(),
        LlmStatus::Starting => "loading".to_string(),
        LlmStatus::Disabled => "not configured".to_string(),
        LlmStatus::Failed(why) => format!("failed: {why}"),
    };
    Status {
        notes: state.services.notes.clone(),
        llm,
        analyser: e.analyzer_name(),
        ocr: e.has_ocr(),
        dictionary: e.has_dictionary(),
        hotkeys: state.cfg().hotkeys,
        vocab: e.vocab_stats().ok().map(|s| VocabCounts { total: s.total, saved: s.saved, known: s.known, due_now: s.due_now }),
        scan_visible: state.scan().visible,
        lite: state.services.lite,
        tts: match state.services.tts_status() {
            LlmStatus::Disabled => "off".into(),
            LlmStatus::Starting => "loading".into(),
            LlmStatus::Ready => "ready".into(),
            LlmStatus::Failed(why) => format!("failed: {why}"),
        },
        tts_source: match (state.services.tts.is_some(), e.config().tts.provider) {
            (false, _) => String::new(),
            (true, maru_core::config::TtsProvider::Local) => "local".into(),
            (true, maru_core::config::TtsProvider::Endpoint) => "server".into(),
        },
        hover_mode: state.panel().hover_mode,
        live_mode,
        live_area: if live_mode { state.live_region() } else { None },
        ocr_reader: match state.services.vision_status() {
            LlmStatus::Disabled => "off".into(),
            LlmStatus::Starting => "loading".into(),
            LlmStatus::Ready => "ready".into(),
            LlmStatus::Failed(why) => format!("failed: {why}"),
        },
    }
}

/// The page measured its content; resize the window to fit and re-place it.
#[tauri::command]
pub fn resize_panel(app: AppHandle, state: S, width: f32, height: f32) {
    let max_w = state.cfg().ui.max_width_px as f32 * 1.6;
    state.panel().size = (width.clamp(160.0, max_w), height.clamp(40.0, 1400.0));
    let placement: Placement = state.cfg().ui.placement;
    windows::place_and_show(&app, &state, placement);
}

#[tauri::command]
pub fn dismiss_panel(app: AppHandle, state: S) {
    windows::hide_panel(&app, &state);
}

#[tauri::command]
pub fn set_pinned(state: S, pinned: bool) {
    let mut p = state.panel();
    p.pinned = pinned;
    if !pinned {
        p.custom_pos = None; // unpinned: it goes back next to its text with the next answer
    }
}

/// The user drags the panel by its top bar. `dx`/`dy` are physical pixels from where the drag began.
/// A dragged panel pins itself and stays where it is put.
#[tauri::command]
pub fn panel_drag(app: AppHandle, state: S, phase: String, dx: f32, dy: f32) {
    let Some(win) = tauri::Manager::get_webview_window(&app, windows::PANEL) else { return };
    match phase.as_str() {
        "start" => {
            let at = win.outer_position().map(|p| (p.x as f32, p.y as f32)).unwrap_or_default();
            let mut p = state.panel();
            p.drag_origin = Some(at);
            p.pinned = true;
        }
        "move" => {
            let (origin, size) = {
                let p = state.panel();
                (p.drag_origin, win.outer_size().map(|s| (s.width as f32, s.height as f32)).unwrap_or(p.size))
            };
            let Some((ox, oy)) = origin else { return };
            let want = maru_core::Point { x: ox + dx, y: oy + dy };
            let monitor = windows::monitor_rect(&app, maru_core::Point { x: want.x + size.0 / 2.0, y: want.y + size.1 / 2.0 });
            let at = crate::logic::keep_on_screen(want, size, monitor);
            let _ = win.set_position(tauri::PhysicalPosition::new(at.x as i32, at.y as i32));
            state.panel().custom_pos = Some((at.x, at.y));
        }
        _ => state.panel().drag_origin = None,
    }
}

/// The scan overlay reports where its labels are, so the window can accept clicks only over them.
#[tauri::command]
pub fn scan_labels(state: S, dpr: f32, rects: Vec<Rect>) {
    let mut scan = state.scan();
    scan.dpr = dpr;
    scan.rects = rects;
}

/// A scan label was clicked.
#[tauri::command]
pub fn scan_open(app: AppHandle, state: S, text: String, rect: Rect) {
    flow::open_scan_item(app, state.inner().clone(), text, rect);
}

/// The picker reports a rectangle in *its own* pixels; add the window's screen origin.
#[tauri::command]
pub fn picker_done(app: AppHandle, state: S, rect: Option<Rect>) {
    let rect = rect.map(|r| {
        let origin = tauri::Manager::get_webview_window(&app, windows::PICKER).and_then(|w| w.outer_position().ok()).unwrap_or_default();
        Rect::new(r.x + origin.x as f32, r.y + origin.y as f32, r.w, r.h)
    });
    flow::picked(app, state.inner().clone(), rect);
}

#[derive(Serialize)]
pub struct WordContext {
    pub sentence: String,
    pub translation: Option<String>,
}

#[tauri::command]
pub fn word_contexts(state: S, key: String) -> Res<Vec<WordContext>> {
    let found = state.engine().word_contexts(&key, 3).map_err(err)?;
    Ok(found.into_iter().map(|c| WordContext { sentence: c.sentence, translation: c.translation }).collect())
}

#[tauri::command]
pub fn open_hub(app: AppHandle, tab: String) -> Res<()> {
    windows::open_hub(&app, &tab).map_err(err)
}

/// A spoken clip for the page to play.
#[derive(Serialize)]
pub struct Clip {
    pub mime: String,
    /// The audio file, base64.
    pub data: String,
    pub seconds: Option<f32>,
    pub cached: bool,
    pub took_ms: u64,
}

fn clip_of(audio: maru_tts::Audio, cached: bool, started: std::time::Instant) -> Clip {
    use base64::Engine as _;
    Clip {
        seconds: maru_tts::wav::info(&audio.bytes).map(|i| i.seconds),
        data: base64::engine::general_purpose::STANDARD.encode(&audio.bytes),
        mime: audio.mime,
        cached,
        took_ms: started.elapsed().as_millis() as u64,
    }
}

/// Speak a sentence with the *saved* speech settings, so a change can be tried without a restart
/// (a local voice still needs its server running, which only a restart starts).
#[tauri::command]
pub async fn tts_test(state: S<'_>, text: Option<String>) -> Res<Clip> {
    use maru_core::config::TtsProvider;
    let paths = state.services.paths.clone();
    let cfg = maru_core::Config::load(&paths.config_file).map_err(err)?.tts;
    if !cfg.enabled {
        return Err("Speech is switched off. Turn it on, save, and restart.".into());
    }
    if cfg.provider == TtsProvider::Local {
        match state.services.tts_status() {
            LlmStatus::Ready => {}
            LlmStatus::Starting => return Err("The local voice is still loading; try again in a moment.".into()),
            LlmStatus::Failed(why) => return Err(format!("The local voice is not running: {why}")),
            LlmStatus::Disabled => return Err("The local voice is not running yet. Save and restart Maru to start it.".into()),
        }
    }
    let tts = maru_tts::Tts::new(&cfg, &paths).map_err(err)?;
    let text = text.filter(|t| !t.trim().is_empty()).unwrap_or_else(|| "안녕하세요. 만나서 반갑습니다. 한국어를 공부하고 있어요.".into());
    let started = std::time::Instant::now();
    let (audio, cached) = tts.speak(&text).await.map_err(err)?;
    Ok(clip_of(audio, cached, started))
}

/// How `text` will be read: one entry per sentence.
#[tauri::command]
pub fn tts_plan(text: String) -> Vec<String> {
    maru_tts::split::sentences(&text)
}

/// Speech for one sentence (or a single word) with the voice that is running now.
#[tauri::command]
pub async fn tts_clip(state: S<'_>, text: String) -> Res<Clip> {
    let Some(tts) = state.services.tts.clone() else {
        return Err("Speech is off. Turn it on in Settings → Speech.".into());
    };
    match state.services.tts_status() {
        LlmStatus::Ready => {}
        LlmStatus::Starting => return Err("The voice is still loading; try again in a moment.".into()),
        LlmStatus::Failed(why) => return Err(format!("The voice is not available: {why}")),
        LlmStatus::Disabled => return Err("Speech is off.".into()),
    }
    let started = std::time::Instant::now();
    let (audio, cached) = tts.speak(&text).await.map_err(err)?;
    Ok(clip_of(audio, cached, started))
}

/// The panel starts or stops reading aloud; while it reads, it does not hide itself.
#[tauri::command]
pub fn tts_reading(state: S, active: bool) {
    state.panel().reading = active;
}
