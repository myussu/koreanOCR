//! Read the selection by copying it, then put the clipboard back.
//!
//! This is the universal fallback (works in anything that supports Ctrl+C) at
//! the cost of briefly touching the clipboard. Content we cannot faithfully
//! restore (files, rich formats) makes us refuse rather than destroy it.

use std::time::{Duration, Instant};

use arboard::{Clipboard, ImageData};
use maru_core::CaptureSource;
use windows::Win32::System::DataExchange::{CountClipboardFormats, GetClipboardSequenceNumber};

use super::{fail, keys};
use crate::{Result, Selection};

enum Saved {
    Text(String),
    Image(ImageData<'static>),
    Empty,
}

fn save(cb: &mut Clipboard) -> Option<Saved> {
    if let Ok(t) = cb.get_text() {
        return Some(Saved::Text(t));
    }
    if let Ok(img) = cb.get_image() {
        return Some(Saved::Image(ImageData { width: img.width, height: img.height, bytes: img.bytes.into_owned().into() }));
    }
    // SAFETY: no preconditions.
    let formats = unsafe { CountClipboardFormats() };
    if formats == 0 { Some(Saved::Empty) } else { None }
}

fn restore(cb: &mut Clipboard, saved: Saved) {
    let _ = match saved {
        Saved::Text(t) => cb.set_text(t),
        Saved::Image(img) => cb.set_image(img),
        Saved::Empty => cb.clear(),
    };
}

pub fn copy_selection(timeout: Duration) -> Result<Option<Selection>> {
    let mut cb = Clipboard::new().map_err(|e| fail("clipboard", e))?;
    let Some(saved) = save(&mut cb) else {
        tracing::debug!("clipboard holds content we cannot restore; skipping copy");
        return Ok(None);
    };

    // SAFETY: no preconditions.
    let before = unsafe { GetClipboardSequenceNumber() };
    keys::send_ctrl_c();

    let deadline = Instant::now() + timeout;
    let mut changed = false;
    while Instant::now() < deadline {
        // SAFETY: no preconditions.
        if unsafe { GetClipboardSequenceNumber() } != before {
            changed = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let text = if changed { cb.get_text().ok() } else { None };
    restore(&mut cb, saved);

    Ok(text.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).map(|text| Selection {
        text,
        rect: None,
        source: CaptureSource::Clipboard,
    }))
}
