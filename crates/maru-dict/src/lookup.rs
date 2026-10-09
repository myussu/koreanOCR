use maru_core::event::{DictEntry, DictSense};
use maru_core::{Eojeol, Pos, Role};
use rusqlite::{OptionalExtension, params};

use crate::{Dictionary, Result};

/// Dictionary keys to try for a word, most specific first: the full lemma
/// (공부하다), then the noun it was built on (공부), then the first stem.
pub fn lookup_keys(e: &Eojeol) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    let mut push = |k: String| {
        if !k.is_empty() && !keys.contains(&k) {
            keys.push(k);
        }
    };
    if let Some(l) = &e.lemma {
        push(l.clone());
        for suffix in ["하다", "되다", "시키다", "스럽다", "롭다"] {
            if let Some(stem) = l.strip_suffix(suffix) {
                push(stem.to_string());
            }
        }
    }
    if let Some(stem) = e.morphs.iter().find(|m| m.role == Role::Stem) {
        let form = if matches!(e.pos, Pos::Verb | Pos::Adjective) && !stem.form.ends_with('다') {
            format!("{}다", stem.form)
        } else {
            stem.form.clone()
        };
        push(form);
    }
    push(e.surface.trim_matches(|c: char| !c.is_alphanumeric()).to_string());
    keys
}

fn pos_matches(entry_pos: &str, pos: Pos) -> bool {
    match pos {
        Pos::Noun => entry_pos == "noun" || entry_pos == "proper noun",
        Pos::Pronoun => entry_pos == "pronoun",
        Pos::Numeral => entry_pos == "numeral",
        Pos::Verb => entry_pos == "verb",
        Pos::Adjective => entry_pos == "adjective" || entry_pos == "verb",
        Pos::Adverb => entry_pos == "adverb",
        Pos::Determiner => entry_pos == "determiner",
        Pos::Interjection => entry_pos == "interjection",
        _ => false,
    }
}

/// "to study; to learn" → "to study" — one short reading, UI-sized.
pub fn short_gloss(g: &str) -> String {
    let first = g.split([';', '—']).next().unwrap_or(g).trim();
    let mut s: String = first.chars().take(40).collect();
    if first.chars().count() > 40 {
        s.push('…');
    }
    s
}

impl Dictionary {
    pub fn lookup(&self, lemma: &str) -> Result<Vec<DictEntry>> {
        let conn = self.conn();
        let level: Option<String> =
            conn.query_row("SELECT level FROM levels WHERE lemma = ?1", params![lemma], |r| r.get(0)).optional()?;
        let mut stmt = conn.prepare_cached(
            "SELECT lemma, pos, senses, hanja, etymology FROM entries WHERE lemma = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map(params![lemma], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (lemma, pos, senses, hanja, etymology) = row?;
            #[derive(serde::Deserialize)]
            struct S {
                gloss: String,
                #[serde(default)]
                examples: Vec<String>,
            }
            let senses: Vec<S> = serde_json::from_str(&senses).unwrap_or_default();
            out.push(DictEntry {
                lemma,
                pos,
                senses: senses.into_iter().map(|s| DictSense { gloss: s.gloss, examples: s.examples }).collect(),
                hanja,
                etymology,
                level: level.clone(),
            });
        }
        Ok(out)
    }

    /// Resolve an inflected form to its lemma via Wiktionary form-of data.
    fn lemma_of_form(&self, form: &str) -> Result<Option<String>> {
        let conn = self.conn();
        Ok(conn
            .query_row("SELECT lemma FROM forms WHERE form = ?1 LIMIT 1", params![form], |r| r.get(0))
            .optional()?)
    }

    /// All entries for a word, trying every candidate key; first key with hits wins.
    pub fn lookup_word(&self, e: &Eojeol) -> Result<Vec<DictEntry>> {
        for key in lookup_keys(e) {
            let mut hits = self.lookup(&key)?;
            if hits.is_empty()
                && let Some(lemma) = self.lemma_of_form(&key)?
            {
                hits = self.lookup(&lemma)?;
            }
            if !hits.is_empty() {
                // Entries whose part of speech matches the analysis first.
                hits.sort_by_key(|h| !pos_matches(&h.pos, e.pos));
                return Ok(hits);
            }
        }
        Ok(Vec::new())
    }

