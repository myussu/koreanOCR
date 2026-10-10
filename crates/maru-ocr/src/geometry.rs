//! Pure geometry/text helpers (no ONNX): word boxes from per-character boxes,
//! and reading order for detected lines.

use maru_core::{Rect, WordBox};

use crate::OcrLine;

/// Relative width of a character, used when no per-character boxes exist.
fn char_weight(c: char) -> f32 {
    if c.is_whitespace() {
        0.35
    } else if crate_is_wide(c) {
        1.0
    } else if c.is_ascii_uppercase() || c.is_ascii_digit() {
        0.62
    } else if matches!(c, 'i' | 'l' | 'j' | 't' | 'f' | 'I' | '.' | ',' | ':' | ';' | '\'' | '!' | '|') {
        0.3
    } else if c.is_ascii() {
        0.52
    } else {
        1.0
    }
}

fn crate_is_wide(c: char) -> bool {
    matches!(c as u32, 0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7A3 | 0x4E00..=0x9FFF | 0x3040..=0x30FF | 0xFF00..=0xFFEF)
}

/// Split a recognised line into space-delimited words with boxes.
///
/// * `char_boxes` has one box per character (spaces included) → exact;
/// * one box per *non-space* character → exact;
/// * otherwise the line box is divided proportionally to typical glyph widths.
pub fn words_from_chars(text: &str, line: Rect, char_boxes: &[Rect]) -> Vec<WordBox> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return Vec::new();
    }
    let non_space = chars.iter().filter(|c| !c.is_whitespace()).count();

    // Box for every character index (None for spaces when only non-space boxes exist).
    let per_char: Vec<Option<Rect>> = if char_boxes.len() == chars.len() {
        char_boxes.iter().copied().map(Some).collect()
    } else if char_boxes.len() == non_space && non_space > 0 {
        let mut it = char_boxes.iter().copied();
        chars.iter().map(|c| if c.is_whitespace() { None } else { it.next() }).collect()
    } else {
        proportional(&chars, line)
    };

    let mut words = Vec::new();
    let mut cur = String::new();
    let mut rect: Option<Rect> = None;
    let flush = |cur: &mut String, rect: &mut Option<Rect>, words: &mut Vec<WordBox>| {
        if let (false, Some(r)) = (cur.is_empty(), rect.take()) {
            words.push(WordBox { text: std::mem::take(cur), rect: r });
        }
        cur.clear();
    };
    for (i, (c, b)) in chars.iter().zip(per_char).enumerate() {
        if c.is_whitespace() {
            flush(&mut cur, &mut rect, &mut words);
        } else {
            cur.push(*c);
            if let Some(b) = b {
                rect = Some(rect.map_or(b, |r| r.union(&b)));
            }
            // PP-OCR often drops the space after sentence punctuation ("정한다.정기회의").
            if matches!(c, '.' | ',' | '!' | '?' | ';') && chars.get(i + 1).is_some_and(|n| ('가'..='힣').contains(n)) {
                flush(&mut cur, &mut rect, &mut words);
            }
        }
    }
    flush(&mut cur, &mut rect, &mut words);
    words
}

/// Re-divide recognised words to follow `respaced`, a copy of the same text with spaces
/// *inserted* (Kiwi's spacing restoration). Each split word's box is shared out by glyph width.
///
/// A new break is kept only if `accept(left)` says the text before it can stand as a finished
/// word; otherwise the pieces on both sides stay joined. (Kiwi will also split compound nouns
/// like 정기회의 and even single syllables, which is not what we want from OCR repair.)
///
/// Returns `None` unless `respaced` is exactly the words with extra breaks, so a spacer that
/// also deleted, merged or rewrote anything can never corrupt the boxes.
pub fn resplit_words(words: &[WordBox], respaced: &str, accept: &dyn Fn(&str) -> bool) -> Option<Vec<WordBox>> {
    let tokens: Vec<&str> = respaced.split_whitespace().collect();
    let mut next = 0;
    let mut out = Vec::with_capacity(words.len());
    for word in words {
        let mut pieces: Vec<&str> = Vec::new();
        let mut joined = String::new();
        while joined.len() < word.text.len() {
            let t = tokens.get(next)?;
            joined.push_str(t);
            pieces.push(t);
            next += 1;
        }
        if joined != word.text {
            return None;
        }
        // Merge pieces back together wherever the break would not be a word boundary.
        let mut groups: Vec<String> = Vec::new();
        let mut current = String::new();
        for (i, piece) in pieces.iter().enumerate() {
            current.push_str(piece);
            if i + 1 == pieces.len() || accept(&current) {
                groups.push(std::mem::take(&mut current));
            }
        }
        if groups.len() == 1 {
            out.push(word.clone());
            continue;
        }
        let chars: Vec<char> = word.text.chars().collect();
        let boxes = proportional(&chars, word.rect);
        let mut at = 0;
        for group in groups {
            let n = group.chars().count();
            let rect = boxes[at..at + n].iter().flatten().copied().reduce(|a, b| a.union(&b))?;
            out.push(WordBox { text: group, rect });
            at += n;
        }
    }
    (next == tokens.len()).then_some(out)
}

