//! Selected text through UI Automation.
//!
//! `TextPattern` gives the exact characters and their on-screen rectangles, so
//! no OCR is needed for the large majority of apps (browsers, editors, Office,
//! PDF viewers). Apps that do not expose it fall through to the clipboard and
//! OCR paths in the engine.

use std::ffi::c_void;
use std::time::Duration;

use maru_core::{CaptureSource, Point, Rect};
use windows::Win32::Foundation::POINT;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::Ole::{SafeArrayAccessData, SafeArrayDestroy, SafeArrayGetLBound, SafeArrayGetUBound, SafeArrayUnaccessData};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern, TextUnit_Word, UIA_TextPatternId,
};

use super::com::ComWorker;
use crate::{Result, Selection};

const CALL_TIMEOUT: Duration = Duration::from_millis(1500);
/// How many ancestors to try when the focused element itself has no text pattern.
const MAX_ANCESTORS: usize = 4;

pub struct Uia {
    worker: ComWorker<IUIAutomation>,
}

impl Uia {
    pub fn start() -> Result<Self> {
        let worker = ComWorker::start("maru-uia", || {
            // SAFETY: creating the in-process UI Automation client on its own MTA thread.
            unsafe { CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }
        })?;
        Ok(Self { worker })
    }

    pub fn selection(&self) -> Result<Option<Selection>> {
        self.worker.call(CALL_TIMEOUT, |uia| {
            // SAFETY: COM calls on the thread that owns `uia`.
            unsafe { read_selection(uia) }
        })?
    }

    pub fn word_at(&self, p: Point) -> Result<Option<Selection>> {
        self.worker.call(CALL_TIMEOUT, move |uia| {
            // SAFETY: as above.
            unsafe { read_word_at(uia, p) }
        })?
    }
}

unsafe fn text_pattern(el: &IUIAutomationElement) -> Option<IUIAutomationTextPattern> {
    // SAFETY: caller guarantees COM is initialised on this thread.
    unsafe { el.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId).ok() }
}

/// Try the element, then a few ancestors (focus often sits on a child of the document).
unsafe fn pattern_near(uia: &IUIAutomation, start: IUIAutomationElement) -> Option<IUIAutomationTextPattern> {
    // SAFETY: caller guarantees COM is initialised on this thread.
    unsafe {
        if let Some(p) = text_pattern(&start) {
            return Some(p);
        }
        let walker = uia.ControlViewWalker().ok()?;
        let mut current = start;
        for _ in 0..MAX_ANCESTORS {
            current = walker.GetParentElement(&current).ok()?;
            if let Some(p) = text_pattern(&current) {
                return Some(p);
            }
        }
        None
    }
}

unsafe fn read_selection(uia: &IUIAutomation) -> Result<Option<Selection>> {
    // SAFETY: caller guarantees COM is initialised on this thread.
    unsafe {
        let Ok(focused) = uia.GetFocusedElement() else { return Ok(None) };
        let Some(pattern) = pattern_near(uia, focused) else { return Ok(None) };
        let Ok(ranges) = pattern.GetSelection() else { return Ok(None) };
        let count = ranges.Length().unwrap_or(0);

        let mut text = String::new();
        let mut rect: Option<Rect> = None;
        for i in 0..count {
            let Ok(range) = ranges.GetElement(i) else { continue };
            if let Ok(t) = range.GetText(-1) {
                text.push_str(&t.to_string());
            }
            if let Ok(sa) = range.GetBoundingRectangles() {
                for r in rects_from_safearray(sa) {
                    rect = Some(rect.map_or(r, |acc| acc.union(&r)));
                }
            }
        }
        Ok(finish(text, rect))
    }
}

unsafe fn read_word_at(uia: &IUIAutomation, p: Point) -> Result<Option<Selection>> {
    // SAFETY: caller guarantees COM is initialised on this thread.
    unsafe {
        let pt = POINT { x: p.x.round() as i32, y: p.y.round() as i32 };
        let Ok(el) = uia.ElementFromPoint(pt) else { return Ok(None) };
        let Some(pattern) = pattern_near(uia, el) else { return Ok(None) };
        let Ok(range) = pattern.RangeFromPoint(pt) else { return Ok(None) };
        let _ = range.ExpandToEnclosingUnit(TextUnit_Word);
        let text = range.GetText(-1).map(|t| t.to_string()).unwrap_or_default();
        let mut rect: Option<Rect> = None;
        if let Ok(sa) = range.GetBoundingRectangles() {
            for r in rects_from_safearray(sa) {
                rect = Some(rect.map_or(r, |acc| acc.union(&r)));
            }
        }
        Ok(finish(text, rect).map(|mut s| {
            s.source = CaptureSource::Hover;
            s
        }))
    }
}

fn finish(text: String, rect: Option<Rect>) -> Option<Selection> {
    let text = text.trim().to_string();
    (!text.is_empty()).then_some(Selection { text, rect, source: CaptureSource::Accessibility })
}

/// UIA returns bounding rectangles as a SAFEARRAY of doubles: left, top, width, height per line.
unsafe fn rects_from_safearray(sa: *mut windows::Win32::System::Com::SAFEARRAY) -> Vec<Rect> {
    // SAFETY: `sa` comes from UIA and is destroyed exactly once here.
    unsafe {
        if sa.is_null() {
            return Vec::new();
        }
        let lower = SafeArrayGetLBound(sa, 1).unwrap_or(0);
        let upper = SafeArrayGetUBound(sa, 1).unwrap_or(-1);
        let count = (upper - lower + 1).max(0) as usize;
        let mut rects = Vec::new();
        let mut data: *mut c_void = std::ptr::null_mut();
        if count >= 4 && SafeArrayAccessData(sa, &mut data).is_ok() && !data.is_null() {
            let values = std::slice::from_raw_parts(data as *const f64, count);
            rects.extend(values.chunks_exact(4).map(|c| Rect::new(c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32)));
            let _ = SafeArrayUnaccessData(sa);
        }
        let _ = SafeArrayDestroy(sa);
        rects
    }
}
