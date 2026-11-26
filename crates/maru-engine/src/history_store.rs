//! Live subtitle lines saved on disk (`history.db`), so a cutscene can be read again after Maru restarts.

use std::path::Path;

use maru_core::Direction;
use rusqlite::{Connection, params};

use crate::engine::{Engine, EngineError, Result, lock};
use crate::memory::HistoryItem;

pub(crate) struct HistoryStore {
    conn: Connection,
    /// When this Maru run started: lines of one run belong together.
    session: i64,
}

fn err(e: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("Live history: {e}"))
}

impl HistoryStore {
    pub(crate) fn open(path: &Path, session: i64) -> Result<Self> {
        let conn = Connection::open(path).map_err(err)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS live_lines (
            id INTEGER PRIMARY KEY AUTOINCREMENT, session INTEGER NOT NULL, at INTEGER NOT NULL,
            appeared_at INTEGER, app TEXT, source TEXT NOT NULL, translation TEXT NOT NULL,
            first_ms INTEGER, total_ms INTEGER, starred INTEGER NOT NULL DEFAULT 0, mined INTEGER NOT NULL DEFAULT 0);",
        )
        .map_err(err)?;
        // `mined` came after the first saved histories: add it to them.
        let has_mined: bool = conn
            .query_row("SELECT COUNT(*) FROM pragma_table_info('live_lines') WHERE name = 'mined'", [], |r| r.get::<_, i64>(0))
            .map_err(err)?
            > 0;
        if !has_mined {
            conn.execute_batch("ALTER TABLE live_lines ADD COLUMN mined INTEGER NOT NULL DEFAULT 0;").map_err(err)?;
        }
        Ok(Self { conn, session })
    }

    pub(crate) fn insert(&self, item: &HistoryItem) -> Result<i64> {
        self.conn
            .execute(
                "INSERT INTO live_lines (session, at, appeared_at, app, source, translation, first_ms, total_ms, starred)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![self.session, item.at, item.appeared_at, item.app, item.source, item.translation, item.first_ms, item.total_ms, item.starred],
            )
            .map_err(err)?;
        Ok(self.conn.last_insert_rowid())
    }

    /// A fuller reading takes over the row of the reading it replaces.
    pub(crate) fn update(&self, row: i64, item: &HistoryItem) -> Result<()> {
        self.conn
            .execute(
                "UPDATE live_lines SET at = ?2, appeared_at = ?3, source = ?4, translation = ?5, first_ms = ?6, total_ms = ?7, starred = ?8 WHERE id = ?1",
                params![row, item.at, item.appeared_at, item.source, item.translation, item.first_ms, item.total_ms, item.starred],
            )
            .map_err(err)?;
        Ok(())
    }

    pub(crate) fn set_mined(&self, row: i64) -> Result<()> {
        self.conn.execute("UPDATE live_lines SET mined = 1 WHERE id = ?1", [row]).map_err(err)?;
        Ok(())
    }

    pub(crate) fn set_starred(&self, row: i64, starred: bool) -> Result<()> {
        self.conn.execute("UPDATE live_lines SET starred = ?2 WHERE id = ?1", params![row, starred]).map_err(err)?;
        Ok(())
    }

    /// The newest `limit` lines, oldest first.
    pub(crate) fn recent(&self, limit: usize) -> Result<Vec<HistoryItem>> {
        let mut rows = self.query("ORDER BY id DESC LIMIT ?1", limit as i64)?;
        rows.reverse();
        Ok(rows)
    }

    /// Every starred line, oldest first.
    pub(crate) fn starred(&self) -> Result<Vec<HistoryItem>> {
        self.query("WHERE starred = ?1 ORDER BY id", 1)
    }

    fn query(&self, tail: &str, arg: i64) -> Result<Vec<HistoryItem>> {
        let sql = format!("SELECT id, at, appeared_at, app, source, translation, first_ms, total_ms, starred, mined FROM live_lines {tail}");
        let mut stmt = self.conn.prepare(&sql).map_err(err)?;
        stmt.query_map([arg], |r| {
            Ok(HistoryItem {
                live: true,
                appeared_at: r.get(2)?,
                first_ms: r.get(6)?,
                total_ms: r.get(7)?,
                saved_id: Some(r.get(0)?),
                starred: r.get(8)?,
                mined: r.get(9)?,
                ..HistoryItem::new(0, r.get(1)?, r.get(4)?, r.get(5)?, Direction::KoToEn, r.get(3)?)
            })
        })
        .map_err(err)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(err)
    }
}