/// New wording for the same line, keeping the old boxes: `new_text` must have exactly as many
/// non-space characters as the words had (a re-reading that fixed letters but did not add or lose
/// any). Each character keeps the slot it had, so words follow the new spacing without losing the
/// engine's own positions. `None` when the lengths differ.
pub fn regroup_words(old: &[WordBox], new_text: &str) -> Option<Vec<WordBox>> {
    let slots: Vec<Rect> = old
        .iter()
        .flat_map(|w| {
            let chars: Vec<char> = w.text.chars().collect();
            proportional(&chars, w.rect).into_iter().flatten().collect::<Vec<_>>()
        })
        .collect();
    let mut at = 0;
    let mut out = Vec::new();
    for word in new_text.split_whitespace() {
        let n = word.chars().count();
        let rect = slots.get(at..at + n)?.iter().copied().reduce(|a, b| a.union(&b))?;
        out.push(WordBox { text: word.to_string(), rect });
        at += n;
    }
    (at == slots.len()).then_some(out)
}

fn proportional(chars: &[char], line: Rect) -> Vec<Option<Rect>> {
    let total: f32 = chars.iter().map(|&c| char_weight(c)).sum();
    if total <= 0.0 {
        return vec![None; chars.len()];
    }
    let mut x = line.x;
    chars
        .iter()
        .map(|&c| {
            let w = line.w * char_weight(c) / total;
            let r = Rect::new(x, line.y, w, line.h);
            x += w;
            Some(r)
        })
        .collect()
}

