//! Exact OCR fixes belong to one captured program, never to the live watcher or a global rule.
use crate::engine::{Engine, EngineError, Inner, Result, lock};
use crate::{EventRx, labels::LabelCache, scan::Found};
use maru_core::{Capture, CaptureSource, Event, LabelSource, ScanItem};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    Snapshot { id: u64 },
    Scan { id: u64, item: u32 },
}

#[derive(Clone, Serialize)]
pub struct CorrectionView {
    pub target: Target,
    pub observed: String,
    pub text: String,
    pub program: String,
    pub remembered: bool,
}

#[derive(Clone)]
struct Snapshot {
    id: u64,
    original: Capture,
    capture: Capture,
}
#[derive(Default)]
pub(crate) struct ScanSnapshot {
    pub(crate) id: u64,
    pub(crate) scope: String,
    pub(crate) app: Option<maru_core::AppInfo>,
    pub(crate) items: HashMap<u32, ScanItem>,
    pub(crate) originals: HashMap<u32, String>,
    pub(crate) next_item: u32,
}
#[derive(Default)]
pub(crate) struct OcrState {
    snapshot: Option<Snapshot>,
    scan: ScanSnapshot,
}

pub(crate) fn init_store(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS ocr_corrections (
        scope TEXT NOT NULL, observed TEXT NOT NULL, corrected TEXT NOT NULL,
        PRIMARY KEY(scope, observed));",
    )
}

fn exact(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
fn validate(text: &str) -> std::result::Result<String, String> {
    let text = exact(text);
    if text.is_empty() || !text.chars().any(char::is_alphanumeric) {
        return Err("Enter the text you can read in the snapshot.".into());
    }
    if text.chars().count() > 4096 || text.chars().any(char::is_control) {
        return Err("Use at most 4,096 characters without control characters.".into());
    }
    Ok(text)
}

impl LabelCache {
    pub fn ocr_correction(&self, scope: &str, observed: &str) -> rusqlite::Result<Option<String>> {
        if scope.is_empty() {
            return Ok(None);
        }
        self.conn()
            .query_row(
                "SELECT corrected FROM ocr_corrections WHERE scope=?1 AND observed=?2",
                params![scope, exact(observed)],
                |r| r.get(0),
            )
            .optional()
    }
    pub fn remember_ocr_correction(
        &self,
        scope: &str,
        observed: &str,
        corrected: &str,
    ) -> rusqlite::Result<()> {
        self.conn().execute(
            "INSERT INTO ocr_corrections (scope,observed,corrected) VALUES (?1,?2,?3)
            ON CONFLICT(scope,observed) DO UPDATE SET corrected=excluded.corrected",
            params![scope, exact(observed), corrected],
        )?;
        Ok(())
    }
    pub fn forget_ocr_correction(&self, scope: &str, observed: &str) -> rusqlite::Result<()> {
        self.conn().execute(
            "DELETE FROM ocr_corrections WHERE scope=?1 AND observed=?2",
            params![scope, exact(observed)],
        )?;
        Ok(())
    }
}

fn saved(inner: &Inner, scope: &str, observed: &str) -> Option<String> {
    inner
        .labels
        .as_ref()
        .and_then(|c| match c.ocr_correction(scope, observed) {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(%error,"could not read OCR corrections");
                None
            }
        })
}

pub(crate) fn prepare_snapshot(inner: &Inner, id: u64, capture: &mut Capture) {
    let mut state = lock(&inner.ocr_corrections);
    if capture.source != CaptureSource::Ocr {
        state.snapshot = None;
        return;
    }
    let original = capture.clone();
    let scope = capture
        .app
        .as_ref()
        .map(|a| a.exe.as_str())
        .unwrap_or_default();
    if let Some(text) = saved(inner, scope, &capture.text) {
        capture.text = text;
        capture.words.clear();
        capture.confidence = None;
    }
    state.snapshot = Some(Snapshot {
        id,
        original,
        capture: capture.clone(),
    });
}

