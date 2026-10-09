//! Offline dictionary.
//!
//! The data is *not* bundled: `import_kaikki` ingests a Wiktionary extract
//! (JSON lines, one entry per line, as published by kaikki.org / wiktextract)
//! and `import_levels` ingests a `lemma<TAB>level` list such as the NIKL
//! learner vocabulary. Everything lands in one SQLite file that is opened
//! read-mostly at run time.

mod import;
mod lookup;
mod schema;

use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;
use thiserror::Error;

pub use import::{ImportStats, normalize_level};
pub use lookup::{lookup_keys, short_gloss};

#[derive(Debug, Error)]
pub enum DictError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T, E = DictError> = std::result::Result<T, E>;

pub struct Dictionary {
    conn: Mutex<Connection>,
}

impl Dictionary {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        schema::init(&conn)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        schema::init(&conn)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub(crate) fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn entry_count(&self) -> Result<u64> {
        let n: i64 = self.conn().query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0))?;
        Ok(n as u64)
    }

    pub fn is_empty(&self) -> bool {
        self.entry_count().map(|n| n == 0).unwrap_or(true)
    }
}
