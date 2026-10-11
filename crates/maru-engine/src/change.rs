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

/// Luminance at or above this is "bright" for [`text_score`], at or below the second is "dark".
const BRIGHT: u32 = 224;
const DARK: u32 = 80;
/// How far (in pixels) a dark pixel may be from a bright one and still count as its outline.
const REACH: usize = 2;

/// How much text-like contrast a frame holds: pixels that are very bright right next to very dark ones, and the
/// other way round (white lettering with a dark outline, black lettering on a light box).
///
/// A backdrop, however busy and however fast it moves, scores low; lettering scores in the hundreds or thousands
/// and does so the instant it appears, so a jump in this number says "text just appeared or went" long before
/// anything is read. It is the same cheap kind of signal as [`signature`] (a few milliseconds for a subtitle
/// strip), and it is only ever used to decide *when to look*, never what the text says.
pub fn text_score(img: &RgbImage) -> u32 {
    let (w, h) = (img.width() as usize, img.height() as usize);
    if w < 2 * REACH + 1 || h < 2 * REACH + 1 {
        return 0;
    }
    // 1 = bright, 2 = dark, 0 = neither
    let mut class = vec![0u8; w * h];
    for (c, p) in class.iter_mut().zip(img.pixels()) {
        let l = (u32::from(p[0]) * 54 + u32::from(p[1]) * 183 + u32::from(p[2]) * 19) >> 8;
        *c = if l >= BRIGHT {
            1
        } else if l <= DARK {
            2
        } else {
            0
        };
    }
    let near_bright = dilate(&class, w, h, 1);
    let near_dark = dilate(&class, w, h, 2);
    let mut score = 0u32;
    for i in 0..w * h {
        if (class[i] == 1 && near_dark[i]) || (class[i] == 2 && near_bright[i]) {
            score += 1;
        }
    }
    score
}

/// For every pixel: is there a pixel of `wanted` class within [`REACH`] of it (a square neighbourhood)?
fn dilate(class: &[u8], w: usize, h: usize, wanted: u8) -> Vec<bool> {
    // along each row first, then down each column
    let mut across = vec![false; w * h];
    for y in 0..h {
        let row = &class[y * w..(y + 1) * w];
        for x in 0..w {
            let (a, b) = (x.saturating_sub(REACH), (x + REACH + 1).min(w));
            across[y * w + x] = row[a..b].contains(&wanted);
        }
    }
    let mut out = vec![false; w * h];
    for x in 0..w {
        for y in 0..h {
            let (a, b) = (y.saturating_sub(REACH), (y + REACH + 1).min(h));
            out[y * w + x] = (a..b).any(|yy| across[yy * w + x]);
        }
    }
    out
}

/// The smallest change in [`text_score`] (in pixels, for a frame of `area` pixels) that counts as text appearing or going.
pub fn text_floor(area: usize) -> u32 {
    (area / 2500).max(40) as u32
}

/// How much text-like contrast a frame must hold before it is believed to contain lettering at all.
pub fn text_presence(area: usize) -> u32 {
    (area / 400).max(150) as u32
}

/// Did the text signal jump between two readings? A fraction of the larger one, and at least `text_floor`,
/// so a steady subtitle's few percent of wobble over a moving backdrop is not a jump.
pub fn text_jumped(before: u32, after: u32, area: usize) -> bool {
    let (b, a) = (i64::from(before), i64::from(after));
    (a - b).unsigned_abs() > u64::from(text_floor(area)).max(((a.max(b) as f64) * 0.15) as u64)
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

    /// A strip of lettering: bright blocks with a dark outline, over a backdrop of mid tones that moves with `shift`.
    fn strip(w: u32, h: u32, shift: u32, lettering: bool) -> RgbImage {
        let mut img = RgbImage::from_fn(w, h, |x, y| {
            let v = 100 + ((x + y * 3 + shift * 7) % 70) as u8; // mid tones only
            Rgb([v, v, v])
        });
        if lettering {
            for k in 0..14u32 {
                let (x0, y0) = (20 + k * 18, 30);
                for y in y0..y0 + 18 {
                    for x in x0..x0 + 12 {
                        let edge = x < x0 + 2 || x >= x0 + 10 || y < y0 + 2 || y >= y0 + 16;
                        img.put_pixel(x, y, if edge { Rgb([10, 10, 10]) } else { Rgb([250, 250, 250]) });
                    }
                }
            }
        }
        img
    }

    #[test]
    fn lettering_scores_high_and_a_moving_backdrop_scores_nothing() {
        let area = 300 * 80;
        let empty: Vec<u32> = (0..8).map(|i| text_score(&strip(300, 80, i, false))).collect();
        assert!(empty.iter().all(|s| *s == 0), "mid tones have no extreme contrast: {empty:?}");
        let text = text_score(&strip(300, 80, 3, true));
        assert!(text > 5 * text_presence(area), "lettering is far above the presence line: {text}");
        let again = text_score(&strip(300, 80, 5, true));
        assert!(!text_jumped(text, again, area), "the same lettering over a different backdrop is steady: {text} vs {again}");
    }

    #[test]
    fn dark_lettering_on_a_light_box_counts_too() {
        let mut img = RgbImage::from_pixel(300, 80, Rgb([240, 240, 240]));
        for k in 0..14u32 {
            for y in 30..48 {
                for x in 20 + k * 18..32 + k * 18 {
                    img.put_pixel(x, y, Rgb([15, 15, 15]));
                }
            }
        }
        assert!(text_score(&img) > text_presence(300 * 80), "{}", text_score(&img));
    }

    #[test]
    fn a_jump_needs_a_real_change_not_wobble() {
        let area = 960 * 130;
        assert!(text_jumped(0, 1100, area), "text appears");
        assert!(text_jumped(2000, 0, area), "text goes");
        assert!(text_jumped(2000, 2600, area), "one line replaced by a longer one");
        assert!(!text_jumped(2000, 2100, area), "five percent is wobble");
        assert!(!text_jumped(0, 30, area), "a few specks are not text");
        assert!(!text_jumped(20, 0, area));
    }

    #[test]
    fn tiny_images_do_not_panic() {
        assert_eq!(text_score(&RgbImage::new(0, 0)), 0);
        assert_eq!(text_score(&RgbImage::new(3, 3)), 0);
        assert_eq!(text_score(&RgbImage::from_pixel(5, 5, Rgb([255, 255, 255]))), 0, "bright with nothing dark near it is not text");
    }

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
