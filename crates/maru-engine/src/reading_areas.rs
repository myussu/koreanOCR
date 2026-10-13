//! Saved game regions use ordinary snapshot OCR and translation. Watching never loads a live model.
use crate::change::{Signature, changed_fraction, signature};
use crate::engine::{EngineError, Result, lock};
use crate::reading_change::TextChange;
use crate::reading_store::Store;
use crate::{CaptureError, Engine};
use image::RgbImage;
use maru_core::{Capture, CaptureSource, Point, Rect};
use maru_platform::app_window::AppWindow;
use serde::Serialize;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub use crate::reading_store::ReadingArea;

#[derive(Clone)]
struct Pick {
    area: Option<ReadingArea>,
    name: String,
    window: AppWindow,
}
struct Pending {
    serial: u64,
    capture: Capture,
    manual: bool,
    window: AppWindow,
}

#[derive(Default)]
pub(crate) struct ReadingState {
    store: Option<Store>,
    areas: Vec<ReadingArea>,
    target: Option<AppWindow>,
    selected: Option<i64>,
    automatic: bool,
    held: bool,
    picker: Option<Pick>,
    serial: u64,
    pending: Option<Pending>,
    active: Option<u64>,
    retained: Option<u64>,
    manual_capture: Arc<tokio::sync::Mutex<()>>,
    change: TextChange,
    frame: Option<Signature>,
    dirty: bool,
    frame_since: Option<Instant>,
    looked_at: Option<Instant>,
    suspended: bool,
    phase: &'static str,
    message: Option<String>,
}

