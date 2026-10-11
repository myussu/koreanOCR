//! Joining pieces of screen text that belong together.
//!
//! The OCR detector sometimes cuts one phrase into two boxes (a wide gap, a decorative separator,
//! two text colours). The scan then labels each half on its own. The person can join them by hand;
//! [`join_pieces`] builds the single item, and the join is remembered as a [`JoinRule`] per program so
//! [`apply_rules`] makes the same join on later scans of the same screen.

use maru_core::Rect;

use crate::labels::key;
use crate::scan::Found;

/// A label on screen that the person picked to join: its text and where it is (physical pixels).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct JoinPiece {
    pub text: String,
    pub rect: Rect,
}

/// A remembered join: when lines whose [`key`]s are `parts` sit next to each other, in this order,
/// they become one line with text `joined`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinRule {
    pub parts: Vec<String>,
    pub joined: String,
}

/// Pieces closer than this (as a fraction of the text height) are one word: no space between them.
const SAME_WORD_GAP: f32 = 0.25;
/// Two lines count as "next to each other" up to this many text heights apart.
const NEAR_GAP: f32 = 4.0;

fn height(a: &Rect, b: &Rect) -> f32 {
    a.h.max(b.h).max(1.0)
}

/// Reading order: rows from top to bottom, each row from left to right. A piece joins the current row
/// when its vertical middle lies within the row's first piece.
pub(crate) fn reading_order(mut pieces: Vec<Found>) -> Vec<Found> {
    pieces.sort_by(|a, b| (a.rect.y + a.rect.h / 2.0).total_cmp(&(b.rect.y + b.rect.h / 2.0)));
    let mut rows: Vec<Vec<Found>> = Vec::new();
    for p in pieces {
        let middle = p.rect.y + p.rect.h / 2.0;
        match rows.last_mut() {
            Some(row) if middle >= row[0].rect.y && middle <= row[0].rect.bottom() => row.push(p),
            _ => rows.push(vec![p]),
        }
    }
    rows.into_iter()
        .flat_map(|mut row| {
            row.sort_by(|a, b| a.rect.x.total_cmp(&b.rect.x));
            row
        })
        .collect()
}

/// One item made of `pieces`: their text in reading order and the rectangle around all of them.
/// Within a row a space goes between pieces only when the gap is wider than a letter gap; between rows always.
pub(crate) fn join_pieces(pieces: Vec<Found>) -> Option<Found> {
    let ordered = reading_order(pieces);
    let first = ordered.first()?;
    let mut rect = first.rect;
    let mut text = first.text.clone();
    for pair in ordered.windows(2) {
        let (prev, next) = (&pair[0], &pair[1]);
        let same_row = next.rect.y + next.rect.h / 2.0 <= prev.rect.bottom() && next.rect.y + next.rect.h / 2.0 >= prev.rect.y;
        let gap = next.rect.x - prev.rect.right();
        if !same_row || gap > SAME_WORD_GAP * height(&prev.rect, &next.rect) {
            text.push(' ');
        }
        text.push_str(&next.text);
        rect = rect.union(&next.rect);
    }
    Some(Found { rect, text })
}

/// Is `b` right after `a` in reading order and close to it: on the same row to its right, or just below it?
pub(crate) fn follows(a: &Rect, b: &Rect) -> bool {
    let h = height(a, b);
    let vertical_overlap = (a.bottom().min(b.bottom()) - a.y.max(b.y)).max(0.0);
    let same_row = vertical_overlap >= 0.5 * a.h.min(b.h);
    if same_row {
        let gap = b.x - a.right();
        return gap >= -0.5 * h && gap <= NEAR_GAP * h;
    }
    let horizontal_overlap = (a.right().min(b.right()) - a.x.max(b.x)).max(0.0);
    let below = b.y - a.bottom();
    below >= -0.5 * h && below <= 1.5 * h && horizontal_overlap >= 0.3 * a.w.min(b.w)
}

