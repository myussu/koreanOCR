//! Remembered screen-scan labels.
//!
//! Game menus repeat: the same button, the same tab, the same tooltip, every session. Once the
//! model has labelled a string it never needs to again, so a second visit to a screen is
//! labelled instantly, with no model call at all.
//!
//! Entries are keyed by the *program* they were seen in plus the text, so one game's "상점" can
//! be "Shop" and another's "Store". A string never seen in this program falls back to whatever
//! any program called it, because the common ones (확인, 취소, 설정) mean the same everywhere.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use rusqlite::{Connection, params};

pub struct LabelCache {
    conn: Mutex<Connection>,
}

/// The cache key for a string: whitespace collapsed, edge punctuation dropped, Latin lower-cased.
/// OCR often adds or drops a stray `:`, `.` or space at the edges; none of that changes the label.
pub fn key(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim_matches(|c: char| !c.is_alphanumeric() && !maru_lang::hangul::is_hangul(c));
    trimmed.chars().flat_map(char::to_lowercase).collect()
}

impl LabelCache {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS labels (
                 scope TEXT NOT NULL,
                 text TEXT NOT NULL,
                 label TEXT NOT NULL,
                 hits INTEGER NOT NULL DEFAULT 1,
                 updated INTEGER NOT NULL,
                 PRIMARY KEY (scope, text)
             );
             CREATE INDEX IF NOT EXISTS labels_by_text ON labels (text);",
        )?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Labels for `texts` (keyed by [`key`]); texts never seen are simply absent.
    pub fn get_many(&self, scope: &str, texts: &[&str]) -> HashMap<String, String> {
        let conn = self.conn();
        let Ok(mut stmt) = conn.prepare_cached("SELECT label FROM labels WHERE text = ?1 ORDER BY (scope = ?2) DESC, hits DESC, updated DESC LIMIT 1") else {
            return HashMap::new();
        };
        texts
            .iter()
            .filter_map(|text| {
                let k = key(text);
                let label: Option<String> = stmt.query_row(params![k, scope], |r| r.get(0)).ok();
                label.map(|l| (k, l))
            })
            .collect()
    }

    /// Remember `(text, label)` pairs for `scope`. Empty labels and `?` (not text) are ignored.
    pub fn put_many(&self, scope: &str, entries: &[(String, String)], now: i64) {
        let mut conn = self.conn();
        let Ok(tx) = conn.transaction() else { return };
        {
            let Ok(mut stmt) = tx.prepare_cached(
                "INSERT INTO labels (scope, text, label, hits, updated) VALUES (?1, ?2, ?3, 1, ?4)
                 ON CONFLICT (scope, text) DO UPDATE SET label = excluded.label, hits = hits + 1, updated = excluded.updated",
            ) else {
                return;
            };
            for (text, label) in entries {
                let (k, label) = (key(text), label.trim());
                if k.is_empty() || label.is_empty() || label == "?" {
                    continue;
                }
                if let Err(e) = stmt.execute(params![scope, k, label, now]) {
                    tracing::warn!("could not remember label for {k}: {e}");
                }
            }
        }
        if let Err(e) = tx.commit() {
            tracing::warn!("could not save labels: {e}");
        }
    }

    pub fn len(&self) -> usize {
        self.conn().query_row("SELECT COUNT(*) FROM labels", [], |r| r.get::<_, i64>(0)).map(|n| n as usize).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    #[test]
    fn keys_ignore_edge_punctuation_spacing_and_case() {
        assert_eq!(key("  인벤토리: "), "인벤토리");
        assert_eq!(key("HP  최대"), "hp 최대");
        assert_eq!(key("[확인]"), "확인");
        assert_eq!(key("..."), "");
    }

    #[test]
    fn remembered_labels_come_back_for_slightly_different_ocr() {
        let c = LabelCache::open_in_memory().unwrap();
        c.put_many("game.exe", &pairs(&[("인벤토리", "Inventory"), ("확인", "OK")]), 1);
        let got = c.get_many("game.exe", &["인벤토리 :", "확인", "처음 보는 글자"]);
        assert_eq!(got.get("인벤토리").map(String::as_str), Some("Inventory"));
        assert_eq!(got.get("확인").map(String::as_str), Some("OK"));
        assert_eq!(got.len(), 2, "unknown strings are absent");
    }

    #[test]
    fn this_programs_wording_wins_but_other_programs_fill_the_gaps() {
        let c = LabelCache::open_in_memory().unwrap();
        c.put_many("farm.exe", &pairs(&[("상점", "Shop")]), 1);
        c.put_many("rpg.exe", &pairs(&[("상점", "Store"), ("취소", "Cancel")]), 2);
        assert_eq!(c.get_many("farm.exe", &["상점"])["상점"], "Shop");
        assert_eq!(c.get_many("rpg.exe", &["상점"])["상점"], "Store");
        assert_eq!(c.get_many("farm.exe", &["취소"])["취소"], "Cancel", "a string only seen elsewhere is still reused");
    }

    #[test]
    fn junk_is_not_remembered_and_updates_replace_the_label() {
        let c = LabelCache::open_in_memory().unwrap();
        c.put_many("g", &pairs(&[("ㅁㄴㅇ", "?"), ("빈칸", "  "), ("...", "x"), ("저장", "Save")]), 1);
        assert_eq!(c.len(), 1);
        c.put_many("g", &pairs(&[("저장", "Save game")]), 2);
        assert_eq!(c.get_many("g", &["저장"])["저장"], "Save game");
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn labels_survive_a_restart() {
        let path = std::env::temp_dir().join(format!("maru-labels-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        {
            let c = LabelCache::open(&path).unwrap();
            c.put_many("g", &pairs(&[("옵션", "Options")]), 1);
        }
        let again = LabelCache::open(&path).unwrap();
        assert_eq!(again.get_many("g", &["옵션"])["옵션"], "Options");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn empty_batches_are_fine() {
        let c = LabelCache::open_in_memory().unwrap();
        c.put_many("g", &[], 1);
        assert!(c.get_many("g", &[]).is_empty());
        assert!(c.is_empty());
    }
}