/// Sort lines top-to-bottom, then left-to-right within a row.
/// Lines whose vertical centres are within half a line height share a row.
pub fn reading_order(lines: &mut [OcrLine]) {
    lines.sort_by(|a, b| {
        let (ca, cb) = (a.rect.y + a.rect.h / 2.0, b.rect.y + b.rect.h / 2.0);
        let tolerance = a.rect.h.min(b.rect.h) / 2.0;
        if (ca - cb).abs() <= tolerance {
            a.rect.x.total_cmp(&b.rect.x)
        } else {
            ca.total_cmp(&cb)
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: f32) -> Rect {
        Rect::new(x, 0.0, 10.0, 20.0)
    }

    #[test]
    fn exact_boxes_when_every_char_including_spaces_has_one() {
        let text = "한국 어";
        let boxes = [r(0.0), r(10.0), r(20.0), r(30.0)];
        let w = words_from_chars(text, Rect::new(0.0, 0.0, 40.0, 20.0), &boxes);
        assert_eq!(w.len(), 2);
        assert_eq!((w[0].text.as_str(), w[0].rect), ("한국", Rect::new(0.0, 0.0, 20.0, 20.0)));
        assert_eq!((w[1].text.as_str(), w[1].rect), ("어", r(30.0)));
    }

    #[test]
    fn exact_boxes_when_only_non_space_chars_have_one() {
        let w = words_from_chars("가나 다", Rect::new(0.0, 0.0, 40.0, 20.0), &[r(0.0), r(10.0), r(30.0)]);
        assert_eq!(w.iter().map(|x| x.text.as_str()).collect::<Vec<_>>(), ["가나", "다"]);
        assert_eq!(w[1].rect, r(30.0));
    }

    #[test]
    fn proportional_fallback_splits_the_line_box() {
        let line = Rect::new(100.0, 50.0, 300.0, 24.0);
        let w = words_from_chars("저는 학생입니다", line, &[]);
        assert_eq!(w.len(), 2);
        // words tile the line without leaving it
        assert!(w[0].rect.x >= 100.0 - 0.01 && w[1].rect.right() <= 400.0 + 0.01);
        assert!(w[0].rect.right() <= w[1].rect.x + 0.01);
        // the longer word gets the larger box
        assert!(w[1].rect.w > w[0].rect.w);
    }

    #[test]
    fn mixed_scripts_get_sensible_widths() {
        let w = words_from_chars("AI 모델", Rect::new(0.0, 0.0, 200.0, 20.0), &[]);
        assert_eq!(w.len(), 2);
        assert!(w[0].rect.w < w[1].rect.w, "two Latin capitals are narrower than two Hangul syllables");
    }

    #[test]
    fn missing_space_after_sentence_punctuation_is_restored_but_numbers_and_domains_are_not() {
        let line = Rect { x: 0.0, y: 0.0, w: 300.0, h: 20.0 };
        let texts = |s: &str| words_from_chars(s, line, &[]).into_iter().map(|w| w.text).collect::<Vec<_>>();
        assert_eq!(texts("정한다.정기회의 100일을,임시회의"), ["정한다.", "정기회의", "100일을,", "임시회의"]);
        assert_eq!(texts("3.14 www.naver.com"), ["3.14", "www.naver.com"]);
    }

    #[test]
    fn respacing_splits_a_word_box_and_rejects_anything_but_insertions() {
        let w = |t: &str, x: f32, width: f32| WordBox { text: t.into(), rect: Rect::new(x, 0.0, width, 20.0) };
        let words = vec![w("천까지승차해", 0.0, 120.0), w("주시기", 130.0, 60.0)];
        let any = |_: &str| true;
        let out = resplit_words(&words, "천까지 승차해 주시기", &any).unwrap();
        assert_eq!(out.iter().map(|b| b.text.as_str()).collect::<Vec<_>>(), ["천까지", "승차해", "주시기"]);
        // the pieces tile the original box, in order
        assert!((out[0].rect.x - 0.0).abs() < 0.01 && (out[1].rect.right() - 120.0).abs() < 0.01);
        assert!(out[0].rect.right() <= out[1].rect.x + 0.01);
        assert_eq!(out[2].rect, words[1].rect);
        // unchanged text is accepted as is; anything that deletes, merges or rewrites is not
        assert_eq!(resplit_words(&words, "천까지승차해 주시기", &any).unwrap(), words);
        assert!(resplit_words(&words, "천까지승차해주시기", &any).is_none());
        assert!(resplit_words(&words, "전까지 승차해 주시기", &any).is_none());
        assert!(resplit_words(&words, "천까지 승차해", &any).is_none());
    }

    #[test]
    fn breaks_the_caller_rejects_are_merged_back() {
        let words = vec![WordBox { text: "d한민국".into(), rect: Rect::new(0.0, 0.0, 80.0, 20.0) }];
        // only "…까지"-style endings count as a word end here
        let out = resplit_words(&words, "d 한 민 국", &|left: &str| left == "d").unwrap();
        assert_eq!(out.iter().map(|b| b.text.as_str()).collect::<Vec<_>>(), ["d", "한민국"]);
        assert_eq!(resplit_words(&words, "d 한 민 국", &|_: &str| false).unwrap(), words);
    }

    #[test]
    fn regrouping_keeps_the_old_positions_when_only_letters_and_spacing_change() {
        let w = |t: &str, x: f32, width: f32| WordBox { text: t.into(), rect: Rect::new(x, 0.0, width, 20.0) };
        let old = vec![w("전금도", 0.0, 60.0), w("충분히", 100.0, 60.0)];
        let new = regroup_words(&old, "지 금도 충분히").unwrap();
        assert_eq!(new.iter().map(|b| b.text.as_str()).collect::<Vec<_>>(), ["지", "금도", "충분히"]);
        assert!((new[0].rect.w - 20.0).abs() < 0.01 && (new[1].rect.x - 20.0).abs() < 0.01 && (new[1].rect.w - 40.0).abs() < 0.01);
        assert_eq!(new[2].rect, old[1].rect, "an untouched word keeps its box exactly");
        assert!(regroup_words(&old, "지금도 충분").is_none(), "a different number of characters cannot be mapped");
        assert!(regroup_words(&old, "지금도 충분히 요").is_none());
    }

    #[test]
    fn empty_and_blank_text() {
        assert!(words_from_chars("", Rect::default(), &[]).is_empty());
        assert!(words_from_chars("   ", Rect::new(0.0, 0.0, 10.0, 10.0), &[]).is_empty());
    }

    #[test]
    fn reading_order_groups_rows_then_columns() {
        let mk = |t: &str, x: f32, y: f32| OcrLine { text: t.into(), rect: Rect::new(x, y, 50.0, 20.0), confidence: 1.0, words: vec![] };
        let mut lines = vec![mk("c", 0.0, 100.0), mk("b", 200.0, 3.0), mk("a", 0.0, 0.0)];
        reading_order(&mut lines);
        assert_eq!(lines.iter().map(|l| l.text.as_str()).collect::<String>(), "abc");
    }
}