    /// Up to `max` short meanings for the word, best part-of-speech match first.
    pub fn short_glosses(&self, e: &Eojeol, max: usize) -> Result<Vec<String>> {
        let mut out: Vec<String> = Vec::new();
        for entry in self.lookup_word(e)? {
            for sense in entry.senses.iter().take(2) {
                let g = short_gloss(&sense.gloss);
                if !g.is_empty() && !out.contains(&g) {
                    out.push(g);
                }
                if out.len() >= max {
                    return Ok(out);
                }
            }
        }
        Ok(out)
    }

    /// Short meanings and course level in a single pass (what the overlay shows under a word).
    pub fn gloss_and_level(&self, e: &Eojeol, max: usize) -> Result<(Vec<String>, Option<String>)> {
        let entries = self.lookup_word(e)?;
        let mut glosses: Vec<String> = Vec::new();
        'outer: for entry in &entries {
            for sense in entry.senses.iter().take(2) {
                let g = short_gloss(&sense.gloss);
                if !g.is_empty() && !glosses.contains(&g) {
                    glosses.push(g);
                }
                if glosses.len() >= max {
                    break 'outer;
                }
            }
        }
        let mut level = entries.iter().find_map(|en| en.level.clone());
        if level.is_none() {
            level = self.level_of(e.vocab_key())?;
        }
        Ok((glosses, level))
    }

    /// Every lemma whose course level is TOPIK `n` or easier (for "I know everything up to level N").
    pub fn lemmas_up_to_topik(&self, n: u8) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached("SELECT lemma, level FROM levels")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (lemma, level) = row?;
            let rank = match level.strip_prefix("TOPIK ").and_then(|d| d.parse::<u8>().ok()) {
                Some(n) => n,
                None => match level.as_str() {
                    "Beginner" => 2,
                    "Intermediate" => 4,
                    "Advanced" => 6,
                    _ => continue,
                },
            };
            if rank <= n {
                out.push(lemma);
            }
        }
        Ok(out)
    }

    pub fn level_of(&self, lemma: &str) -> Result<Option<String>> {
        let conn = self.conn();
        Ok(conn.query_row("SELECT level FROM levels WHERE lemma = ?1", params![lemma], |r| r.get(0)).optional()?)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use maru_core::{Morpheme, WordState};

    use super::*;
    use crate::normalize_level;

    const KAIKKI: &str = r#"
{"word":"학교","lang_code":"ko","pos":"noun","etymology_text":"Sino-Korean word from 學校 (hakgyo)","senses":[{"glosses":["school"],"examples":[{"text":"학교에 가요.","english":"I go to school."}]}]}
{"word":"공부하다","lang_code":"ko","pos":"verb","senses":[{"glosses":["to study; to learn"]},{"glosses":["to do homework"]}]}
{"word":"공부","lang_code":"ko","pos":"noun","forms":[{"form":"工夫","tags":["hanja"]}],"senses":[{"glosses":["study, studies"]}]}
{"word":"했어요","lang_code":"ko","pos":"verb","senses":[{"glosses":["inflection of 하다"],"form_of":[{"word":"하다"}]}]}
{"word":"하다","lang_code":"ko","pos":"verb","senses":[{"glosses":["to do"]}]}
{"word":"dog","lang_code":"en","pos":"noun","senses":[{"glosses":["x"]}]}
not json at all
{"word":"빈","lang_code":"ko","pos":"noun","senses":[]}
"#;

    fn dict() -> Dictionary {
        let d = Dictionary::open_in_memory().unwrap();
        let stats = d.import_kaikki(Cursor::new(KAIKKI), |_| {}).unwrap();
        assert_eq!(stats.entries, 4);
        assert_eq!(stats.forms, 1);
        assert_eq!(stats.skipped, 3); // english, malformed, no senses
        d.import_levels(Cursor::new("word\tlevel\n학교\t1\n공부\t초급\n하다\t99\n")).unwrap();
        d
    }

    fn word(lemma: &str, surface: &str, pos: Pos, stems: &[&str]) -> Eojeol {
        Eojeol {
            index: 0,
            surface: surface.into(),
            char_start: 0,
            char_end: surface.chars().count(),
            morphs: stems
                .iter()
                .map(|f| Morpheme { form: (*f).into(), tag: "NNG".into(), role: Role::Stem, label: String::new(), start: 0, end: 1 })
                .collect(),
            lemma: Some(lemma.into()),
            pos,
            content: true,
            formula: String::new(),
            romanization: String::new(),
            pronunciation: String::new(),
            dict_glosses: vec![],
            level: None,
            state: WordState::New,
            lookups: 0,
            rect: None,
        }
    }

    #[test]
    fn lookup_returns_senses_hanja_etymology_and_level() {
        let d = dict();
        let e = &d.lookup("학교").unwrap()[0];
        assert_eq!(e.senses[0].gloss, "school");
        assert_eq!(e.senses[0].examples, ["학교에 가요. — I go to school."]);
        assert_eq!(e.hanja.as_deref(), Some("學校"));
        assert_eq!(e.level.as_deref(), Some("TOPIK 1"));
        assert_eq!(d.lookup("공부").unwrap()[0].hanja.as_deref(), Some("工夫"));
    }

    #[test]
    fn invalid_levels_are_ignored_and_header_skipped() {
        let d = dict();
        assert_eq!(d.level_of("하다").unwrap(), None);
        assert_eq!(d.level_of("공부").unwrap().as_deref(), Some("Beginner"));
    }

    #[test]
    fn eojeol_lookup_prefers_full_lemma_then_root() {
        let d = dict();
        let e = word("공부하다", "공부했어요", Pos::Verb, &["공부"]);
        let hits = d.lookup_word(&e).unwrap();
        assert_eq!(hits[0].lemma, "공부하다");
        let g = d.short_glosses(&e, 3).unwrap();
        assert_eq!(g, ["to study", "to do homework"]);

        // lemma not in the dictionary → falls back to the noun root
        let e2 = word("공부되다", "공부되고", Pos::Verb, &["공부"]);
        assert_eq!(d.lookup_word(&e2).unwrap()[0].lemma, "공부");
    }

    #[test]
    fn inflected_form_resolves_through_form_of_data() {
        let d = dict();
        let e = word("했어요", "했어요", Pos::Verb, &[]);
        assert_eq!(d.lookup_word(&e).unwrap()[0].lemma, "하다");
    }

    #[test]
    fn gloss_and_level_in_one_call() {
        let d = dict();
        let e = word("학교", "학교에서", Pos::Noun, &["학교"]);
        assert_eq!(d.gloss_and_level(&e, 3).unwrap(), (vec!["school".to_string()], Some("TOPIK 1".to_string())));
        let unknown = word("모름", "모름", Pos::Noun, &["모름"]);
        assert_eq!(d.gloss_and_level(&unknown, 3).unwrap(), (vec![], None));
    }

    #[test]
    fn lemmas_up_to_a_level() {
        let d = dict();
        d.import_levels(Cursor::new("가\t3\n나\t초급\n다\t5\n")).unwrap();
        let mut up_to_3 = d.lemmas_up_to_topik(3).unwrap();
        up_to_3.sort();
        assert_eq!(up_to_3, ["가", "공부", "나", "학교"]); // 공부 = Beginner (2), 학교 = TOPIK 1
        assert_eq!(d.lemmas_up_to_topik(1).unwrap(), ["학교"]);
    }

    #[test]
    fn level_normalisation() {
        assert_eq!(normalize_level("3").as_deref(), Some("TOPIK 3"));
        assert_eq!(normalize_level("고급").as_deref(), Some("Advanced"));
        assert_eq!(normalize_level("7"), None);
        assert_eq!(normalize_level("level"), None);
    }

    #[test]
    fn short_gloss_trims() {
        assert_eq!(short_gloss("to study; to learn"), "to study");
        assert!(short_gloss(&"x".repeat(100)).ends_with('…'));
    }

    #[test]
    fn empty_dictionary_reports_empty() {
        assert!(Dictionary::open_in_memory().unwrap().is_empty());
    }
}