pub(crate) fn begin_scan(inner: &Inner, id: u64) {
    let app = inner
        .platform
        .screen
        .foreground_app();
    let scope = app.as_ref().map(|a| a.exe.clone()).unwrap_or_default();
    let mut state = lock(&inner.ocr_corrections);
    *lock(&inner.scan_scope) = (id, scope.clone());
    state.scan = ScanSnapshot {
        id,
        scope,
        app,
        ..Default::default()
    };
}

/// Joining, separation and correction share the same current-item check and lock.
pub(crate) fn with_scan<T>(inner: &Inner, id: u64, action: impl FnOnce(&mut ScanSnapshot) -> Result<T>) -> Result<T> {
    let mut state = lock(&inner.ocr_corrections);
    if state.scan.id != id {
        return Err(EngineError::Other("That scan is no longer current. Scan again before changing its labels.".into()));
    }
    action(&mut state.scan)
}

pub(crate) fn correct_found(inner: &Inner, scope: &str, found: &mut [Found]) {
    for item in found {
        if let Some(text) = saved(inner, scope, &item.text) {
            item.text = text;
        }
    }
}
pub(crate) fn reserve_scan_item(inner: &Inner, id: u64, item: u32) {
    let mut state = lock(&inner.ocr_corrections);
    if state.scan.id == id {
        state.scan.next_item = state.scan.next_item.max(item.saturating_add(1));
    }
}
pub(crate) fn register_items(
    inner: &Inner,
    id: u64,
    items: &[ScanItem],
    originals: &[Found],
    corrected: &[Found],
) {
    let mut state = lock(&inner.ocr_corrections);
    if state.scan.id != id {
        return;
    }
    for item in items {
        let observed = corrected
            .iter()
            .position(|f| f.rect == item.rect && f.text == item.text)
            .and_then(|i| originals.get(i))
            .map(|f| f.text.clone())
            .unwrap_or_else(|| item.text.clone());
        state.scan.next_item = state.scan.next_item.max(item.item.saturating_add(1));
        state.scan.originals.entry(item.item).or_insert(observed);
        state.scan.items.insert(item.item, item.clone());
    }
}

fn view(inner: &Inner, state: &OcrState, target: Target) -> Result<CorrectionView> {
    let (observed, text, program) = match target {
        Target::Snapshot { id } => {
            let s=state.snapshot.as_ref().filter(|s|s.id==id).ok_or_else(||EngineError::Other("That snapshot is no longer current. Open the correction from the current snapshot.".into()))?;
            (
                s.original.text.clone(),
                s.capture.text.clone(),
                s.original
                    .app
                    .as_ref()
                    .map(|a| a.exe.clone())
                    .unwrap_or_default(),
            )
        }
        Target::Scan { id, item } => {
            let s = &state.scan;
            let it = s.items.get(&item).filter(|_| s.id == id).ok_or_else(|| {
                EngineError::Other(
                    "That scan label is no longer current. Scan again before correcting it.".into(),
                )
            })?;
            (
                s.originals
                    .get(&item)
                    .cloned()
                    .unwrap_or_else(|| it.text.clone()),
                it.text.clone(),
                s.scope.clone(),
            )
        }
    };
    let remembered = saved(inner, &program, &observed).is_some();
    Ok(CorrectionView {
        target,
        observed,
        text,
        program,
        remembered,
    })
}

impl Engine {
    pub fn ocr_correction_view(&self, target: Target) -> Result<CorrectionView> {
        view(&self.inner, &lock(&self.inner.ocr_corrections), target)
    }

    fn save_correction(&self, v: &CorrectionView, text: &str, remember: bool) -> Result<()> {
        if !remember {
            return Ok(());
        }
        if v.program.is_empty() {
            return Err(EngineError::Other(
                "The snapshot has no captured program, so this correction can only apply once."
                    .into(),
            ));
        }
        let store =
            self.inner.labels.as_ref().ok_or_else(|| {
                EngineError::Other("The OCR correction store is unavailable.".into())
            })?;
        store
            .remember_ocr_correction(&v.program, &v.observed, text)
            .map_err(|e| EngineError::Other(e.to_string()))
    }