impl Engine {
    /// Mark a saved live line as made into cards.
    pub fn mark_mined(&self, saved_id: i64) -> Result<()> {
        let store = lock(&self.inner.history_store);
        lock(&self.inner.history).set_mined_saved(saved_id);
        match store.as_ref() {
            Some(store) => store.set_mined(saved_id),
            None => Ok(()),
        }
    }

    /// Open the saved live history and show its newest lines, as if they had just been read.
    pub fn open_history(&self, path: &Path) -> Result<()> {
        let store = HistoryStore::open(path, maru_srs::now_secs())?;
        let lines = store.recent(self.inner.cfg.continuous.history_len)?;
        lock(&self.inner.history).restore(lines);
        *lock(&self.inner.history_store) = Some(store);
        Ok(())
    }
}

/// Add a live line to the history (or let it replace the partial reading it grew from) and save it.
pub(crate) fn record(inner: &crate::engine::Inner, mut item: HistoryItem, replaces: Option<u64>) {
    let store = lock(&inner.history_store);
    let mut history = lock(&inner.history);
    let previous = replaces.and_then(|id| history.saved_id_of(id));
    crate::live_star::carry_star(inner, &mut item, replaces.is_some_and(|id| history.is_starred(id)));
    if let Some(store) = store.as_ref() {
        let saved = match previous {
            Some(row) => store.update(row, &item).map(|()| row),
            None => store.insert(&item),
        };
        match saved {
            Ok(row) => item.saved_id = Some(row),
            Err(e) => tracing::warn!("{e}"),
        }
    }
    match replaces {
        Some(id) => history.replace(id, item),
        None => history.push(item),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(source: &str, translation: &str) -> HistoryItem {
        HistoryItem { live: true, first_ms: Some(300), total_ms: Some(700), ..HistoryItem::new(7, 1_760_000_000, source.into(), translation.into(), Direction::KoToEn, Some("game.exe".into())) }
    }

    #[test]
    fn saved_lines_come_back_in_order_and_a_fuller_reading_takes_over_its_row() {
        let path = std::env::temp_dir().join(format!("maru-history-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let store = HistoryStore::open(&path, 1).unwrap();
            let first = store.insert(&line("이걸 타면 위로", "Ride this up")).unwrap();
            store.insert(&line("왜 아직도 여기 있는 거야?", "Why are you still here?")).unwrap();
            store.update(first, &line("이걸 타면 위로 갈 수 있어요.", "You can go up if you ride this.")).unwrap();
        }
        let store = HistoryStore::open(&path, 2).unwrap();
        let lines = store.recent(10).unwrap();
        assert_eq!(lines.iter().map(|l| l.source.as_str()).collect::<Vec<_>>(), ["이걸 타면 위로 갈 수 있어요.", "왜 아직도 여기 있는 거야?"]);
        assert!(lines.iter().all(|l| l.live && l.saved_id.is_some() && l.app.as_deref() == Some("game.exe")));
        assert_eq!(store.recent(1).unwrap()[0].source, "왜 아직도 여기 있는 거야?", "the newest lines are kept");
        store.set_starred(lines[0].saved_id.unwrap(), true).unwrap();
        assert_eq!(store.starred().unwrap().iter().map(|l| l.source.as_str()).collect::<Vec<_>>(), ["이걸 타면 위로 갈 수 있어요."]);
        assert!(store.recent(10).unwrap()[0].starred);
        store.set_mined(lines[0].saved_id.unwrap()).unwrap();
        assert!(store.recent(10).unwrap()[0].mined && !store.recent(10).unwrap()[1].mined);
        drop(store);
        let _ = std::fs::remove_file(path);
    }
}
