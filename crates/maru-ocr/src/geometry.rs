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
    for (c, b) in chars.iter().zip(per_char) {
        if c.is_whitespace() {
            flush(&mut cur, &mut rect, &mut words);
        } else {
            cur.push(*c);
            if let Some(b) = b {
                rect = Some(rect.map_or(b, |r| r.union(&b)));
            }
        }
    }
    flush(&mut cur, &mut rect, &mut words);
    words
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
