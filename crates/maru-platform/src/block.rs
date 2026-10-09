//! Choose the block of text the user means from OCR lines.
//!
//! When nothing is selected, the user usually points at a sentence or
//! paragraph. We take the line nearest the cursor and extend over neighbouring
//! lines that visually belong to the same block (small gap, overlapping columns).

use maru_core::{Point, Rect};
use maru_ocr::OcrLine;

fn distance_to(r: &Rect, p: Point) -> f32 {
    let dx = (r.x - p.x).max(0.0).max(p.x - r.right());
    let dy = (r.y - p.y).max(0.0).max(p.y - r.bottom());
    (dx * dx + dy * dy).sqrt()
}

fn horizontal_overlap(a: &Rect, b: &Rect) -> f32 {
    let w = (a.right().min(b.right()) - a.x.max(b.x)).max(0.0);
    w / a.w.min(b.w).max(1.0)
}

/// Lines (in reading order) forming the block under/near `cursor`.
/// Returns an empty vec if no line is within `max_distance` of the cursor.
pub fn block_at(lines: &[OcrLine], cursor: Point, max_distance: f32) -> Vec<&OcrLine> {
    let mut ordered: Vec<&OcrLine> = lines.iter().filter(|l| !l.text.trim().is_empty()).collect();
    ordered.sort_by(|a, b| a.rect.y.total_cmp(&b.rect.y).then(a.rect.x.total_cmp(&b.rect.x)));
    let Some(seed) = ordered
        .iter()
        .enumerate()
        .map(|(i, l)| (i, distance_to(&l.rect, cursor)))
        .filter(|&(_, d)| d <= max_distance)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
    else {
        return Vec::new();
    };

    let belongs = |a: &OcrLine, b: &OcrLine| {
        let gap = if b.rect.y >= a.rect.y { b.rect.y - a.rect.bottom() } else { a.rect.y - b.rect.bottom() };
        let line_h = a.rect.h.min(b.rect.h).max(1.0);
        gap <= line_h * 0.9 && horizontal_overlap(&a.rect, &b.rect) >= 0.3
    };
    let (mut start, mut end) = (seed, seed);
    while start > 0 && belongs(ordered[start], ordered[start - 1]) {
        start -= 1;
    }
    while end + 1 < ordered.len() && belongs(ordered[end], ordered[end + 1]) {
        end += 1;
    }
    ordered[start..=end].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(t: &str, x: f32, y: f32, w: f32) -> OcrLine {
        OcrLine { text: t.into(), rect: Rect::new(x, y, w, 20.0), confidence: 0.9, words: vec![] }
    }

    fn texts<'a>(v: &[&'a OcrLine]) -> Vec<&'a str> {
        v.iter().map(|l| l.text.as_str()).collect()
    }

    #[test]
    fn wrapped_paragraph_is_taken_as_one_block() {
        let lines = vec![
            line("title", 10.0, 0.0, 80.0),
            line("first line of text", 10.0, 60.0, 300.0),
            line("second line of text", 10.0, 84.0, 290.0),
            line("third", 10.0, 108.0, 120.0),
            line("footer far below", 10.0, 300.0, 200.0),
        ];
        let block = block_at(&lines, Point { x: 100.0, y: 90.0 }, 50.0);
        assert_eq!(texts(&block), ["first line of text", "second line of text", "third"]);
    }

    #[test]
    fn side_by_side_columns_are_not_merged() {
        let lines = vec![line("left column", 10.0, 50.0, 200.0), line("right column", 600.0, 52.0, 200.0)];
        let block = block_at(&lines, Point { x: 50.0, y: 60.0 }, 50.0);
        assert_eq!(texts(&block), ["left column"]);
    }

    #[test]
    fn nothing_near_the_cursor_gives_nothing() {
        let lines = vec![line("far away", 500.0, 500.0, 100.0)];
        assert!(block_at(&lines, Point { x: 0.0, y: 0.0 }, 100.0).is_empty());
        assert!(block_at(&[], Point::default(), 100.0).is_empty());
    }

    #[test]
    fn nearest_line_wins_when_between_two_blocks() {
        let lines = vec![line("above", 10.0, 0.0, 100.0), line("below", 10.0, 200.0, 100.0)];
        let block = block_at(&lines, Point { x: 50.0, y: 170.0 }, 100.0);
        assert_eq!(texts(&block), ["below"]);
    }
}
