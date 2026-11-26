//! English "equivalents" that are only the headword in Latin letters.
//!
//! The learner's dictionary gives some words no English word at all, only their romanization: 녀석 → "nyeoseok",
//! the 수 of 할 수 있다 → "su", 고추장 → "gochujang", the auxiliary 않다 → "anta". On a card that is no meaning.
//! A sense whose English is such a spelling takes the gloss the dictionary puts at the start of its definition
//! ("red chili paste: A unique Korean condiment…") when it has one; otherwise it is left without English, and
//! the card's meaning then comes from a model, like a word the dictionary lacks.
//!
//! The rule is deliberately narrow: only an exact match with a way the dictionary spells the headword counts,
//! capitalised equivalents are names (Seoul) and stay, and a word both languages borrowed (라디오 → "radio",
//! Spanish "radio") stays too.

use maru_dict::{Nikl, NiklSense};
use maru_lang::pronounce::{pronounce_word, romanize_word};
use serde::Serialize;

fn letters(s: &str) -> String {
    s.chars().filter(char::is_ascii_alphabetic).map(|c| c.to_ascii_lowercase()).collect()
}

/// The headword without the linking ㅅ the dictionary's spelling leaves out (봇짐 → bojim): the final ㅅ of
/// every syllable but the last.
fn without_linking_s(word: &str) -> String {
    const BASE: u32 = 0xAC00;
    const FINAL_S: u32 = 19;
    let count = word.chars().count();
    word.chars()
        .enumerate()
        .map(|(i, c)| {
            let code = c as u32;
            let syllable = (BASE..=0xD7A3).contains(&code);
            if syllable && i + 1 < count && (code - BASE) % 28 == FINAL_S { char::from_u32(code - FINAL_S).unwrap_or(c) } else { c }
        })
        .collect()
}

/// The ways the dictionary writes `headword` in Latin letters: as it is spelled, as it sounds, syllable by
/// syllable, and without a linking ㅅ.
fn spellings(headword: &str) -> Vec<String> {
    let by_syllable: String = headword.chars().map(|c| letters(&romanize_word(&c.to_string()))).collect();
    let mut all = vec![
        letters(&romanize_word(headword)),
        letters(&romanize_word(&pronounce_word(headword))),
        by_syllable,
        letters(&romanize_word(&without_linking_s(headword))),
    ];
    all.retain(|s| !s.is_empty());
    all.dedup();
    all
}

/// Is `part` (one equivalent between semicolons) only `headword` written in Latin letters? Hyphens and spaces do
/// not matter ("-su", "su-"); a capitalised word is a name and is not one.
pub fn is_romanization(headword: &str, part: &str) -> bool {
    let part = part.trim();
    let spelled = letters(part);
    !spelled.is_empty() && !part.chars().find(|c| c.is_alphabetic()).is_some_and(char::is_uppercase) && spellings(headword).contains(&spelled)
}

/// The gloss the dictionary puts at the start of a definition: `red chili paste: A unique…` → `red chili paste`.
pub fn definition_gloss(definition: &str) -> Option<&str> {
    let (head, rest) = definition.split_once(": ")?;
    let head = head.trim();
    (!head.is_empty() && head.len() <= 60 && !head.contains(['.', '(', '"']) && !rest.trim().is_empty()).then_some(head)
}

/// What became of a sense's English equivalent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum English {
    /// It had real words (perhaps beside a romanization, which is dropped).
    Kept,
    /// Only a romanization: the definition's own gloss stands in.
    FromDefinition,
    /// Only a romanization that Spanish has too (radio, hotel): a borrowed word, kept.
    Borrowed,
    /// Only a romanization and nothing to replace it with.
    Nothing,
}

