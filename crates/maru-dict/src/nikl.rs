//! The learner's dictionary: 국립국어원 «한국어기초사전» (Basic Korean Dictionary, https://krdict.korean.go.kr,
//! CC BY-SA 2.0 KR), with an English and a Spanish equivalent for every sense.
//!
//! It lives beside the Wiktionary dictionary in its own file (`nikl.db`), made by `tools/build_nikl.py` and
//! `maru dict nikl`; the data is not bundled. Word cards take their meanings from it: a word with one sense needs no
//! model, and for the others a model only chooses among the dictionary's senses.

use std::io::BufRead;
use std::path::Path;
use std::sync::Mutex;

use maru_core::Pos;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;

use crate::Result;

/// One sense of a word, as the dictionary gives it.
#[derive(Debug, Clone, PartialEq)]
pub struct NiklSense {
    /// The dictionary's English equivalent (`belly; stomach`).
    pub en: String,
    /// The dictionary's Spanish equivalent; some senses have none.
    pub es: String,
    pub en_definition: String,
    pub es_definition: String,
    /// The dictionary's own part of speech (`명사`).
    pub pos: String,
    /// `초급`, `중급`, `고급`, or empty.
    pub level: String,
    /// Which of several words written the same way (배: belly, ship, pear); 0 when there is only one.
    pub homonym: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct NiklStats {
    pub words: u64,
    pub senses: u64,
}

pub struct Nikl {
    conn: Mutex<Connection>,
}

#[derive(Deserialize)]
struct Record {
    #[serde(default)]
    meta: Option<std::collections::BTreeMap<String, String>>,
    #[serde(default)]
    w: String,
    #[serde(default)]
    h: u32,
    #[serde(default)]
    p: String,
    #[serde(default)]
    l: String,
    #[serde(default)]
    s: Vec<SenseRecord>,
}

#[derive(Deserialize)]
struct SenseRecord {
    #[serde(default)]
    en: String,
    #[serde(default)]
    es: String,
    #[serde(default)]
    end: String,
    #[serde(default)]
    esd: String,
}

/// Whether a dictionary entry of this part of speech can be the word the analyser found.
fn pos_matches(entry: &str, pos: Pos) -> bool {
    match pos {
        Pos::Noun => matches!(entry, "명사" | "의존 명사"),
        Pos::Pronoun => entry == "대명사",
        Pos::Numeral => entry == "수사",
        Pos::Verb => matches!(entry, "동사" | "보조 동사"),
        // The analyser calls some descriptive verbs adjectives and the reverse.
        Pos::Adjective => matches!(entry, "형용사" | "보조 형용사" | "동사"),
        Pos::Adverb => entry == "부사",
        Pos::Determiner => entry == "관형사",
        Pos::Interjection => entry == "감탄사",
        _ => false,
    }
}

impl Nikl {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        Self::with(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::with(Connection::open_in_memory()?)
    }

