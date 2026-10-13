//! Anki membership and incomplete adds are separate from the local review queue.
use crate::{Result, Vocab};
use maru_core::config::ReverseMode;
use rusqlite::{Connection, OptionalExtension, params};

pub(crate) fn init(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS anki_membership (key TEXT PRIMARY KEY);
        CREATE TABLE IF NOT EXISTS anki_pending_adds (
            key TEXT NOT NULL, endpoint TEXT NOT NULL, note_type TEXT NOT NULL,
            note_id INTEGER NOT NULL, deck TEXT NOT NULL, reverse TEXT NOT NULL, app TEXT,
            PRIMARY KEY(key, endpoint, note_type));",
    )
}

#[derive(Debug, Clone)]
pub struct PendingAdd {
    pub key: String,
    pub endpoint: String,
    pub note_type: String,
    pub note_id: u64,
    pub deck: String,
    pub reverse: ReverseMode,
    pub app: Option<String>,
}

impl Vocab {
    /// Reconcile only after every Anki page was read successfully. Local saves and overrides survive.
    pub fn replace_anki_progress(&self, states: &[(String, Option<f32>)], now: i64) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE words SET anki_interval = NULL WHERE anki_interval IS NOT NULL",
            [],
        )?;
        tx.execute("DELETE FROM anki_membership", [])?;
        for (key, interval) in states {
            tx.execute("INSERT OR IGNORE INTO words (key, surface, first_seen, last_seen) VALUES (?1, ?1, ?2, ?2)", params![key, now])?;
            tx.execute(
                "UPDATE words SET anki_interval = ?2, saved = 1 WHERE key = ?1",
                params![key, interval],
            )?;
            tx.execute(
                "INSERT OR IGNORE INTO anki_membership (key) VALUES (?1)",
                [key],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn remember_anki_membership(&self, key: &str) -> Result<()> {
        self.conn().execute(
            "INSERT OR IGNORE INTO anki_membership (key) VALUES (?1)",
            [key],
        )?;
        Ok(())
    }

    pub fn anki_count(&self) -> Result<u32> {
        Ok(self
            .conn()
            .query_row("SELECT COUNT(*) FROM anki_membership", [], |r| r.get(0))?)
    }

    pub fn remember_pending_add(&self, add: &PendingAdd) -> Result<()> {
        let note_id = i64::try_from(add.note_id)
            .map_err(|_| crate::SrsError::Other("Anki note ID is out of range".into()))?;
        let reverse = serde_json::to_string(&add.reverse)
            .map_err(|e| crate::SrsError::Other(e.to_string()))?;
        self.conn().execute("INSERT INTO anki_pending_adds (key, endpoint, note_type, note_id, deck, reverse, app)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT(key, endpoint, note_type) DO UPDATE SET
            note_id = excluded.note_id, deck = excluded.deck, reverse = excluded.reverse, app = excluded.app",
            params![add.key, add.endpoint, add.note_type, note_id, add.deck, reverse, add.app])?;
        Ok(())
    }

    pub fn pending_add(
        &self,
        key: &str,
        endpoint: &str,
        note_type: &str,
    ) -> Result<Option<PendingAdd>> {
        let row = self
            .conn()
            .query_row(
                "SELECT note_id, deck, reverse, app FROM anki_pending_adds
            WHERE key = ?1 AND endpoint = ?2 AND note_type = ?3",
                params![key, endpoint, note_type],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .optional()?;
        row.map(|(note_id, deck, reverse, app)| {
            Ok(PendingAdd {
                key: key.into(),
                endpoint: endpoint.into(),
                note_type: note_type.into(),
                note_id: u64::try_from(note_id)
                    .map_err(|_| crate::SrsError::Other("Anki note ID is invalid".into()))?,
                deck,
                app,
                reverse: serde_json::from_str(&reverse)
                    .map_err(|e| crate::SrsError::Other(e.to_string()))?,
            })
        })
        .transpose()
    }

    pub fn finish_pending_add(&self, key: &str, endpoint: &str, note_type: &str) -> Result<()> {
        self.conn().execute(
            "DELETE FROM anki_pending_adds WHERE key = ?1 AND endpoint = ?2 AND note_type = ?3",
            params![key, endpoint, note_type],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LookupInfo, SrsConfig};
    use maru_core::WordState;

    #[test]
    fn incomplete_adds_survive_reopening_and_are_isolated_to_the_server_and_note_type() {
        let file = std::env::temp_dir().join(format!(
            "maru-anki-pending-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let v = Vocab::open(&file, SrsConfig::default()).unwrap();
        v.remember_pending_add(&PendingAdd {
            key: "학교".into(),
            endpoint: "http://local".into(),
            note_type: "Maru Korean".into(),
            note_id: 77,
            deck: "Korean::Game".into(),
            reverse: ReverseMode::Subdeck,
            app: Some("game.exe".into()),
        })
        .unwrap();
        drop(v);
        let v = Vocab::open(&file, SrsConfig::default()).unwrap();
        let pending = v
            .pending_add("학교", "http://local", "Maru Korean")
            .unwrap()
            .unwrap();
        assert_eq!(pending.note_id, 77);
        assert_eq!(pending.reverse, ReverseMode::Subdeck);
        assert!(
            v.pending_add("학교", "http://other", "Maru Korean")
                .unwrap()
                .is_none()
        );
        assert!(
            v.pending_add("학교", "http://local", "Other")
                .unwrap()
                .is_none()
        );
        v.finish_pending_add("학교", "http://local", "Maru Korean")
            .unwrap();
        assert!(
            v.pending_add("학교", "http://local", "Maru Korean")
                .unwrap()
                .is_none()
        );
        drop(v);
        std::fs::remove_file(file).unwrap();
    }

    #[test]
    fn missing_and_suspended_cards_clear_old_progress_without_removing_local_learning() {
        let v = Vocab::open_in_memory(SrsConfig::default()).unwrap();
        for key in ["deleted", "suspended", "manual"] {
            v.set_anki_state(key, Some(45.0), 1).unwrap();
        }
        v.mark_known("manual", true, 1).unwrap();
        v.replace_anki_progress(
            &[("suspended".into(), None), ("manual".into(), Some(0.0))],
            2,
        )
        .unwrap();
        assert_eq!(
            v.get("deleted").unwrap().unwrap().state,
            WordState::Learning
        );
        assert!(v.get("deleted").unwrap().unwrap().saved);
        assert_eq!(v.get("suspended").unwrap().unwrap().anki_interval, None);
        assert_eq!(v.get("manual").unwrap().unwrap().state, WordState::Known);
        assert_eq!(v.anki_count().unwrap(), 2);
    }

    #[test]
    fn a_word_in_the_local_review_queue_is_not_counted_as_an_anki_note() {
        let v = Vocab::open_in_memory(SrsConfig::default()).unwrap();
        let info = LookupInfo {
            key: "학교".into(),
            surface: "학교".into(),
            ..Default::default()
        };
        v.record_lookup(&info, 1).unwrap();
        v.record_lookup(&info, 2).unwrap();
        assert_eq!(v.stats(2).unwrap().saved, 1);
        assert_eq!(v.anki_count().unwrap(), 0);
    }
}