/// The English to show for a sense of `headword`, never a bare romanization (empty when there is none), and how
/// it was arrived at.
pub fn usable_english(headword: &str, sense: &NiklSense) -> (String, English) {
    let parts: Vec<&str> = sense.en.split(';').map(str::trim).filter(|p| !p.is_empty()).collect();
    let real: Vec<&str> = parts.iter().copied().filter(|p| !is_romanization(headword, p)).collect();
    if real.len() == parts.len() {
        return (sense.en.clone(), English::Kept);
    }
    if !real.is_empty() {
        return (real.join("; "), English::Kept);
    }
    if let Some(gloss) = definition_gloss(&sense.en_definition) {
        return (gloss.to_string(), English::FromDefinition);
    }
    let spanish: Vec<String> = sense.es.split([',', ';']).map(letters).collect();
    if parts.iter().all(|p| spanish.contains(&letters(p))) {
        return (sense.en.clone(), English::Borrowed);
    }
    (String::new(), English::Nothing)
}

/// The Spanish to show: the romanization some senses list first is dropped when real words follow it
/// ("gochujang, pasta de chile coreana" → "pasta de chile coreana").
pub fn usable_spanish(headword: &str, sense: &NiklSense) -> String {
    let parts: Vec<&str> = sense.es.split([',', ';']).map(str::trim).filter(|p| !p.is_empty()).collect();
    let real: Vec<&str> = parts.iter().copied().filter(|p| !is_romanization(headword, p)).collect();
    if real.is_empty() || real.len() == parts.len() { sense.es.clone() } else { real.join(", ") }
}

/// How much of a dictionary the rule touches (`maru dict romanized`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct RomanizedCount {
    pub senses: usize,
    pub entries: usize,
    /// Senses whose English had a romanization in it.
    pub affected_senses: usize,
    /// Entries with at least one such sense.
    pub affected_entries: usize,
    /// … where real English words were left once it was dropped.
    pub kept_other_words: usize,
    /// … that took the gloss at the start of their definition.
    pub from_definition: usize,
    /// … kept as a word both languages borrowed.
    pub borrowed: usize,
    /// … left without English: a model words the card.
    pub nothing: usize,
    /// Entries left without any English at all.
    pub entries_without_english: usize,
}