impl ReadingState {
    fn reset(&mut self) {
        self.serial = self.serial.wrapping_add(1);
        self.pending = None;
        self.retained = None;
        self.change.reset();
        self.frame = None;
        self.dirty = true;
        self.frame_since = None;
        self.looked_at = None;
        self.message = None;
        self.phase = "ready";
    }
    fn area(&self) -> Option<&ReadingArea> {
        self.areas.iter().find(|a| Some(a.id) == self.selected)
    }
    fn store(&mut self) -> Result<&mut Store> {
        self.store
            .as_mut()
            .ok_or_else(|| error("storage is unavailable; check Status for the startup error"))
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ReadingStatus {
    pub areas: Vec<ReadingArea>,
    pub selected: Option<i64>,
    pub game: Option<String>,
    pub exe: Option<String>,
    pub automatic: bool,
    pub held: bool,
    pub phase: String,
    pub message: Option<String>,
    pub pending: bool,
    pub busy: bool,
    pub can_create: bool,
    pub position_check: bool,
}

pub struct ReadingJob {
    pub serial: u64,
    pub capture: Capture,
}
fn error(message: &str) -> EngineError {
    EngineError::Other(format!("Saved reading areas: {message}"))
}
fn own_app(exe: &str) -> bool {
    ["maru-app.exe", "maru.exe", "maru-app", "maru"]
        .iter()
        .any(|name| exe.eq_ignore_ascii_case(name))
}
fn same_window(a: &AppWindow, b: &AppWindow) -> bool {
    a.identity == b.identity && a.rect == b.rect && a.app.exe.eq_ignore_ascii_case(&b.app.exe)
}
fn name(text: &str) -> Result<String> {
    let text = text.trim();
    if text.is_empty() || text.chars().count() > 64 || text.chars().any(char::is_control) {
        return Err(error("give the area a name of 1–64 characters"));
    }
    Ok(text.into())
}
fn normalized(rect: Rect, window: Rect) -> Result<Rect> {
    if [
        rect.x, rect.y, rect.w, rect.h, window.x, window.y, window.w, window.h,
    ]
    .iter()
    .any(|v| !v.is_finite())
        || rect.w < 8.0
        || rect.h < 8.0
        || window.is_empty()
        || rect.x < window.x - 1.0
        || rect.y < window.y - 1.0
        || rect.right() > window.right() + 1.0
        || rect.bottom() > window.bottom() + 1.0
    {
        return Err(error(
            "draw an area inside the game's visible content, at least 8 × 8 pixels",
        ));
    }
    let x = ((rect.x - window.x) / window.w).max(0.0);
    let y = ((rect.y - window.y) / window.h).max(0.0);
    Ok(Rect::new(
        x,
        y,
        ((rect.right().min(window.right()) - window.x) / window.w - x).max(0.0),
        ((rect.bottom().min(window.bottom()) - window.y) / window.h - y).max(0.0),
    ))
}

impl Engine {
    pub fn open_reading_areas(&self, path: &Path) -> Result<()> {
        let store = Store::open(path)?;
        let areas = store.list()?;
        let mut state = lock(&self.inner.reading_areas);
        state.store = Some(store);
        state.areas = areas;
        Ok(())
    }

    /// Observe application identity even while watching is off; opening Maru keeps the game target.
    pub fn reading_context(&self) {
        let screen = &self.inner.platform.screen;
        let Some(app) = screen.foreground_app().filter(|a| !own_app(&a.exe)) else {
            return;
        };
        let Some(window) = screen.app_window(&app.exe) else {
            return;
        };
        let mut state = lock(&self.inner.reading_areas);
        if state.automatic
            || state.picker.is_some()
            || state.active.is_some()
            || state.pending.is_some()
            || state.phase == "capturing"
        {
            return;
        }
        if state
            .target
            .as_ref()
            .is_none_or(|w| !w.app.exe.eq_ignore_ascii_case(&app.exe))
        {
            state.reset();
            state.held = false;
            state.selected = state
                .areas
                .iter()
                .find(|a| a.exe.eq_ignore_ascii_case(&app.exe) && a.selected)
                .or_else(|| {
                    state
                        .areas
                        .iter()
                        .find(|a| a.exe.eq_ignore_ascii_case(&app.exe))
                })
                .map(|a| a.id);
        }
        state.target = Some(window);
    }

    pub fn reading_status(&self) -> ReadingStatus {
        let state = lock(&self.inner.reading_areas);
        let position_check = state
            .area()
            .and_then(|a| {
                self.inner
                    .platform
                    .screen
                    .app_window(&a.exe)
                    .map(|w| a.needs_check(w.rect))
            })
            .unwrap_or(false);
        let phase = if state.picker.is_some() {
            "choosing"
        } else if state.held && state.automatic {
            "held"
        } else if state.phase.is_empty() {
            "ready"
        } else {
            state.phase
        };
        ReadingStatus {
            areas: state.areas.clone(),
            selected: state.selected,
            game: state
                .area()
                .map(|a| a.game.clone())
                .or_else(|| state.target.as_ref().map(|w| w.app.title.clone())),
            exe: state
                .area()
                .map(|a| a.exe.clone())
                .or_else(|| state.target.as_ref().map(|w| w.app.exe.clone())),
            automatic: state.automatic,
            held: state.held,
            phase: phase.into(),
            message: state.message.clone(),
            pending: state.pending.is_some(),
            busy: state.active.is_some(),
            can_create: state.store.is_some() && state.target.is_some(),
            position_check,
        }
    }

    pub fn select_reading_area(&self, id: i64) -> Result<()> {
        let mut state = lock(&self.inner.reading_areas);
        let area = state
            .areas
            .iter()
            .find(|a| a.id == id)
            .cloned()
            .ok_or_else(|| error("this area no longer exists"))?;
        state.store()?.select(&area)?;
        state.areas = state.store()?.list()?;
        state.reset();
        state.selected = Some(id);
        state.automatic = false;
        state.held = false;
        state.target = self.inner.platform.screen.app_window(&area.exe);
        Ok(())
    }

    pub fn rename_reading_area(&self, id: i64, new_name: &str) -> Result<()> {
        let new_name = name(new_name)?;
        let mut state = lock(&self.inner.reading_areas);
        state.store()?.rename(id, &new_name)?;
        state.areas = state.store()?.list()?;
        Ok(())
    }

    pub fn remove_reading_area(&self, id: i64) -> Result<()> {
        let mut state = lock(&self.inner.reading_areas);
        state.store()?.remove(id)?;
        state.areas = state.store()?.list()?;
        if state.selected == Some(id) {
            state.reset();
            state.selected = None;
            state.automatic = false;
            state.held = false;
        }
        Ok(())
    }

    pub fn begin_reading_pick(&self, label: &str, id: Option<i64>) -> Result<()> {
        let label = name(label)?;
        self.reading_context();
        let mut state = lock(&self.inner.reading_areas);
        if state.picker.is_some() {
            return Err(error("finish or cancel the open area selection first"));
        }
        let area = id
            .map(|id| {
                state
                    .areas
                    .iter()
                    .find(|a| a.id == id)
                    .cloned()
                    .ok_or_else(|| error("this area no longer exists"))
            })
            .transpose()?;
        let window = if let Some(area) = &area {
            self.inner.platform.screen.app_window(&area.exe)
        } else {
            state.target.clone()
        }
        .ok_or_else(|| {
            error("open the game and use Save reading area from the floating controls")
        })?;
        if state.areas.len() >= 100 && area.is_none() {
            return Err(error(
                "there are already 100 saved areas; remove an unused area first",
            ));
        }
        if self
            .inner
            .platform
            .screen
            .foreground_app()
            .is_some_and(|a| !own_app(&a.exe) && !a.exe.eq_ignore_ascii_case(&window.app.exe))
        {
            return Err(error(
                "bring the selected game to the foreground before choosing its area",
            ));
        }
        state.store()?;
        state.reset();
        state.picker = Some(Pick {
            area,
            name: label,
            window,
        });
        Ok(())
    }

    pub fn finish_reading_pick(&self, rect: Option<Rect>) -> Result<Option<ReadingArea>> {
        let mut state = lock(&self.inner.reading_areas);
        let Some(pick) = state.picker.take() else {
            return Ok(None);
        };
        let Some(rect) = rect else { return Ok(None) };
        let current = self
            .inner
            .platform
            .screen
            .app_window(&pick.window.app.exe)
            .ok_or_else(|| error("the game window disappeared; select the area again"))?;
        if current.identity != pick.window.identity || current.rect != pick.window.rect {
            return Err(error(
                "the game window moved or changed size during selection; select the area again",
            ));
        }
        let region = normalized(rect, current.rect)?;
        let game = self
            .inner
            .cfg
            .profile_for(&current.app.exe)
            .map(|p| p.name.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(&current.app.title)
            .to_string();
        let area = ReadingArea {
            id: pick.area.map_or(0, |a| a.id),
            exe: current.app.exe.to_ascii_lowercase(),
            game,
            name: pick.name,
            region,
            reference_width: current.rect.w,
            reference_height: current.rect.h,
            selected: true,
        };
        let area = state.store()?.save(area)?;
        state.areas = state.store()?.list()?;
        state.reset();
        state.selected = Some(area.id);
        state.target = Some(current);
        state.automatic = false;
        state.held = false;
        Ok(Some(area))
    }

    pub fn reading_picker_open(&self) -> bool {
        lock(&self.inner.reading_areas).picker.is_some()
    }
    pub fn reading_picker_hint(&self) -> Option<(String, Option<Rect>)> {
        let state = lock(&self.inner.reading_areas);
        state.picker.as_ref().map(|pick| {
            (
                pick.name.clone(),
                pick.area
                    .as_ref()
                    .map(|area| area.rectangle(pick.window.rect)),
            )
        })
    }

    pub fn set_reading_auto(&self, enabled: bool) -> Result<()> {
        let mut state = lock(&self.inner.reading_areas);
        if state.picker.is_some() {
            return Err(error("finish or cancel the area selection first"));
        }
        if enabled {
            state
                .area()
                .ok_or_else(|| error("save or select a reading area first"))?;
            if self.inner.ocr.is_none() {
                return Err(CaptureError::NoOcr.into());
            }
        }
        let retained = if enabled { None } else { state.active };
        state.reset();
        state.retained = retained;
        state.automatic = enabled;
        state.held = false;
        state.suspended = false;
        state.phase = if enabled { "watching" } else { "ready" };
        Ok(())
    }

    pub fn set_reading_hold(&self, held: bool) {
        let mut state = lock(&self.inner.reading_areas);
        state.held = held;
        if held {
            if state.phase == "capturing" {
                state.serial = state.serial.wrapping_add(1);
            }
            state.retained = state.active;
            state.pending = None;
        } else {
            state.change.reset();
            state.frame = None;
            state.dirty = true;
            state.frame_since = None;
            state.looked_at = None;
        }
    }

    pub fn hold_reading_for_interaction(&self) {
        let mut state = lock(&self.inner.reading_areas);
        if state.automatic {
            state.held = true;
            if state.phase == "capturing" {
                state.serial = state.serial.wrapping_add(1);
            }
            state.retained = state.active;
            state.pending = None;
        }
    }
    /// A different snapshot replaces the panel; its newer events must own the display.
    pub fn interrupt_reading(&self) {
        let mut state = lock(&self.inner.reading_areas);
        if state.automatic {
            state.held = true;
        }
        state.reset();
    }

    pub fn confirm_reading_position(&self) -> Result<()> {
        let mut state = lock(&self.inner.reading_areas);
        let mut area = state
            .area()
            .cloned()
            .ok_or_else(|| error("select a reading area first"))?;
        let window = self
            .inner
            .platform
            .screen
            .app_window(&area.exe)
            .ok_or_else(|| error("the game window is not visible"))?;
        normalized(area.rectangle(window.rect), window.rect)?;
        area.reference_width = window.rect.w;
        area.reference_height = window.rect.h;
        state.store()?.save(area)?;
        state.areas = state.store()?.list()?;
        state.reset();
        state.target = Some(window);
        Ok(())
    }

    fn reading_window(&self, manual: bool) -> Result<(ReadingArea, AppWindow)> {
        let area = lock(&self.inner.reading_areas)
            .area()
            .cloned()
            .ok_or_else(|| error("save or select a reading area first"))?;
        let foreground = self
            .inner
            .platform
            .screen
            .foreground_app()
            .ok_or_else(|| error("bring the game to the foreground"))?;
        if !foreground.exe.eq_ignore_ascii_case(&area.exe) && !(manual && own_app(&foreground.exe))
        {
            return Err(error("bring the selected game to the foreground"));
        }
        let window = self
            .inner
            .platform
            .screen
            .app_window(&area.exe)
            .ok_or_else(|| error("the game window is minimized or no longer visible"))?;
        if area.needs_check(window.rect) {
            return Err(error(
                "the game window changed size; check or reposition the saved area",
            ));
        }
        let rect = area.rectangle(window.rect);
        let bounds = self.inner.platform.screen.screen_bounds();
        normalized(rect, window.rect)?;
        if rect.overlap_fraction(&bounds) < 0.999 {
            return Err(error("move the whole reading area onto the screen"));
        }
        Ok((area, window))
    }

    async fn read_image(&self, image: RgbImage, rect: Rect, window: AppWindow) -> Result<Capture> {
        let ocr = self.inner.ocr.clone().ok_or(CaptureError::NoOcr)?;
        let cursor = self.inner.platform.screen.cursor();
        tokio::task::spawn_blocking(move || {
            let out = ocr
                .recognize(&image)
                .map_err(|e| error(&e.to_string()))?
                .to_screen(
                    Point {
                        x: rect.x,
                        y: rect.y,
                    },
                    1.0,
                );
            Ok(Capture {
                text: out
                    .lines
                    .iter()
                    .map(|l| l.text.trim())
                    .filter(|t| !t.is_empty())
                    .collect::<Vec<_>>()
                    .join(" "),
                source: CaptureSource::Ocr,
                rect: out.bounds().or(Some(rect)),
                words: out.words(),
                cursor,
                app: Some(window.app),
                confidence: Some(out.confidence()),
            })
        })
        .await
        .map_err(|e| error(&e.to_string()))?
    }

    async fn reading_frame(&self, rect: Rect) -> Result<RgbImage> {
        let screen = self.inner.platform.screen.clone();
        tokio::task::spawn_blocking(move || screen.grab(rect).map_err(|e| error(&e.to_string())))
            .await
            .map_err(|e| error(&e.to_string()))?
    }

    pub async fn request_reading(&self) -> Result<()> {
        let gate = lock(&self.inner.reading_areas).manual_capture.clone();
        let _capture = gate
            .try_lock_owned()
            .map_err(|_| error("a manual capture is already in progress; wait for it to finish"))?;
        if self.reading_picker_open() {
            return Err(error("finish or cancel the area selection first"));
        }
        self.reading_context();
        let (area, window) = self.reading_window(true)?;
        let serial = {
            let mut state = lock(&self.inner.reading_areas);
            state.reset();
            state.phase = "capturing";
            state.target = Some(window.clone());
            state.serial
        };
        let rect = area.rectangle(window.rect);
        let result = async {
            self.read_image(self.reading_frame(rect).await?, rect, window.clone())
                .await
        }
        .await;
        let current = self.reading_window(true);
        let mut state = lock(&self.inner.reading_areas);
        if state.serial != serial {
            return Err(error(
                "a newer reading or area selection replaced this request",
            ));
        }
        match result {
            Ok(capture)
                if !capture.text.trim().is_empty()
                    && current.as_ref().is_ok_and(|(_, w)| same_window(w, &window)) =>
            {
                state.change.accept_manual(&capture.text);
                state.pending = Some(Pending {
                    serial,
                    capture,
                    manual: true,
                    window,
                });
                state.phase = "queued";
                Ok(())
            }
            Ok(capture) => {
                let message = if current.is_err() {
                    current.err().map(|e| e.to_string()).unwrap_or_default()
                } else if capture.text.trim().is_empty() {
                    "no text was recognized in that area".into()
                } else {
                    "the game window moved while reading; read again".into()
                };
                state.phase = "error";
                state.message = Some(message.clone());
                Err(error(&message))
            }
            Err(e) => {
                state.phase = "error";
                state.message = Some(e.to_string());
                Err(e)
            }
        }
    }

    /// One inexpensive poll, with OCR only after a visual change or a periodic check.
    pub async fn observe_reading(&self) {
        self.reading_context();
        {
            let state = lock(&self.inner.reading_areas);
            if !state.automatic || state.held || state.picker.is_some() {
                return;
            }
        }
        let (area, window) = match self.reading_window(false) {
            Ok(pair) => pair,
            Err(e) => {
                let mut state = lock(&self.inner.reading_areas);
                if !state.suspended {
                    state.reset();
                    state.suspended = true;
                }
                state.phase = "paused";
                state.message = Some(e.to_string());
                return;
            }
        };
        let serial = {
            let mut state = lock(&self.inner.reading_areas);
            if state.suspended
                || state
                    .target
                    .as_ref()
                    .is_some_and(|w| !same_window(w, &window))
            {
                state.reset();
                state.suspended = false;
            }
            state.target = Some(window.clone());
            state.serial
        };
        let rect = area.rectangle(window.rect);
        let image = match self.reading_frame(rect).await {
            Ok(image) => image,
            Err(e) => {
                self.reading_observation_error(serial, e);
                return;
            }
        };
        let now = Instant::now();
        let next = signature(&image);
        let should_read = {
            let mut state = lock(&self.inner.reading_areas);
            if state.serial != serial || !state.automatic || state.held || state.picker.is_some() {
                return;
            }
            if state
                .frame
                .as_ref()
                .is_none_or(|old| changed_fraction(old, &next, 4) > 0.003)
            {
                state.frame_since = Some(now);
                state.dirty = true;
            }
            state.frame = Some(next);
            let elapsed = state.looked_at.map_or_else(
                || {
                    state.looked_at = Some(now);
                    Duration::ZERO
                },
                |at| now.duration_since(at),
            );
            let stable = state
                .frame_since
                .is_some_and(|at| now.duration_since(at) >= Duration::from_millis(650));
            let needs_check = state.change.waiting() || stable && state.dirty;
            if elapsed >= Duration::from_secs(2)
                || needs_check && elapsed >= Duration::from_millis(650)
            {
                state.looked_at = Some(now);
                state.dirty = false;
                true
            } else {
                false
            }
        };
        if !should_read {
            return;
        }
        let capture = match self.read_image(image, rect, window.clone()).await {
            Ok(capture) => capture,
            Err(e) => {
                self.reading_observation_error(serial, e);
                return;
            }
        };
        if !self
            .reading_window(false)
            .as_ref()
            .is_ok_and(|(_, w)| same_window(w, &window))
        {
            return;
        }
        let mut state = lock(&self.inner.reading_areas);
        if state.serial != serial || !state.automatic || state.held || state.picker.is_some() {
            return;
        }
        if let Some(text) = state.change.observe(&capture.text, Instant::now()) {
            state.serial = state.serial.wrapping_add(1);
            state.message = None;
            if text.is_empty() {
                state.pending = None;
                state.phase = "empty";
            } else {
                state.pending = Some(Pending {
                    serial: state.serial,
                    capture,
                    manual: false,
                    window,
                });
                state.phase = "queued";
            }
        } else if state.change.waiting() {
            state.phase = "settling";
        } else if state.active.is_none()
            && state.pending.is_none()
            && !matches!(state.phase, "error" | "empty")
        {
            state.phase = "watching";
        }
    }

    fn reading_observation_error(&self, serial: u64, e: EngineError) {
        let mut state = lock(&self.inner.reading_areas);
        if state.serial == serial {
            state.phase = "error";
            state.message = Some(e.to_string());
            state.looked_at = Some(Instant::now());
        }
    }

    pub fn take_reading_job(&self) -> Option<ReadingJob> {
        let foreground = self.inner.platform.screen.foreground_app()?;
        let mut state = lock(&self.inner.reading_areas);
        if state.active.is_some() || state.picker.is_some() {
            return None;
        }
        let pending = state.pending.as_ref()?;
        if !pending.manual && (!state.automatic || state.held) {
            return None;
        }
        if pending.capture.app.as_ref().is_none_or(|a| {
            !a.exe.eq_ignore_ascii_case(&foreground.exe) && !own_app(&foreground.exe)
        }) {
            return None;
        }
        if self
            .inner
            .platform
            .screen
            .app_window(&pending.window.app.exe)
            .is_none_or(|w| !same_window(&w, &pending.window))
        {
            state.reset();
            state.phase = "paused";
            state.message=Some("The game window moved or changed size before reading; read again or check its position.".into());
            return None;
        }
        let pending = state.pending.take()?;
        state.active = Some(pending.serial);
        state.phase = "reading";
        Some(ReadingJob {
            serial: pending.serial,
            capture: pending.capture,
        })
    }

    pub fn accepts_reading(&self, serial: u64) -> bool {
        let state = lock(&self.inner.reading_areas);
        state.active == Some(serial) && (state.serial == serial || state.retained == Some(serial))
    }

    pub fn finish_reading(&self, serial: u64, failure: Option<String>) {
        let mut state = lock(&self.inner.reading_areas);
        if state.active == Some(serial) {
            state.active = None;
        }
        if state.serial == serial || state.retained == Some(serial) {
            state.retained = None;
            state.phase = if failure.is_some() {
                "error"
            } else if state.automatic {
                "watching"
            } else {
                "ready"
            };
            state.message = failure;
        }
    }
}
