use std::io::BufRead;

use rusqlite::params;
use serde::Serialize;
use serde_json::Value;

use crate::{Dictionary, Result};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ImportStats {
    pub lines: u64,
    pub entries: u64,
    pub forms: u64,
    pub skipped: u64,
}

#[derive(Serialize)]
struct SenseOut {
    gloss: String,
    examples: Vec<String>,
}

fn normalize_pos(pos: &str) -> String {
    match pos {
        "adj" => "adjective",
        "adv" => "adverb",
        "pron" => "pronoun",
        "num" => "numeral",
        "det" => "determiner",
        "intj" => "interjection",
        "name" => "proper noun",
        "prep_phrase" => "phrase",
        other => other,
    }
    .to_string()
}

/// First run of CJK ideographs, which in Wiktionary etymologies is the Hanja.
fn first_hanja(text: &str) -> Option<String> {
    let mut run = String::new();
    for c in text.chars() {
        if ('\u{4E00}'..='\u{9FFF}').contains(&c) {
            run.push(c);
        } else if !run.is_empty() {
            break;
        }
    }
    (!run.is_empty()).then_some(run)
}

/// Shorten "a; b; c" glosses, drop leading usage labels like "(informal)".
fn clean_gloss(g: &str) -> String {
    g.trim().to_string()
}

fn string_list(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str().map(clean_gloss)).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default()
}

impl Dictionary {
    /// Import Wiktionary JSON lines. Tolerant: unknown/missing fields are skipped,
    /// malformed lines are counted and ignored.
    pub fn import_kaikki<R: BufRead>(&self, reader: R, mut progress: impl FnMut(u64)) -> Result<ImportStats> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mut stats = ImportStats::default();
        {
            let mut insert_entry =
                tx.prepare("INSERT INTO entries (lemma, pos, senses, hanja, etymology) VALUES (?1, ?2, ?3, ?4, ?5)")?;
            let mut insert_form = tx.prepare("INSERT OR IGNORE INTO forms (form, lemma) VALUES (?1, ?2)")?;

            for line in reader.lines() {
                let line = line?;
                stats.lines += 1;
                if line.trim().is_empty() {
                    continue;
                }
                let Ok(v) = serde_json::from_str::<Value>(&line) else {
                    stats.skipped += 1;
                    continue;
                };
                if v.get("lang_code").and_then(Value::as_str).is_some_and(|c| c != "ko") {
                    stats.skipped += 1;
                    continue;
                }
                let Some(word) = v.get("word").and_then(Value::as_str) else {
                    stats.skipped += 1;
                    continue;
                };
                let pos = normalize_pos(v.get("pos").and_then(Value::as_str).unwrap_or("other"));

                let mut senses: Vec<SenseOut> = Vec::new();
                let forms_before = stats.forms;
                for s in v.get("senses").and_then(Value::as_array).into_iter().flatten() {
                    // "X is an inflection of Y": remember the mapping, not a meaning.
                    if let Some(targets) = s.get("form_of").and_then(Value::as_array) {
                        for t in targets {
                            if let Some(lemma) = t.get("word").and_then(Value::as_str) {
                                insert_form.execute(params![word, lemma])?;
                                stats.forms += 1;
                            }
                        }
                        continue;
                    }
                    let glosses = string_list(s.get("glosses").unwrap_or(&Value::Null));
                    let Some(gloss) = glosses.first().cloned() else { continue };
                    let examples = s
                        .get("examples")
                        .and_then(Value::as_array)
                        .map(|a| {
                            a.iter()
                                .filter_map(|e| {
                                    let ko = e.get("text").and_then(Value::as_str)?;
                                    Some(match e.get("english").and_then(Value::as_str) {
                                        Some(en) => format!("{ko} — {en}"),
                                        None => ko.to_string(),
                                    })
                                })
                                .take(2)
                                .collect()
                        })
                        .unwrap_or_default();
                    senses.push(SenseOut { gloss, examples });
                }
                if senses.is_empty() {
                    // A pure "inflection of X" line already produced form data.
                    if stats.forms == forms_before {
                        stats.skipped += 1;
                    }
                    continue;
                }

                let etymology = v.get("etymology_text").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
                let hanja = v
                    .get("forms")
                    .and_then(Value::as_array)
                    .and_then(|forms| {
                        forms.iter().find_map(|f| {
                            let tagged = f.get("tags").and_then(Value::as_array).is_some_and(|t| t.iter().any(|x| x == "hanja"));
                            tagged.then(|| f.get("form").and_then(Value::as_str).map(str::to_owned)).flatten()
                        })
                    })
                    .or_else(|| etymology.and_then(first_hanja));

                insert_entry.execute(params![word, pos, serde_json::to_string(&senses).unwrap_or_default(), hanja, etymology])?;
                stats.entries += 1;
                if stats.entries.is_multiple_of(5000) {
                    progress(stats.entries);
                }
            }
        }
        tx.commit()?;
        Ok(stats)
    }

    /// Import `lemma<TAB>level` (or comma separated) lines. A header line is skipped
    /// automatically because it will not normalise to a level.
    pub fn import_levels<R: BufRead>(&self, reader: R) -> Result<u64> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mut n = 0;
        {
            let mut stmt = tx.prepare("INSERT OR REPLACE INTO levels (lemma, level) VALUES (?1, ?2)")?;
            for line in reader.lines() {
                let line = line?;
                let mut cols = line.split(['\t', ',']).map(str::trim);
                let (Some(lemma), Some(level)) = (cols.next(), cols.next()) else { continue };
                if lemma.is_empty() {
                    continue;
                }
                if let Some(level) = normalize_level(level) {
                    stmt.execute(params![lemma, level])?;
                    n += 1;
                }
            }
        }
        tx.commit()?;
        Ok(n)
    }
}

/// `1`..`6` → "TOPIK n"; 초급/중급/고급 → Beginner/Intermediate/Advanced.
pub fn normalize_level(raw: &str) -> Option<String> {
    let s = raw.trim();
    let digits: String = s.chars().filter(char::is_ascii_digit).collect();
    if !digits.is_empty() && digits.len() == 1 {
        let n: u8 = digits.parse().ok()?;
        if (1..=6).contains(&n) {
            return Some(format!("TOPIK {n}"));
        }
    }
    match s {
        "초급" | "beginner" | "Beginner" | "A" => Some("Beginner".into()),
        "중급" | "intermediate" | "Intermediate" | "B" => Some("Intermediate".into()),
        "고급" | "advanced" | "Advanced" | "C" => Some("Advanced".into()),
        _ => None,
    }
}