    pub fn forget_ocr_fix(&self, target: Target) -> Result<()> {
        let state = lock(&self.inner.ocr_corrections);
        let v = view(&self.inner, &state, target)?;
        if let Some(store) = &self.inner.labels {
            store
                .forget_ocr_correction(&v.program, &v.observed)
                .map_err(|e| EngineError::Other(e.to_string()))?;
        }
        Ok(())
    }

    pub fn correct_snapshot(
        &self,
        id: u64,
        text: &str,
        remember: bool,
    ) -> Result<(Capture, u64, EventRx)> {
        let text = validate(text).map_err(EngineError::Other)?;
        let mut state = lock(&self.inner.ocr_corrections);
        let v = view(&self.inner, &state, Target::Snapshot { id })?;
        self.save_correction(&v, &text, remember)?;
        let s = state
            .snapshot
            .as_mut()
            .ok_or_else(|| EngineError::Other("The snapshot is no longer current.".into()))?;
        let mut capture = s.capture.clone();
        capture.text = text;
        capture.words.clear();
        capture.confidence = None;
        let next = self.next_id();
        s.id = next;
        s.capture = capture.clone();
        drop(state);
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        self.spawn_into(
            &self.inner.translate_task,
            crate::pipeline::translate(self.inner.clone(), next, capture.clone(), tx),
        );
        Ok((capture, next, rx))
    }

    pub fn correct_scan(&self, id: u64, item: u32, text: &str, remember: bool) -> Result<EventRx> {
        let text = validate(text).map_err(EngineError::Other)?;
        let mut state = lock(&self.inner.ocr_corrections);
        let v = view(&self.inner, &state, Target::Scan { id, item })?;
        self.save_correction(&v, &text, remember)?;
        let next = state.scan.next_item;
        state.scan.next_item = next.checked_add(1).ok_or_else(|| {
            EngineError::Other("Too many corrections in this scan; scan again.".into())
        })?;
        let rect = state
            .scan
            .items
            .get(&item)
            .ok_or_else(|| EngineError::Other("The scan item disappeared.".into()))?
            .rect;
        let join = state.scan.items.get(&item).and_then(|i| i.join.clone());
        state.scan.items.insert(
            next,
            ScanItem {
                item: next,
                rect,
                text: text.clone(),
                label: String::new(),
                source: LabelSource::Pending,
                known: false,
                join: join.clone(),
            },
        );
        state.scan.originals.insert(next, v.observed);
        state.scan.items.remove(&item);
        state.scan.originals.remove(&item);
        let scope = state.scan.scope.clone();
        drop(state);
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let _ = tx.send(Event::ScanCorrection {
            id,
            item,
            replacement: next,
        });
        let inner = self.inner.clone();
        self.inner.runtime.spawn(async move {
            let em = crate::pipeline::Emitter::new(tx, id);
            crate::scan::label(
                &inner,
                &em,
                crate::scan::LabelInput::Joined { items: vec![crate::scan_joining::JoinedFound::new(Found { rect, text }, join)], correct_ocr: false },
                next,
                &scope,
                std::time::Instant::now(),
            )
            .await;
            em.done();
        });
        Ok(rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_corrections_are_isolated_to_the_program_and_can_be_forgotten() {
        let c = LabelCache::open_in_memory().unwrap();
        c.remember_ocr_correction("game.exe", "ㅅ컵", "스킵")
            .unwrap();
        assert_eq!(
            c.ocr_correction("game.exe", " ㅅ컵 ").unwrap(),
            Some("스킵".into())
        );
        assert_eq!(c.ocr_correction("other.exe", "ㅅ컵").unwrap(), None);
        assert_eq!(c.ocr_correction("game.exe", "ㅅ컵하기").unwrap(), None);
        c.forget_ocr_correction("game.exe", "ㅅ컵").unwrap();
        assert_eq!(c.ocr_correction("game.exe", "ㅅ컵").unwrap(), None);
    }
}