/// Walk the whole dictionary and count what the rule does to it.
pub fn count(nikl: &Nikl) -> maru_dict::Result<RomanizedCount> {
    let mut out = RomanizedCount::default();
    let mut entry = (-1i64, false, false); // id, has an affected sense, has usable English
    let close = |entry: &(i64, bool, bool), out: &mut RomanizedCount| {
        if entry.0 >= 0 {
            out.entries += 1;
            out.affected_entries += usize::from(entry.1);
            out.entries_without_english += usize::from(entry.1 && !entry.2);
        }
    };
    nikl.each_sense(|id, headword, sense| {
        if id != entry.0 {
            close(&entry, &mut out);
            entry = (id, false, false);
        }
        out.senses += 1;
        let had_romanization = sense.en.split(';').any(|p| is_romanization(headword, p));
        let (english, how) = usable_english(headword, sense);
        entry.2 |= !english.is_empty();
        if had_romanization {
            entry.1 = true;
            out.affected_senses += 1;
            match how {
                English::Kept => out.kept_other_words += 1,
                English::FromDefinition => out.from_definition += 1,
                English::Borrowed => out.borrowed += 1,
                English::Nothing => out.nothing += 1,
            }
        }
    })?;
    close(&entry, &mut out);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sense(en: &str, es: &str, definition: &str) -> NiklSense {
        NiklSense { en: en.into(), es: es.into(), en_definition: definition.into(), es_definition: String::new(), pos: "명사".into(), level: String::new(), homonym: 0 }
    }

    #[test]
    fn a_headword_written_in_latin_letters_is_recognised_however_the_dictionary_spells_it() {
        for (word, spelled) in [("녀석", "nyeoseok"), ("수", "su"), ("수", "su-"), ("수", "-su"), ("내", "nae"), ("않다", "anta"), ("고추장", "gochujang"), ("국", "guk"), ("거나", "-geona"), ("갈비탕", "galbitang")] {
            assert!(is_romanization(word, spelled), "{word} → {spelled}");
        }
        assert!(is_romanization("식혜", "sikhye"), "syllable by syllable, as the dictionary writes it");
        for (word, real) in [("수", "male"), ("국", "soup"), ("서울", "Seoul"), ("카메라", "camera"), ("학교", "school"), ("수", ""), ("배", "pear")] {
            assert!(!is_romanization(word, real), "{word} → {real} is a word, or a name");
        }
    }

    #[test]
    fn a_romanized_equivalent_is_replaced_by_the_definitions_gloss_or_left_empty_and_real_words_are_kept() {
        let chili = sense("gochujang", "gochujang, pasta de chile coreana", "red chili paste: A unique Korean condiment of dark red color.");
        assert_eq!(usable_english("고추장", &chili), ("red chili paste".to_string(), English::FromDefinition));
        assert_eq!(usable_spanish("고추장", &chili), "pasta de chile coreana");
        let guy = sense("nyeoseok", "", "(impolite form) A bound noun used to refer to a man.");
        assert_eq!(usable_english("녀석", &guy), (String::new(), English::Nothing), "nothing to show: a model words this card");
        assert_eq!(usable_spanish("녀석", &guy), "");
        let kimchi = sense("gimchi; kimchi", "kimchi", "A staple of Korean food.");
        assert_eq!(usable_english("김치", &kimchi), ("kimchi".to_string(), English::Kept), "the real word beside the romanization stays");
        let radio = sense("radio", "radio", "A mechanical device that receives radio waves.");
        assert_eq!(usable_english("라디오", &radio), ("radio".to_string(), English::Borrowed), "a word both languages borrowed is a meaning");
        let school = sense("school", "escuela", "An institution.");
        assert_eq!(usable_english("학교", &school), ("school".to_string(), English::Kept));
        assert_eq!(usable_spanish("학교", &school), "escuela");
        assert_eq!(usable_english("서울", &sense("Seoul", "Seúl", "A metropolitan city.")).1, English::Kept, "a name is not a romanization to hide");
    }

    #[test]
    fn only_a_short_gloss_before_a_colon_counts_as_the_definitions_own_word() {
        assert_eq!(definition_gloss("soup: A dish made by boiling meat."), Some("soup"));
        assert_eq!(definition_gloss("wail; lament: To cry loudly."), Some("wail; lament"));
        assert_eq!(definition_gloss("A bound noun used to refer to a man."), None);
        assert_eq!(definition_gloss("(honorific) 있다: to be"), None, "a note in brackets is not a gloss");
        assert_eq!(definition_gloss("A prefix used to mean \"not\": it negates."), None);
        assert_eq!(definition_gloss(&format!("{}: x", "long ".repeat(20))), None);
    }

    #[test]
    fn the_whole_dictionary_is_counted_entry_by_entry() {
        let nikl = Nikl::open_in_memory().unwrap();
        let file = r#"{"w":"고추장","h":0,"p":"명사","l":"","s":[{"en":"gochujang","es":"gochujang, pasta de chile","end":"red chili paste: A condiment."}]}
{"w":"녀석","h":0,"p":"의존 명사","l":"","s":[{"en":"nyeoseok","end":"A bound noun used to refer to a man."},{"en":"nyeoseok","end":"A bound noun used to refer to a boy."}]}
{"w":"국","h":0,"p":"명사","l":"","s":[{"en":"guk","es":"sopa","end":"soup: A dish."},{"en":"soup","es":"sopa","end":"Liquid."}]}
{"w":"라디오","h":0,"p":"명사","l":"","s":[{"en":"radio","es":"radio","end":"A device."}]}
{"w":"학교","h":0,"p":"명사","l":"","s":[{"en":"school","es":"escuela"}]}
"#;
        nikl.import_jsonl(std::io::Cursor::new(file)).unwrap();
        let c = count(&nikl).unwrap();
        assert_eq!((c.entries, c.senses), (5, 7));
        assert_eq!((c.affected_senses, c.affected_entries), (5, 4));
        assert_eq!((c.from_definition, c.nothing, c.borrowed, c.kept_other_words), (2, 2, 1, 0));
        assert_eq!(c.entries_without_english, 1, "only 녀석 is left with nothing to show");
    }
}
