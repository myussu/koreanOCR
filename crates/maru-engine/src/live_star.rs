//! Starring a live subtitle line from the game, to study it later. The line meant is the one on screen, which may
//! still be translating, may be a re-reading of an earlier line, or may later be replaced by a fuller reading.

use std::collections::HashSet;

use crate::engine::{Engine, EngineError, Inner, Result, lock};
use crate::memory::HistoryItem;

/// Lines starred while they were still being translated: the star is applied when they reach the history.
#[derive(Default)]
pub(crate) struct PendingStars(HashSet<u64>);

/// What a star did, for the band's confirmation.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Starred {
    /// The live line id the band knows the line by.
    pub id: u64,
    pub starred: bool,
}

impl Engine {
    /// Star (or un-star) the live line the band is showing (`shown`, its live id; 0 = none this session). A line still
    /// being translated is starred as soon as it arrives; without one on screen, the newest live line is meant.
    pub fn star_live_line(&self, shown: u64) -> Result<Starred> {
        let inner = &self.inner;
        let target = lock(&inner.live_recent).original(shown).unwrap_or(shown);
        let mut history = lock(&inner.history);
        let id = if target != 0 && !history.has_live(target) && target > history.newest_live_id().unwrap_or(0) {
            // Still translating: remember the wish (pressing again cancels it).
            let mut pending = lock(&inner.pending_stars);
            let starred = pending.0.insert(target) || { pending.0.remove(&target); false };
            return Ok(Starred { id: shown, starred });
        } else if target != 0 && history.has_live(target) {
            target
        } else {
            history.newest_live_id().ok_or_else(|| EngineError::Other("No live subtitle line to star yet.".into()))?
        };
        let starred = history.toggle_star(id).unwrap_or(false);
        let row = history.saved_id_of(id);
        // Recording a line locks the store before the history: never hold the history while waiting for the store.
        drop(history);
        if let Some(row) = row {
            save_star(inner, row, starred);
        }
        Ok(Starred { id: if shown == 0 { id } else { shown }, starred })
    }

    /// Star or un-star a saved live line from the History page.
    pub fn set_line_star(&self, saved_id: i64, starred: bool) -> Result<()> {
        // Same lock order as recording a line: the store first, then the history.
        let store = lock(&self.inner.history_store);
        lock(&self.inner.history).set_star_saved(saved_id, starred);
        let store = store.as_ref().ok_or_else(|| EngineError::Other("The live history is not saved on this PC.".into()))?;
        store.set_starred(saved_id, starred)
    }

    /// Every starred live line in `history.db`, oldest first: they stay there after they leave the recent history.
    pub fn starred_lines(&self) -> Result<Vec<HistoryItem>> {
        match lock(&self.inner.history_store).as_ref() {
            Some(store) => store.starred(),
            None => Ok(lock(&self.inner.history).items().into_iter().filter(|i| i.live && i.starred).collect()),
        }
    }
}

fn save_star(inner: &Inner, row: i64, starred: bool) {
    if let Some(store) = lock(&inner.history_store).as_ref()
        && let Err(e) = store.set_starred(row, starred)
    {
        tracing::warn!("{e}");
    }
}

/// A line reaching the history keeps a star given while it was translating, or to the partial reading it replaces.
pub(crate) fn carry_star(inner: &Inner, item: &mut HistoryItem, replaced_starred: bool) {
    if lock(&inner.pending_stars).0.remove(&item.id) || replaced_starred {
        item.starred = true;
    }
}