    fn with(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS nikl_entries (
                id      INTEGER PRIMARY KEY,
                lemma   TEXT NOT NULL,
                homonym INTEGER NOT NULL DEFAULT 0,
                pos     TEXT NOT NULL DEFAULT '',
                level   TEXT NOT NULL DEFAULT ''
            );
            CREATE INDEX IF NOT EXISTS nikl_entries_lemma ON nikl_entries(lemma);
            CREATE TABLE IF NOT EXISTS nikl_senses (
                entry  INTEGER NOT NULL,
                n      INTEGER NOT NULL,
                en     TEXT NOT NULL DEFAULT '',
                es     TEXT NOT NULL DEFAULT '',
                en_def TEXT NOT NULL DEFAULT '',
                es_def TEXT NOT NULL DEFAULT '',
                PRIMARY KEY (entry, n)
            );
            -- where the data came from and its licence, as written by tools/build_nikl.py
            CREATE TABLE IF NOT EXISTS nikl_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            ",
        )?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Replace the dictionary with the file made by `tools/build_nikl.py`. Lines that are not valid are skipped
    /// and counted; nothing changes unless the whole file was read.
    pub fn import_jsonl<R: BufRead>(&self, reader: R) -> Result<(NiklStats, u64)> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute_batch("DELETE FROM nikl_senses; DELETE FROM nikl_entries; DELETE FROM nikl_meta;")?;
        let (mut stats, mut skipped) = (NiklStats::default(), 0u64);
        {
            let mut entry = tx.prepare("INSERT INTO nikl_entries (lemma, homonym, pos, level) VALUES (?1, ?2, ?3, ?4)")?;
            let mut sense = tx.prepare("INSERT INTO nikl_senses (entry, n, en, es, en_def, es_def) VALUES (?1, ?2, ?3, ?4, ?5, ?6)")?;
            let mut meta = tx.prepare("INSERT OR REPLACE INTO nikl_meta (key, value) VALUES (?1, ?2)")?;
            for line in reader.lines() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                let Ok(record) = serde_json::from_str::<Record>(&line) else {
                    skipped += 1;
                    continue;
                };
                if let Some(fields) = record.meta {
                    for (key, value) in fields {
                        meta.execute(params![key, value])?;
                    }
                    continue;
                }
                let senses: Vec<&SenseRecord> = record.s.iter().filter(|s| !s.en.trim().is_empty() || !s.es.trim().is_empty()).collect();
                if record.w.trim().is_empty() || senses.is_empty() {
                    skipped += 1;
                    continue;
                }
                entry.execute(params![record.w.trim(), record.h, record.p.trim(), record.l.trim()])?;
                let id = tx.last_insert_rowid();
                for (n, s) in senses.iter().enumerate() {
                    sense.execute(params![id, n as i64, s.en.trim(), s.es.trim(), s.end.trim(), s.esd.trim()])?;
                }
                stats.words += 1;
                stats.senses += senses.len() as u64;
            }
        }
        tx.commit()?;
        Ok((stats, skipped))
    }

    pub fn stats(&self) -> Result<NiklStats> {
        let conn = self.conn();
        let words: i64 = conn.query_row("SELECT COUNT(*) FROM nikl_entries", [], |r| r.get(0))?;
        let senses: i64 = conn.query_row("SELECT COUNT(*) FROM nikl_senses", [], |r| r.get(0))?;
        Ok(NiklStats { words: words as u64, senses: senses as u64 })
    }

    pub fn is_empty(&self) -> bool {
        self.stats().map(|s| s.words == 0).unwrap_or(true)
    }

    /// `source`, `license`, `built`… as recorded when the file was made.
    pub fn meta(&self, key: &str) -> Option<String> {
        self.conn().query_row("SELECT value FROM nikl_meta WHERE key = ?1", [key], |r| r.get(0)).optional().ok().flatten()
    }

    /// Every sense in the dictionary with its headword, entry by entry (`f(entry id, headword, sense)`), for counting.
    pub fn each_sense(&self, mut f: impl FnMut(i64, &str, &NiklSense)) -> Result<()> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT e.id, e.lemma, s.en, s.es, s.en_def, s.es_def, e.pos, e.level, e.homonym FROM nikl_entries e JOIN nikl_senses s ON s.entry = e.id ORDER BY e.id, s.n")?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            let lemma: String = r.get(1)?;
            let sense = NiklSense { en: r.get(2)?, es: r.get(3)?, en_definition: r.get(4)?, es_definition: r.get(5)?, pos: r.get(6)?, level: r.get(7)?, homonym: r.get(8)? };
            f(r.get(0)?, &lemma, &sense);
        }
        Ok(())
    }

    /// Every sense of the words written `lemma`, in the dictionary's order. When some of them have the analyser's
    /// part of speech, only those are returned (배 as a noun leaves out nothing; 가다 as a verb leaves out the auxiliary).
    pub fn senses(&self, lemma: &str, pos: Pos) -> Result<Vec<NiklSense>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT s.en, s.es, s.en_def, s.es_def, e.pos, e.level, e.homonym
             FROM nikl_entries e JOIN nikl_senses s ON s.entry = e.id
             WHERE e.lemma = ?1 ORDER BY e.homonym, e.id, s.n",
        )?;
        let all: Vec<NiklSense> = stmt
            .query_map([lemma], |r| {
                Ok(NiklSense { en: r.get(0)?, es: r.get(1)?, en_definition: r.get(2)?, es_definition: r.get(3)?, pos: r.get(4)?, level: r.get(5)?, homonym: r.get(6)? })
            })?
            .collect::<rusqlite::Result<_>>()?;
        if all.iter().any(|s| pos_matches(&s.pos, pos)) {
            return Ok(all.into_iter().filter(|s| pos_matches(&s.pos, pos)).collect());
        }
        Ok(all)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const FILE: &str = r#"{"meta":{"source":"국립국어원 한국어기초사전, https://krdict.korean.go.kr","license":"CC BY-SA 2.0 KR","built":"2026-11-25"}}
{"w":"배","h":1,"p":"명사","l":"초급","s":[{"en":"belly; stomach","es":"barriga","end":"The part of the body.","esd":"Parte del cuerpo."},{"en":"womb"}]}
{"w":"배","h":2,"p":"명사","l":"초급","s":[{"en":"ship; boat","es":"barco"}]}
{"w":"학교","h":0,"p":"명사","l":"초급","s":[{"en":"school","es":"escuela"}]}
{"w":"가다","h":1,"p":"동사","l":"초급","s":[{"en":"go","es":"ir"}]}
{"w":"가다","h":2,"p":"보조 동사","l":"","s":[{"en":"keep doing","es":"seguir"}]}
{"w":"만","h":1,"p":"의존 명사","l":"","s":[{"en":"after","es":"después de"}]}
not json
{"w":"빈","h":0,"p":"명사","l":"","s":[]}
"#;

    fn dictionary() -> Nikl {
        let nikl = Nikl::open_in_memory().unwrap();
        let (stats, skipped) = nikl.import_jsonl(Cursor::new(FILE)).unwrap();
        assert_eq!((stats, skipped), (NiklStats { words: 6, senses: 7 }, 2), "a broken line and a word without senses are skipped");
        nikl
    }

    #[test]
    fn a_word_gives_every_sense_of_every_homonym_in_dictionary_order_with_both_languages() {
        let nikl = dictionary();
        let senses = nikl.senses("배", Pos::Noun).unwrap();
        assert_eq!(senses.iter().map(|s| s.en.as_str()).collect::<Vec<_>>(), ["belly; stomach", "womb", "ship; boat"]);
        assert_eq!((senses[0].es.as_str(), senses[0].en_definition.as_str(), senses[0].level.as_str(), senses[0].homonym), ("barriga", "The part of the body.", "초급", 1));
        assert_eq!(senses[1].es, "", "a sense without a Spanish equivalent stays without one");
        assert_eq!(nikl.senses("학교", Pos::Noun).unwrap().len(), 1);
        assert!(nikl.senses("없는말", Pos::Noun).unwrap().is_empty());
    }

    #[test]
    fn the_part_of_speech_narrows_the_senses_only_when_it_matches_some() {
        let nikl = dictionary();
        assert_eq!(nikl.senses("가다", Pos::Verb).unwrap().len(), 2, "a verb and an auxiliary verb are both verbs");
        assert_eq!(nikl.senses("가다", Pos::Noun).unwrap().len(), 2, "no noun 가다: every sense rather than none");
        assert_eq!(nikl.senses("만", Pos::Noun).unwrap()[0].en, "after");
    }

    #[test]
    fn importing_again_replaces_the_dictionary_and_keeps_where_it_came_from() {
        let nikl = dictionary();
        assert_eq!(nikl.meta("license").as_deref(), Some("CC BY-SA 2.0 KR"));
        assert!(nikl.meta("source").unwrap().contains("krdict.korean.go.kr"));
        let again = r#"{"w":"학교","h":0,"p":"명사","l":"초급","s":[{"en":"school","es":"colegio"}]}"#;
        nikl.import_jsonl(Cursor::new(again)).unwrap();
        assert_eq!(nikl.stats().unwrap(), NiklStats { words: 1, senses: 1 });
        assert_eq!(nikl.senses("학교", Pos::Noun).unwrap()[0].es, "colegio");
        assert!(nikl.senses("배", Pos::Noun).unwrap().is_empty());
        assert!(!nikl.is_empty());
    }
}
