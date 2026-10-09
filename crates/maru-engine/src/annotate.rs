//! Enrich a freshly analysed sentence with everything we already know offline:
//! dictionary meanings and levels, your vocabulary state, and where each word
//! sits on screen.

use maru_core::{Eojeol, WordBox, WordState};
use maru_dict::Dictionary;
use maru_srs::Vocab;

fn core(s: &str) -> &str {
    s.trim_matches(|c: char| !c.is_alphanumeric() && !maru_lang::hangul::is_hangul(c))
}

/// Attach on-screen boxes to words. With equal counts they pair in order; otherwise
/// each word is matched to the next unused box with the same text.
pub fn assign_rects(eojeols: &mut [Eojeol], words: &[WordBox]) {
    if words.is_empty() {
        return;
    }
    if words.len() == eojeols.len() {
        for (e, w) in eojeols.iter_mut().zip(words) {
            e.rect = Some(w.rect);
        }
        return;
    }
    let mut next = 0;
    for e in eojeols.iter_mut() {
        let target = core(&e.surface);
        if let Some(offset) = words[next..].iter().position(|w| core(&w.text) == target) {
            e.rect = Some(words[next + offset].rect);
            next += offset + 1;
        }
    }
}

/// Fill dictionary glosses/levels and vocabulary state.
pub fn annotate(eojeols: &mut [Eojeol], dict: Option<&Dictionary>, vocab: Option<&Vocab>, max_glosses: usize) {
    if let Some(dict) = dict.filter(|d| !d.is_empty()) {
        for e in eojeols.iter_mut().filter(|e| e.content) {
            match dict.gloss_and_level(e, max_glosses) {
                Ok((glosses, level)) => {
                    e.dict_glosses = glosses;
                    e.level = level;
                }
                Err(err) => tracing::warn!("dictionary lookup failed for {}: {err}", e.surface),
            }
        }
    }
    if let Some(vocab) = vocab {
        let keys: Vec<String> = eojeols.iter().filter(|e| e.content).map(|e| e.vocab_key().to_string()).collect();
        let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
        match vocab.get_many(&refs) {
            Ok(known) => {
                for e in eojeols.iter_mut().filter(|e| e.content) {
                    if let Some(rec) = known.get(e.vocab_key()) {
                        e.state = rec.state;
                        e.lookups = rec.lookups;
                    } else {
                        e.state = WordState::New;
                    }
                }
            }
            Err(err) => tracing::warn!("vocabulary lookup failed: {err}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use maru_core::Rect;
    use maru_lang::Analyzer;
    use maru_srs::SrsConfig;

    use super::*;

    fn wb(t: &str, x: f32) -> WordBox {
        WordBox { text: t.into(), rect: Rect::new(x, 0.0, 10.0, 10.0) }
    }

    #[test]
    fn rects_pair_in_order_when_counts_match() {
        let mut e = Analyzer::heuristic().analyze("학교에서 공부했어요").eojeols;
        assign_rects(&mut e, &[wb("학교에서", 0.0), wb("공부했어요", 50.0)]);
        assert_eq!(e[0].rect.unwrap().x, 0.0);
        assert_eq!(e[1].rect.unwrap().x, 50.0);
    }

    #[test]
    fn rects_match_by_text_when_counts_differ() {
        let mut e = Analyzer::heuristic().analyze("저는 학교에 가요").eojeols;
        // OCR split "가요" differently and produced a stray box
        assign_rects(&mut e, &[wb("[", 0.0), wb("저는", 10.0), wb("학교에", 20.0), wb("가요.", 30.0), wb("x", 40.0)]);
        assert_eq!(e[0].rect.unwrap().x, 10.0);
        assert_eq!(e[1].rect.unwrap().x, 20.0);
        assert_eq!(e[2].rect.unwrap().x, 30.0, "trailing punctuation is ignored when matching");
    }

    #[test]
    fn unmatched_words_stay_without_a_box() {
        let mut e = Analyzer::heuristic().analyze("가 나 다").eojeols;
        assign_rects(&mut e, &[wb("가", 0.0), wb("다", 9.0)]);
        assert!(e[0].rect.is_some() && e[1].rect.is_none() && e[2].rect.is_some());
    }

    #[test]
    fn annotation_adds_dictionary_and_vocabulary_state() {
        let dict = Dictionary::open_in_memory().unwrap();
        dict.import_kaikki(
            Cursor::new(r#"{"word":"학교","lang_code":"ko","pos":"noun","senses":[{"glosses":["school"]}]}"#),
            |_| {},
        )
        .unwrap();
        dict.import_levels(Cursor::new("학교\t1\n")).unwrap();
        let vocab = Vocab::open_in_memory(SrsConfig::default()).unwrap();
        vocab.mark_known("학교", true, 0).unwrap();

        let mut e = Analyzer::heuristic().analyze("학교에서 공부했어요").eojeols;
        annotate(&mut e, Some(&dict), Some(&vocab), 3);
        assert_eq!(e[0].dict_glosses, ["school"]);
        assert_eq!(e[0].level.as_deref(), Some("TOPIK 1"));
        assert_eq!(e[0].state, WordState::Known);
        assert_eq!(e[1].state, WordState::New);
        assert!(e[1].dict_glosses.is_empty());
    }

    #[test]
    fn annotation_tolerates_missing_or_empty_sources() {
        let mut e = Analyzer::heuristic().analyze("학교").eojeols;
        annotate(&mut e, None, None, 3);
        annotate(&mut e, Some(&Dictionary::open_in_memory().unwrap()), None, 3);
        assert!(e[0].dict_glosses.is_empty());
    }
}