/// Make the remembered joins on a fresh reading of the screen. Each rule is tried against every line that
/// starts it; a chain of lines that match the rule's parts, each following the one before, becomes one line
/// at the place of the first. Rules apply in the order given, so a join of joined lines works.
pub(crate) fn apply_rules(mut found: Vec<Found>, rules: &[JoinRule]) -> Vec<Found> {
    for rule in rules {
        let Some((first, rest)) = rule.parts.split_first() else { continue };
        let mut start = 0;
        while start < found.len() {
            if key(&found[start].text) != *first {
                start += 1;
                continue;
            }
            // Walk the chain: the nearest following line with the next key.
            let mut chain = vec![start];
            for want in rest {
                let last = found[*chain.last().unwrap_or(&start)].rect;
                let next = found
                    .iter()
                    .enumerate()
                    .filter(|(i, f)| !chain.contains(i) && key(&f.text) == *want && follows(&last, &f.rect))
                    .min_by(|a, b| (a.1.rect.x - last.right()).abs().total_cmp(&(b.1.rect.x - last.right()).abs()).then((a.1.rect.y - last.bottom()).abs().total_cmp(&(b.1.rect.y - last.bottom()).abs())))
                    .map(|(i, _)| i);
                match next {
                    Some(i) => chain.push(i),
                    None => break,
                }
            }
            if chain.len() != rule.parts.len() {
                start += 1;
                continue;
            }
            let rect = chain.iter().map(|i| found[*i].rect).reduce(|a, b| a.union(&b)).unwrap_or(found[start].rect);
            found[start] = Found { rect, text: rule.joined.clone() };
            // Remove the other pieces, highest index first so the rest keep their places.
            let mut gone: Vec<usize> = chain[1..].to_vec();
            gone.sort_unstable_by(|a, b| b.cmp(a));
            for i in gone {
                found.remove(i);
            }
            let moved_before_start = chain[1..].iter().filter(|i| **i < start).count();
            start = start.saturating_sub(moved_before_start) + 1;
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(text: &str, x: f32, y: f32, w: f32, h: f32) -> Found {
        Found { rect: Rect::new(x, y, w, h), text: text.into() }
    }

    fn texts(found: &[Found]) -> Vec<&str> {
        found.iter().map(|f| f.text.as_str()).collect()
    }

    #[test]
    fn pieces_are_joined_in_reading_order_whatever_order_they_were_picked_in() {
        let joined = join_pieces(vec![f("게임", 150.0, 100.0, 60.0, 20.0), f("새", 100.0, 101.0, 40.0, 20.0)]).unwrap();
        assert_eq!(joined.text, "새 게임", "a word-sized gap becomes a space");
        assert_eq!(joined.rect, Rect::new(100.0, 100.0, 110.0, 21.0));
    }

    #[test]
    fn a_tiny_gap_means_one_word() {
        let joined = join_pieces(vec![f("게", 100.0, 100.0, 20.0, 20.0), f("임", 123.0, 100.0, 20.0, 20.0)]).unwrap();
        assert_eq!(joined.text, "게임", "3 px is less than a quarter of the text height");
        let spaced = join_pieces(vec![f("게", 100.0, 100.0, 20.0, 20.0), f("임", 128.0, 100.0, 20.0, 20.0)]).unwrap();
        assert_eq!(spaced.text, "게 임");
    }

    #[test]
    fn rows_come_top_to_bottom_and_always_get_a_space() {
        let joined = join_pieces(vec![f("둘째 줄", 100.0, 130.0, 80.0, 20.0), f("첫째", 100.0, 100.0, 50.0, 20.0), f("줄", 160.0, 102.0, 30.0, 20.0)]).unwrap();
        assert_eq!(joined.text, "첫째 줄 둘째 줄");
        assert_eq!(joined.rect, Rect::new(100.0, 100.0, 90.0, 50.0));
    }

    #[test]
    fn nothing_to_join_gives_nothing() {
        assert!(join_pieces(vec![]).is_none());
        assert_eq!(join_pieces(vec![f("혼자", 1.0, 2.0, 30.0, 20.0)]).unwrap().text, "혼자");
    }

    #[test]
    fn what_follows_what() {
        let a = Rect::new(100.0, 100.0, 60.0, 20.0);
        assert!(follows(&a, &Rect::new(170.0, 100.0, 60.0, 20.0)), "to the right, close");
        assert!(follows(&a, &Rect::new(165.0, 103.0, 60.0, 18.0)), "slightly off the baseline");
        assert!(!follows(&a, &Rect::new(400.0, 100.0, 60.0, 20.0)), "too far to the right");
        assert!(!follows(&Rect::new(170.0, 100.0, 60.0, 20.0), &a), "to the left is not 'after'");
        assert!(follows(&a, &Rect::new(100.0, 124.0, 60.0, 20.0)), "directly below");
        assert!(!follows(&a, &Rect::new(300.0, 124.0, 60.0, 20.0)), "below but somewhere else");
        assert!(!follows(&a, &Rect::new(100.0, 200.0, 60.0, 20.0)), "far below");
    }

    #[test]
    fn a_remembered_join_is_made_again() {
        let rules = vec![JoinRule { parts: vec!["새".into(), "게임".into()], joined: "새 게임".into() }];
        let found = vec![f("이어하기", 100.0, 60.0, 80.0, 20.0), f("새", 100.0, 100.0, 40.0, 20.0), f("게임", 150.0, 100.0, 60.0, 20.0), f("종료", 100.0, 140.0, 40.0, 20.0)];
        let out = apply_rules(found, &rules);
        assert_eq!(texts(&out), ["이어하기", "새 게임", "종료"]);
        assert_eq!(out[1].rect, Rect::new(100.0, 100.0, 110.0, 20.0));
    }

    #[test]
    fn pieces_that_are_not_next_to_each_other_are_left_alone() {
        let rules = vec![JoinRule { parts: vec!["새".into(), "게임".into()], joined: "새 게임".into() }];
        let found = vec![f("새", 100.0, 100.0, 40.0, 20.0), f("게임", 900.0, 600.0, 60.0, 20.0)];
        assert_eq!(texts(&apply_rules(found, &rules)), ["새", "게임"]);
    }

    #[test]
    fn the_same_join_is_made_in_every_place_it_occurs() {
        let rules = vec![JoinRule { parts: vec!["빨간".into(), "물약".into()], joined: "빨간 물약".into() }];
        let found = vec![f("빨간", 100.0, 100.0, 50.0, 20.0), f("물약", 160.0, 100.0, 50.0, 20.0), f("빨간", 100.0, 300.0, 50.0, 20.0), f("물약", 160.0, 300.0, 50.0, 20.0)];
        let out = apply_rules(found, &rules);
        assert_eq!(texts(&out), ["빨간 물약", "빨간 물약"]);
        assert_eq!((out[0].rect.y, out[1].rect.y), (100.0, 300.0), "each pair joined with its own neighbour");
    }

    #[test]
    fn rules_match_through_ocr_edge_punctuation() {
        let rules = vec![JoinRule { parts: vec!["새".into(), "게임".into()], joined: "새 게임".into() }];
        let found = vec![f("[새", 100.0, 100.0, 40.0, 20.0), f("게임:", 150.0, 100.0, 60.0, 20.0)];
        assert_eq!(texts(&apply_rules(found, &rules)), ["새 게임"]);
    }

    #[test]
    fn a_join_of_joins_works_when_the_rules_are_in_order() {
        let rules = vec![
            JoinRule { parts: vec!["새".into(), "게임".into()], joined: "새 게임".into() },
            JoinRule { parts: vec!["새 게임".into(), "시작".into()], joined: "새 게임 시작".into() },
        ];
        let found = vec![f("새", 100.0, 100.0, 30.0, 20.0), f("게임", 140.0, 100.0, 50.0, 20.0), f("시작", 200.0, 100.0, 50.0, 20.0)];
        let out = apply_rules(found, &rules);
        assert_eq!(texts(&out), ["새 게임 시작"]);
        assert_eq!(out[0].rect, Rect::new(100.0, 100.0, 150.0, 20.0));
    }

    #[test]
    fn a_three_part_rule_needs_all_three() {
        let rules = vec![JoinRule { parts: vec!["가".into(), "나".into(), "다".into()], joined: "가 나 다".into() }];
        let two = vec![f("가", 100.0, 100.0, 30.0, 20.0), f("나", 140.0, 100.0, 30.0, 20.0)];
        assert_eq!(texts(&apply_rules(two, &rules)), ["가", "나"]);
        let three = vec![f("가", 100.0, 100.0, 30.0, 20.0), f("나", 140.0, 100.0, 30.0, 20.0), f("다", 180.0, 100.0, 30.0, 20.0)];
        assert_eq!(texts(&apply_rules(three, &rules)), ["가 나 다"]);
    }
}
