//! Confirm repeated live OCR against the lettering in its original frame.

use image::RgbImage;
use maru_core::Rect;
use std::sync::Arc;

use crate::change::{normalize, similarity};

pub(crate) struct Lettering {
    width: usize,
    height: usize,
    bounds: Rect,
    mask: Arc<Vec<bool>>,
}

impl Lettering {
    pub(crate) fn new(frame: &RgbImage, bounds: Option<Rect>, mask: Arc<Vec<bool>>) -> Option<Self> {
        let bounds = bounds.filter(|r| !r.is_empty())?;
        let this = Self { width: frame.width() as usize, height: frame.height() as usize, bounds, mask };
        (this.marked(bounds) >= 150).then_some(this)
    }

    pub(crate) fn remains(&self, frame: &RgbImage, mask: &[bool]) -> bool {
        if frame.dimensions() != (self.width as u32, self.height as u32) {
            return false;
        }
        self.matches(mask, self.bounds, false)
    }

    pub(crate) fn same_read(&self, before: &str, after: &str, next: &Self) -> bool {
        if (self.width, self.height) != (next.width, next.height) || !near(self.bounds, next.bounds) || similarity(before, after) < 0.65 {
            return false;
        }
        // A changed Korean character or number can change the meaning. Only identical lettering
        // may dismiss that change; isolated Latin OCR noise permits a little background movement.
        let noise_only = without_latin_letters(before) == without_latin_letters(after);
        self.matches(&next.mask, self.bounds.union(&next.bounds), noise_only)
    }

    fn marked(&self, bounds: Rect) -> usize {
        let (x0, y0, x1, y1) = self.area(bounds);
        (y0..y1).map(|y| self.mask[y * self.width + x0..y * self.width + x1].iter().filter(|p| **p).count()).sum()
    }

    fn area(&self, bounds: Rect) -> (usize, usize, usize, usize) {
        let x0 = (bounds.x - 4.0).max(0.0).min(self.width as f32) as usize;
        let y0 = (bounds.y - 4.0).max(0.0).min(self.height as f32) as usize;
        let x1 = (bounds.right() + 4.0).ceil().max(x0 as f32).min(self.width as f32) as usize;
        let y1 = (bounds.bottom() + 4.0).ceil().max(y0 as f32).min(self.height as f32) as usize;
        (x0, y0, x1, y1)
    }

    fn matches(&self, next: &[bool], bounds: Rect, noise_only: bool) -> bool {
        let (x0, y0, x1, y1) = self.area(bounds);
        let mut changed = 0;
        let mut marked = 0;
        for top in (y0..y1).step_by(16) {
            for left in (x0..x1).step_by(16) {
                let mut tile_changed = 0;
                for y in top..(top + 16).min(y1) {
                    for x in left..(left + 16).min(x1) {
                        let i = y * self.width + x;
                        marked += usize::from(self.mask[i] || next[i]);
                        tile_changed += usize::from(self.mask[i] != next[i]);
                    }
                }
                if tile_changed > if noise_only { 2 } else { 0 } {
                    return false;
                }
                changed += tile_changed;
            }
        }
        marked >= 150 && changed * 100 <= marked
    }
}

fn near(a: Rect, b: Rect) -> bool {
    let height = a.h.max(b.h);
    (a.x - b.x).abs() <= height / 2.0 && (a.y - b.y).abs() <= height / 2.0
        && (a.w - b.w).abs() <= a.w.max(b.w) * 0.25 && (a.h - b.h).abs() <= height / 2.0
}

fn without_latin_letters(text: &str) -> String {
    normalize(text).chars().filter(|c| !c.is_ascii_alphabetic()).collect()
}

#[cfg(test)]
mod tests {
    use image::Rgb;
    use crate::change::text_mask;

    use super::*;

    fn frame(shift: u32) -> RgbImage {
        let mut frame = RgbImage::from_fn(240, 60, |x, y| { let v = 100 + ((x + y + shift) % 70) as u8; Rgb([v, v, v]) });
        for k in 0..10 {
            for y in 20..38 {
                for x in 20 + k * 20..32 + k * 20 {
                    let edge = x < 22 + k * 20 || x >= 30 + k * 20 || !(22..36).contains(&y);
                    frame.put_pixel(x, y, if edge { Rgb([10, 10, 10]) } else { Rgb([250, 250, 250]) });
                }
            }
        }
        frame
    }

    fn stamp(frame: &RgbImage) -> Lettering {
        Lettering::new(frame, Some(Rect::new(15.0, 15.0, 210.0, 30.0)), Arc::new(text_mask(frame))).unwrap()
    }

    #[test]
    fn latin_noise_on_unchanged_lettering_is_the_same_subtitle() {
        assert!(stamp(&frame(0)).same_read("그건 우리뿐만이 아니에요", "그건 우리뿐만이 아니에요 SP", &stamp(&frame(30))));
    }

    #[test]
    fn a_changed_ocr_character_on_identical_lettering_is_not_another_line() {
        assert!(stamp(&frame(0)).same_read("폐하 왜 그러시죠", "메하 왜 그러시죠", &stamp(&frame(30))));
    }

    #[test]
    fn changed_numbers_and_negations_are_kept_when_even_one_lettering_pixel_changes() {
        let before = frame(0);
        let mut after = frame(30);
        after.put_pixel(25, 25, Rgb([10, 10, 10]));
        for (a, b) in [("제47구가 봉쇄됐습니다", "제48구가 봉쇄됐습니다"), ("그건 우리뿐이에요", "그건 우리뿐만이 아니에요")] {
            assert!(!stamp(&before).same_read(a, b, &stamp(&after)));
        }
    }

    #[test]
    fn real_latin_letters_added_to_the_image_are_kept() {
        let before = frame(0);
        let mut after = frame(30);
        for y in 23..35 {
            for x in 213..223 {
                let edge = !(25..33).contains(&y) || !(215..221).contains(&x);
                after.put_pixel(x, y, if edge { Rgb([10, 10, 10]) } else { Rgb([250, 250, 250]) });
            }
        }
        assert!(!stamp(&before).same_read("그건 우리뿐만이 아니에요", "그건 우리뿐만이 아니에요 SP", &stamp(&after)));
    }

    #[test]
    fn a_missed_read_is_ignored_only_while_the_original_lettering_remains() {
        let before = frame(0);
        assert!(stamp(&before).remains(&frame(30), &text_mask(&frame(30))));
        let blank = RgbImage::from_pixel(240, 60, Rgb([120, 120, 120]));
        assert!(!stamp(&before).remains(&blank, &text_mask(&blank)));
    }

    #[test]
    fn text_without_a_clear_contrast_signal_has_no_visual_confirmation() {
        let blank = RgbImage::from_pixel(240, 60, Rgb([120, 120, 120]));
        assert!(Lettering::new(&blank, Some(Rect::new(15.0, 15.0, 210.0, 30.0)), Arc::new(text_mask(&blank))).is_none());
    }
}
