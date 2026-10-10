//! Vocabulary store: what you looked up, in which sentence, and how well you
//! remember it. One SQLite file; every method is a handful of indexed queries.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use maru_core::WordState;
use rusqlite::{Connection, OptionalExtension, params};

use crate::fsrs::{self, Memory, Params, Rating};
use crate::{Result, SrsError};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SrsConfig {
    /// A word counts as known once stability reaches this many days.
    pub known_stability_days: f32,
    pub desired_retention: f32,
    /// A lookup count at which a word is queued for review automatically (0 = never).
    pub auto_save_after_lookups: u32,
    pub params: Params,
}

impl Default for SrsConfig {
    fn default() -> Self {
        Self {
            known_stability_days: 21.0,
            desired_retention: 0.90,
            auto_save_after_lookups: 2,
            params: fsrs::DEFAULT_PARAMETERS,
        }
    }
}

impl SrsConfig {
    pub fn from_learning(l: &maru_core::config::LearningConfig) -> Self {
        Self {
            known_stability_days: l.known_stability_days,
            desired_retention: l.desired_retention,
            auto_save_after_lookups: l.auto_save_after_lookups,
            params: fsrs::DEFAULT_PARAMETERS,
        }
    }
}

/// Where a word was met.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Context {
    pub sentence: String,
    pub translation: Option<String>,
    pub app: Option<String>,
    pub screenshot: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LookupInfo {
    /// Dictionary form (lemma) — the identity of a word.
    pub key: String,
    pub surface: String,
    pub pos: String,
    pub meaning: Option<String>,
    pub level: Option<String>,
    pub context: Option<Context>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WordRecord {
    pub key: String,
    pub surface: String,
    pub pos: String,
    pub meaning: Option<String>,
    pub level: Option<String>,
    pub lookups: u32,
    pub saved: bool,
    pub marked_known: bool,
    pub stability: Option<f32>,
    pub difficulty: Option<f32>,
    pub due: Option<i64>,
    pub reps: u32,
    pub lapses: u32,
    /// How long Anki says you hold this word (its card interval, in days), once Maru has read it back.
    pub anki_interval: Option<f32>,
    pub state: WordState,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReviewOutcome {
    pub due: i64,
    pub interval_days: f32,
    pub stability: f32,
    pub difficulty: f32,
    pub state: WordState,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VocabStats {
    pub total: u32,
    pub saved: u32,
    pub known: u32,
    pub due_now: u32,
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

const DAY: i64 = 86_400;

pub struct Vocab {
    conn: Mutex<Connection>,
    cfg: SrsConfig,
}

const COLS: &str = "key, surface, pos, meaning, level, lookups, saved, known_override, stability, difficulty, due, reps, lapses, anki_interval";

impl Vocab {
    pub fn open(path: &Path, cfg: SrsConfig) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| SrsError::Other(e.to_string()))?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Self::init(conn, cfg)
    }

    pub fn open_in_memory(cfg: SrsConfig) -> Result<Self> {
        Self::init(Connection::open_in_memory()?, cfg)
    }

    fn init(conn: Connection, cfg: SrsConfig) -> Result<Self> {
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS words (
                key            TEXT PRIMARY KEY,
                surface        TEXT NOT NULL,
                pos            TEXT NOT NULL DEFAULT '',
                meaning        TEXT,
                level          TEXT,
                lookups        INTEGER NOT NULL DEFAULT 0,
                first_seen     INTEGER NOT NULL,
                last_seen      INTEGER NOT NULL,
                saved          INTEGER NOT NULL DEFAULT 0,
                known_override INTEGER NOT NULL DEFAULT 0,
                stability      REAL,
                difficulty     REAL,
                due            INTEGER,
                last_review    INTEGER,
                reps           INTEGER NOT NULL DEFAULT 0,
                lapses         INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS words_due ON words(saved, due);

            CREATE TABLE IF NOT EXISTS contexts (
                id          INTEGER PRIMARY KEY,
                key         TEXT NOT NULL,
                sentence    TEXT NOT NULL,
                translation TEXT,
                app         TEXT,
                screenshot  TEXT,
                ts          INTEGER NOT NULL,
                UNIQUE (key, sentence)
            );

            CREATE TABLE IF NOT EXISTS anki_decks (
                deck      TEXT PRIMARY KEY,
                last_used INTEGER NOT NULL,
                uses      INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS anki_app_deck (
                app  TEXT PRIMARY KEY,
                deck TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS reviews (
                id         INTEGER PRIMARY KEY,
                key        TEXT NOT NULL,
                ts         INTEGER NOT NULL,
                rating     INTEGER NOT NULL,
                elapsed    INTEGER NOT NULL,
                stability  REAL NOT NULL,
                difficulty REAL NOT NULL
            );
            ",
        )?;
        // Added after the first release: older databases get the column on opening.
        let has_interval = conn.prepare("SELECT 1 FROM pragma_table_info('words') WHERE name = 'anki_interval'")?.exists([])?;
        if !has_interval {
            conn.execute("ALTER TABLE words ADD COLUMN anki_interval REAL", [])?;
        }
        Ok(Self { conn: Mutex::new(conn), cfg })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn state_of(&self, known_override: bool, saved: bool, lookups: u32, stability: Option<f32>, anki_interval: Option<f32>) -> WordState {
        let holds = |d: Option<f32>| d.is_some_and(|d| d >= self.cfg.known_stability_days);
        if known_override || holds(stability) || holds(anki_interval) {
            WordState::Known
        } else if saved || lookups > 0 {
            WordState::Learning
        } else {
            WordState::New
        }
    }

    fn record_from_row(&self, r: &rusqlite::Row<'_>) -> rusqlite::Result<WordRecord> {
        let lookups: u32 = r.get(5)?;
        let saved = r.get::<_, i64>(6)? != 0;
        let known = r.get::<_, i64>(7)? != 0;
        let stability: Option<f32> = r.get(8)?;
        let anki_interval: Option<f32> = r.get(13)?;
        Ok(WordRecord {
            key: r.get(0)?,
            surface: r.get(1)?,
            pos: r.get(2)?,
            meaning: r.get(3)?,
            level: r.get(4)?,
            lookups,
            saved,
            marked_known: known,
            stability,
            difficulty: r.get(9)?,
            due: r.get(10)?,
            reps: r.get(11)?,
            lapses: r.get(12)?,
            anki_interval,
            state: self.state_of(known, saved, lookups, stability, anki_interval),
        })
    }

    pub fn get(&self, key: &str) -> Result<Option<WordRecord>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(&format!("SELECT {COLS} FROM words WHERE key = ?1"))?;
        Ok(stmt.query_row(params![key], |r| self.record_from_row(r)).optional()?)
    }

    /// Batch lookup used when annotating a sentence; missing keys are simply absent.
    pub fn get_many(&self, keys: &[&str]) -> Result<HashMap<String, WordRecord>> {
        let mut out = HashMap::new();
        for key in keys {
            if let Some(rec) = self.get(key)? {
                out.insert((*key).to_string(), rec);
            }
        }
        Ok(out)
    }

    /// The user deliberately engaged with a word (clicked it, asked for details).
    /// Counts the lookup, stores the sentence, and queues the word for review
    /// once it has been looked up often enough.
    pub fn record_lookup(&self, info: &LookupInfo, now: i64) -> Result<WordRecord> {
        {
            let mut conn = self.conn();
            let tx = conn.transaction()?;
            tx.execute(
                "INSERT INTO words (key, surface, pos, meaning, level, lookups, first_seen, last_seen)
                 VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?6)
                 ON CONFLICT(key) DO UPDATE SET
                    lookups = lookups + 1,
                    last_seen = ?6,
                    surface = ?2,
                    pos = CASE WHEN ?3 <> '' THEN ?3 ELSE pos END,
                    meaning = COALESCE(?4, meaning),
                    level = COALESCE(?5, level)",
                params![info.key, info.surface, info.pos, info.meaning, info.level, now],
            )?;
            if let Some(c) = &info.context
                && !c.sentence.is_empty()
            {
                tx.execute(
                    "INSERT OR IGNORE INTO contexts (key, sentence, translation, app, screenshot, ts)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![info.key, c.sentence, c.translation, c.app, c.screenshot, now],
                )?;
            }
            let threshold = self.cfg.auto_save_after_lookups;
            if threshold > 0 {
                tx.execute(
                    "UPDATE words SET saved = 1, due = COALESCE(due, ?2)
                     WHERE key = ?1 AND saved = 0 AND lookups >= ?3",
                    params![info.key, now, threshold],
                )?;
            }
            tx.commit()?;
        }
        Ok(self.get(&info.key)?.expect("row was just upserted"))
    }

    fn ensure_row(&self, key: &str, now: i64) -> Result<()> {
        self.conn().execute(
            "INSERT OR IGNORE INTO words (key, surface, first_seen, last_seen) VALUES (?1, ?1, ?2, ?2)",
            params![key, now],
        )?;
        Ok(())
    }

    /// Put a word in the review queue (due immediately).
    pub fn save(&self, key: &str, now: i64) -> Result<WordRecord> {
        self.ensure_row(key, now)?;
        self.conn().execute("UPDATE words SET saved = 1, due = COALESCE(due, ?2) WHERE key = ?1", params![key, now])?;
        Ok(self.get(key)?.expect("row exists"))
    }

    /// Fill in a meaning/level for a word that has none yet (never overwrites what you chose).
    pub fn enrich(&self, key: &str, meaning: Option<&str>, level: Option<&str>) -> Result<()> {
        self.conn().execute(
            "UPDATE words SET meaning = COALESCE(meaning, ?2), level = COALESCE(level, ?3) WHERE key = ?1",
            params![key, meaning, level],
        )?;
        Ok(())
    }

    pub fn unsave(&self, key: &str) -> Result<()> {
        self.conn().execute("UPDATE words SET saved = 0 WHERE key = ?1", params![key])?;
        Ok(())
    }

    /// "I already know this" — glosses fade without any review history.
    pub fn mark_known(&self, key: &str, known: bool, now: i64) -> Result<WordRecord> {
        self.ensure_row(key, now)?;
        self.conn().execute("UPDATE words SET known_override = ?2 WHERE key = ?1", params![key, i64::from(known)])?;
        Ok(self.get(key)?.expect("row exists"))
    }

    /// Mark many lemmas known in one transaction (e.g. "everything at TOPIK 1").
    pub fn mark_known_bulk<'a>(&self, keys: impl IntoIterator<Item = &'a str>, now: i64) -> Result<u32> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mut n = 0;
        {
            let mut ins = tx.prepare_cached(
                "INSERT INTO words (key, surface, first_seen, last_seen, known_override) VALUES (?1, ?1, ?2, ?2, 1)
                 ON CONFLICT(key) DO UPDATE SET known_override = 1",
            )?;
            for k in keys {
                ins.execute(params![k, now])?;
                n += 1;
            }
        }
        tx.commit()?;
        Ok(n)
    }

    /// What Anki reports for a word's card: `Some(days)` once the word is in Anki (0 while it is still
    /// being learned), `None` if it is not there. A word in Anki is saved; one held for long enough
    /// counts as known and its glosses fade.
    pub fn set_anki_state(&self, key: &str, interval_days: Option<f32>, now: i64) -> Result<()> {
        self.ensure_row(key, now)?;
        self.conn().execute(
            "UPDATE words SET anki_interval = ?2, saved = CASE WHEN ?2 IS NULL THEN saved ELSE 1 END WHERE key = ?1",
            params![key, interval_days],
        )?;
        Ok(())
    }

    /// Remember where a word went: the deck becomes a recent choice, and the program it was met in
    /// will suggest that deck next time.
    pub fn remember_deck(&self, deck: &str, app: Option<&str>, now: i64) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO anki_decks (deck, last_used, uses) VALUES (?1, ?2, 1)
             ON CONFLICT(deck) DO UPDATE SET last_used = ?2, uses = uses + 1",
            params![deck, now],
        )?;
        if let Some(app) = app.map(str::trim).filter(|a| !a.is_empty()) {
            conn.execute(
                "INSERT INTO anki_app_deck (app, deck) VALUES (?1, ?2) ON CONFLICT(app) DO UPDATE SET deck = ?2",
                params![app.to_ascii_lowercase(), deck],
            )?;
        }
        Ok(())
    }

    /// Decks used lately, newest first.
    pub fn recent_decks(&self, limit: u32) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached("SELECT deck FROM anki_decks ORDER BY last_used DESC LIMIT ?1")?;
        let rows = stmt.query_map(params![limit], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The deck last used for words from this program.
    pub fn deck_for_app(&self, app: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row("SELECT deck FROM anki_app_deck WHERE app = ?1", params![app.trim().to_ascii_lowercase()], |r| r.get(0))
            .optional()?)
    }

    /// Record a review and schedule the next one.
    pub fn review(&self, key: &str, rating: Rating, now: i64) -> Result<ReviewOutcome> {
        let rec = self.get(key)?.ok_or_else(|| SrsError::Other(format!("unknown word: {key}")))?;
        let last_review: Option<i64> =
            self.conn().query_row("SELECT last_review FROM words WHERE key = ?1", params![key], |r| r.get(0))?;
        let elapsed_days = last_review.map(|t| ((now - t).max(0) / DAY) as u32).unwrap_or(0);
        let memory = match (rec.stability, rec.difficulty) {
            (Some(stability), Some(difficulty)) => Some(Memory { stability, difficulty }),
            _ => None,
        };
        let new = fsrs::review(&self.cfg.params, memory, rating, elapsed_days);
        let interval_days = fsrs::next_interval(&self.cfg.params, new.stability, self.cfg.desired_retention)
            .round()
            .clamp(1.0, 36_500.0);
        let due = if rating == Rating::Again { now + 600 } else { now + interval_days as i64 * DAY };
        let lapse = i64::from(rating == Rating::Again && memory.is_some());

        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE words SET stability = ?2, difficulty = ?3, due = ?4, last_review = ?5,
                              reps = reps + 1, lapses = lapses + ?6, saved = 1, last_seen = ?5
             WHERE key = ?1",
            params![key, new.stability, new.difficulty, due, now, lapse],
        )?;
        tx.execute(
            "INSERT INTO reviews (key, ts, rating, elapsed, stability, difficulty) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![key, now, rating as i64, elapsed_days, new.stability, new.difficulty],
        )?;
        tx.commit()?;

        Ok(ReviewOutcome {
            due,
            interval_days: if rating == Rating::Again { 0.0 } else { interval_days },
            stability: new.stability,
            difficulty: new.difficulty,
            state: self.state_of(rec.marked_known, true, rec.lookups, Some(new.stability), rec.anki_interval),
        })
    }

    /// Words due for review, oldest first.
    pub fn due(&self, now: i64, limit: u32) -> Result<Vec<WordRecord>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT {COLS} FROM words WHERE saved = 1 AND (due IS NULL OR due <= ?1) ORDER BY due LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![now, limit], |r| self.record_from_row(r))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Most recently looked-up words first.
    pub fn recent(&self, limit: u32) -> Result<Vec<WordRecord>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare_cached(&format!("SELECT {COLS} FROM words WHERE lookups > 0 ORDER BY last_seen DESC LIMIT ?1"))?;
        let rows = stmt.query_map(params![limit], |r| self.record_from_row(r))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn saved(&self) -> Result<Vec<WordRecord>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(&format!("SELECT {COLS} FROM words WHERE saved = 1 ORDER BY first_seen"))?;
        let rows = stmt.query_map([], |r| self.record_from_row(r))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn contexts(&self, key: &str, limit: u32) -> Result<Vec<Context>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT sentence, translation, app, screenshot FROM contexts WHERE key = ?1 ORDER BY ts DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![key, limit], |r| {
            Ok(Context { sentence: r.get(0)?, translation: r.get(1)?, app: r.get(2)?, screenshot: r.get(3)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn stats(&self, now: i64) -> Result<VocabStats> {
        let conn = self.conn();
        let count = |sql: &str, p: &[&dyn rusqlite::ToSql]| -> rusqlite::Result<u32> { conn.query_row(sql, p, |r| r.get(0)) };
        Ok(VocabStats {
            total: count("SELECT COUNT(*) FROM words", &[])?,
            saved: count("SELECT COUNT(*) FROM words WHERE saved = 1", &[])?,
            known: count(
                "SELECT COUNT(*) FROM words WHERE known_override = 1 OR COALESCE(stability, 0) >= ?1 OR COALESCE(anki_interval, 0) >= ?1",
                &[&self.cfg.known_stability_days],
            )?,
            due_now: count("SELECT COUNT(*) FROM words WHERE saved = 1 AND (due IS NULL OR due <= ?1)", &[&now])?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vocab() -> Vocab {
        Vocab::open_in_memory(SrsConfig::default()).unwrap()
    }

    fn lookup(key: &str, sentence: &str) -> LookupInfo {
        LookupInfo {
            key: key.into(),
            surface: key.into(),
            pos: "verb".into(),
            meaning: Some("to study".into()),
            level: Some("TOPIK 1".into()),
            context: Some(Context { sentence: sentence.into(), translation: Some("I study.".into()), ..Default::default() }),
        }
    }

    #[test]
    fn unseen_words_are_new() {
        let v = vocab();
        assert!(v.get("공부하다").unwrap().is_none());
        assert!(v.get_many(&["공부하다"]).unwrap().is_empty());
    }

    #[test]
    fn lookup_counts_and_auto_saves_on_second_lookup() {
        let v = vocab();
        let a = v.record_lookup(&lookup("공부하다", "공부해요"), 100).unwrap();
        assert_eq!((a.lookups, a.saved, a.state), (1, false, WordState::Learning));
        let b = v.record_lookup(&lookup("공부하다", "공부했어요"), 200).unwrap();
        assert_eq!((b.lookups, b.saved), (2, true));
        assert_eq!(b.due, Some(200));
        assert_eq!(v.contexts("공부하다", 10).unwrap().len(), 2);
        // same sentence twice is stored once
        v.record_lookup(&lookup("공부하다", "공부했어요"), 300).unwrap();
        assert_eq!(v.contexts("공부하다", 10).unwrap().len(), 2);
    }

    #[test]
    fn marking_known_fades_without_reviews_and_can_be_undone() {
        let v = vocab();
        assert_eq!(v.mark_known("학교", true, 1).unwrap().state, WordState::Known);
        assert_eq!(v.get("학교").unwrap().unwrap().state, WordState::Known);
        assert_eq!(v.mark_known("학교", false, 2).unwrap().state, WordState::New);
    }

    #[test]
    fn enrich_fills_gaps_but_never_overwrites() {
        let v = vocab();
        v.save("물", 0).unwrap();
        v.enrich("물", Some("water"), Some("TOPIK 1")).unwrap();
        v.enrich("물", Some("liquid"), Some("TOPIK 2")).unwrap();
        let w = v.get("물").unwrap().unwrap();
        assert_eq!((w.meaning.as_deref(), w.level.as_deref()), (Some("water"), Some("TOPIK 1")));
    }

    #[test]
    fn bulk_mark_known() {
        let v = vocab();
        assert_eq!(v.mark_known_bulk(["가", "나", "다"], 5).unwrap(), 3);
        let m = v.get_many(&["가", "나", "라"]).unwrap();
        assert_eq!(m.len(), 2);
        assert!(m.values().all(|w| w.state == WordState::Known));
    }

    #[test]
    fn reviews_schedule_and_eventually_make_a_word_known() {
        let v = vocab();
        v.save("사과", 0).unwrap();
        assert_eq!(v.due(0, 10).unwrap().len(), 1);

        let first = v.review("사과", Rating::Good, 0).unwrap();
        assert!(first.interval_days >= 2.0);
        assert_eq!(first.state, WordState::Learning);
        assert!(v.due(0, 10).unwrap().is_empty());
        assert_eq!(v.due(first.due, 10).unwrap().len(), 1);

        let mut now = first.due;
        let mut last = first;
        for _ in 0..6 {
            last = v.review("사과", Rating::Good, now).unwrap();
            now = last.due;
        }
        assert!(last.interval_days > first.interval_days);
        assert_eq!(last.state, WordState::Known);
        assert_eq!(v.get("사과").unwrap().unwrap().reps, 7);
    }

    #[test]
    fn forgetting_counts_a_lapse_and_reschedules_soon() {
        let v = vocab();
        v.save("물", 0).unwrap();
        let ok = v.review("물", Rating::Good, 0).unwrap();
        let bad = v.review("물", Rating::Again, ok.due).unwrap();
        assert_eq!(bad.due, ok.due + 600);
        assert_eq!(v.get("물").unwrap().unwrap().lapses, 1);
        assert!(bad.stability < ok.stability);
    }

    #[test]
    fn stats_and_recent() {
        let v = vocab();
        v.record_lookup(&lookup("a", "s1"), 10).unwrap();
        v.record_lookup(&lookup("b", "s2"), 20).unwrap();
        v.mark_known("c", true, 30).unwrap();
        v.save("b", 20).unwrap();
        let s = v.stats(20).unwrap();
        assert_eq!((s.total, s.saved, s.known, s.due_now), (3, 1, 1, 1));
        assert_eq!(v.recent(5).unwrap()[0].key, "b");
    }

    #[test]
    fn reviewing_an_unknown_word_is_an_error() {
        assert!(vocab().review("없음", Rating::Good, 0).is_err());
    }

    #[test]
    fn a_word_anki_holds_for_long_enough_counts_as_known() {
        let v = vocab();
        v.set_anki_state("공부하다", Some(3.0), 100).unwrap();
        let w = v.get("공부하다").unwrap().unwrap();
        assert!(w.saved, "a word in Anki is saved");
        assert_eq!((w.anki_interval, w.state), (Some(3.0), WordState::Learning));
        v.set_anki_state("공부하다", Some(45.0), 200).unwrap();
        assert_eq!(v.get("공부하다").unwrap().unwrap().state, WordState::Known, "held for weeks");
        assert_eq!(v.stats(300).unwrap().known, 1);
        v.set_anki_state("공부하다", Some(0.0), 300).unwrap();
        assert_eq!(v.get("공부하다").unwrap().unwrap().state, WordState::Learning, "a lapse un-knows it again");
        v.set_anki_state("공부하다", None, 400).unwrap();
        let gone = v.get("공부하다").unwrap().unwrap();
        assert_eq!(gone.anki_interval, None);
        assert!(gone.saved, "it stays saved in Maru");
    }

    #[test]
    fn decks_are_remembered_per_program_and_by_recency() {
        let v = vocab();
        v.remember_deck("Korean::Games", Some("EldenRing.exe"), 10).unwrap();
        v.remember_deck("Korean::YouTube", Some("chrome.exe"), 20).unwrap();
        v.remember_deck("Korean::Games", Some("eldenring.exe"), 30).unwrap();
        assert_eq!(v.recent_decks(5).unwrap(), ["Korean::Games", "Korean::YouTube"]);
        assert_eq!(v.deck_for_app("ELDENRING.EXE").unwrap().as_deref(), Some("Korean::Games"));
        assert_eq!(v.deck_for_app("notepad.exe").unwrap(), None);
        v.remember_deck("Korean::Book", None, 40).unwrap();
        assert_eq!(v.recent_decks(1).unwrap(), ["Korean::Book"]);
    }

    #[test]
    fn an_old_database_gets_the_new_column() {
        let dir = std::env::temp_dir().join(format!("maru-old-db-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("vocab.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE words (key TEXT PRIMARY KEY, surface TEXT NOT NULL, pos TEXT NOT NULL DEFAULT '', meaning TEXT, level TEXT, lookups INTEGER NOT NULL DEFAULT 0, first_seen INTEGER NOT NULL, last_seen INTEGER NOT NULL, saved INTEGER NOT NULL DEFAULT 0, known_override INTEGER NOT NULL DEFAULT 0, stability REAL, difficulty REAL, due INTEGER, last_review INTEGER, reps INTEGER NOT NULL DEFAULT 0, lapses INTEGER NOT NULL DEFAULT 0); INSERT INTO words (key, surface, first_seen, last_seen) VALUES ('옛', '옛', 1, 1);").unwrap();
        }
        let v = Vocab::open(&path, SrsConfig::default()).unwrap();
        assert_eq!(v.get("옛").unwrap().unwrap().anki_interval, None);
        v.set_anki_state("옛", Some(5.0), 2).unwrap();
        drop(v);
        assert_eq!(Vocab::open(&path, SrsConfig::default()).unwrap().get("옛").unwrap().unwrap().anki_interval, Some(5.0), "opening twice is fine");
        std::fs::remove_dir_all(dir).ok();
    }
}
