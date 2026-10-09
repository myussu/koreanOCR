//! Change detection for continuous mode: decide cheaply *whether* the watched
//! region changed before paying for OCR, and whether two OCR reads of it are the
//! same text (OCR jitters by a character or two between frames).

use image::RgbImage;

const COLS: u32 = 48;
const ROWS: u32 = 16;

/// A coarse luminance fingerprint of a frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature(Vec<u8>);

/// Mean luminance on a 48×16 grid. Pixels are sampled on a stride so a full-screen
/// frame costs about as much as a small one.
pub fn signature(img: &RgbImage) -> Signature {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return Signature(vec![0; (COLS * ROWS) as usize]);
    }
    let mut cells = Vec::with_capacity((COLS * ROWS) as usize);
    for row in 0..ROWS {
        let (y0, y1) = (row * h / ROWS, ((row + 1) * h / ROWS).max(row * h / ROWS + 1).min(h));
        for col in 0..COLS {
            let (x0, x1) = (col * w / COLS, ((col + 1) * w / COLS).max(col * w / COLS + 1).min(w));
            let step_x = ((x1 - x0) / 4).max(1) as usize;
            let step_y = ((y1 - y0) / 4).max(1) as usize;
            let (mut sum, mut n) = (0u32, 0u32);
            for y in (y0..y1).step_by(step_y) {
                for x in (x0..x1).step_by(step_x) {
                    let p = img.get_pixel(x, y).0;
                    sum += (u32::from(p[0]) * 299 + u32::from(p[1]) * 587 + u32::from(p[2]) * 114) / 1000;
                    n += 1;
                }
            }
            cells.push((sum / n.max(1)) as u8);
        }
    }
    Signature(cells)
}

/// Fraction of grid cells whose brightness moved by more than `cell_threshold`.
pub fn changed_fraction(a: &Signature, b: &Signature, cell_threshold: u8) -> f32 {
    let changed = a.0.iter().zip(&b.0).filter(|(x, y)| x.abs_diff(**y) > cell_threshold).count();
    changed as f32 / a.0.len().max(1) as f32
}

/// Lower-case letters and digits only, so spacing and punctuation jitter is ignored.
pub fn normalize(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Levenshtein distance between two strings, in characters.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            cur.push((prev[j] + usize::from(ca != cb)).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// 1.0 = identical, 0.0 = nothing in common (Levenshtein over normalised text).
pub fn similarity(a: &str, b: &str) -> f32 {
    let (a, b) = (normalize(a), normalize(b));
    let longest = a.chars().count().max(b.chars().count());
    if longest == 0 {
        return 1.0;
    }
    1.0 - edit_distance(&a, &b) as f32 / longest as f32
}

#[cfg(test)]
mod tests {
    use image::Rgb;

    use super::*;

    fn frame(bg: u8) -> RgbImage {
        RgbImage::from_pixel(400, 100, Rgb([bg, bg, bg]))
    }

    #[test]
    fn identical_frames_do_not_differ() {
        assert_eq!(changed_fraction(&signature(&frame(40)), &signature(&frame(40)), 8), 0.0);
    }

    #[test]
    fn small_text_change_is_noticed_but_noise_is_not() {
        let base = frame(30);
        let mut with_text = base.clone();
        for x in 100..220 {
            for y in 40..60 {
                with_text.put_pixel(x, y, Rgb([240, 240, 240]));
            }
        }
        let f = changed_fraction(&signature(&base), &signature(&with_text), 8);
        assert!(f > 0.05 && f < 0.5, "{f}");

        // +3 brightness everywhere (compression noise) stays below the cell threshold
        assert_eq!(changed_fraction(&signature(&frame(30)), &signature(&frame(33)), 8), 0.0);
    }

    #[test]
    fn works_on_tiny_and_empty_images() {
        let _ = signature(&RgbImage::from_pixel(3, 2, Rgb([1, 2, 3])));
        let _ = signature(&RgbImage::new(0, 0));
    }

    #[test]
    fn edit_distance_basics() {
        assert_eq!(edit_distance("", ""), 0);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("안녕하세요", "안녕하세오"), 1);
        assert_eq!(edit_distance("abc", ""), 3);
    }

    #[test]
    fn ocr_jitter_counts_as_the_same_line() {
        assert_eq!(similarity("안녕하세요 여러분", "안녕하세요여러분!"), 1.0);
        assert!(similarity("저는 학생입니다", "저는 학생입니다.") > 0.99);
        assert!(similarity("저는 학생입니다", "저는 학생입니디") > 0.85);
        assert!(similarity("저는 학생입니다", "오늘은 날씨가 좋네요") < 0.5);
        assert_eq!(similarity("", ""), 1.0);
        assert_eq!(similarity("abc", ""), 0.0);
    }
}
