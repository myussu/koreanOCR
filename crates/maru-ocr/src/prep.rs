//! Image preparation for screen text.
//!
//! Screen captures differ from scanned documents: text is crisp but often tiny,
//! the selection rectangle is tight (detectors need a margin), and backgrounds
//! are flat. A little preparation noticeably helps both speed and accuracy.

use image::{Rgb, RgbImage, imageops};

/// Smallest text height (in image pixels) we try to reach by upscaling. PP-OCR
/// recognisers were trained on lines about 32–48 px tall; 12 px UI text is much
/// better read after a 2–3× enlargement.
const TARGET_MIN_HEIGHT: u32 = 64;
const MAX_UPSCALE: f32 = 3.0;
const PAD: u32 = 12;
/// Never feed more than this many pixels (long side) to the detector.
const MAX_SIDE: u32 = 2000;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Prepared {
    /// Scale applied to the original (>1 = enlarged).
    pub scale: f32,
    /// Margin added on every side, in *prepared* pixels.
    pub pad: u32,
}

impl Prepared {
    /// Map a coordinate in the prepared image back to the original.
    pub fn to_original(&self, v: f32) -> f32 {
        (v - self.pad as f32) / self.scale
    }
}

fn background(img: &RgbImage) -> Rgb<u8> {
    // The most common colour along the border is the background for flat UI text.
    let (w, h) = img.dimensions();
    let mut counts: std::collections::HashMap<[u8; 3], u32> = std::collections::HashMap::new();
    let mut add = |x: u32, y: u32| *counts.entry(img.get_pixel(x, y).0).or_default() += 1;
    for x in 0..w {
        add(x, 0);
        add(x, h - 1);
    }
    for y in 0..h {
        add(0, y);
        add(w - 1, y);
    }
    counts.into_iter().max_by_key(|&(_, n)| n).map(|(c, _)| Rgb(c)).unwrap_or(Rgb([255, 255, 255]))
}

pub fn prepare(img: &RgbImage) -> (RgbImage, Prepared) {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return (img.clone(), Prepared { scale: 1.0, pad: 0 });
    }

    // Single-line crops are the common case for selections: enlarge by height.
    let mut scale = if h < TARGET_MIN_HEIGHT { (TARGET_MIN_HEIGHT as f32 / h as f32).min(MAX_UPSCALE) } else { 1.0 };
    let long = w.max(h) as f32 * scale;
    if long > MAX_SIDE as f32 {
        scale *= MAX_SIDE as f32 / long;
    }

    let scaled = if (scale - 1.0).abs() > 0.01 {
        let nw = ((w as f32 * scale).round() as u32).max(1);
        let nh = ((h as f32 * scale).round() as u32).max(1);
        let filter = if scale > 1.0 { imageops::FilterType::CatmullRom } else { imageops::FilterType::Triangle };
        imageops::resize(img, nw, nh, filter)
    } else {
        scale = 1.0;
        img.clone()
    };

    let bg = background(&scaled);
    let mut out = RgbImage::from_pixel(scaled.width() + 2 * PAD, scaled.height() + 2 * PAD, bg);
    imageops::replace(&mut out, &scaled, PAD as i64, PAD as i64);
    (out, Prepared { scale, pad: PAD })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_crops_are_enlarged_and_padded() {
        let img = RgbImage::from_pixel(120, 20, Rgb([30, 30, 30]));
        let (out, p) = prepare(&img);
        assert!((p.scale - 3.0).abs() < 0.01, "20px → capped at 3x, got {}", p.scale);
        assert_eq!(out.dimensions(), (120 * 3 + 24, 60 + 24));
        assert_eq!(out.get_pixel(0, 0), &Rgb([30, 30, 30]), "padding takes the background colour");
    }

    #[test]
    fn tall_images_are_not_enlarged() {
        let (out, p) = prepare(&RgbImage::from_pixel(300, 200, Rgb([255, 255, 255])));
        assert_eq!(p.scale, 1.0);
        assert_eq!(out.dimensions(), (324, 224));
    }

    #[test]
    fn huge_images_are_capped() {
        let (out, p) = prepare(&RgbImage::from_pixel(4000, 100, Rgb([255, 255, 255])));
        assert!(p.scale < 0.6);
        assert!(out.width() <= MAX_SIDE + 2 * PAD);
    }

    #[test]
    fn coordinates_map_back() {
        let p = Prepared { scale: 2.0, pad: 12 };
        assert_eq!(p.to_original(112.0), 50.0);
    }

    #[test]
    fn background_is_the_dominant_border_colour() {
        let mut img = RgbImage::from_pixel(50, 50, Rgb([10, 20, 30]));
        for x in 20..30 {
            img.put_pixel(x, 25, Rgb([255, 255, 255]));
        }
        assert_eq!(background(&img), Rgb([10, 20, 30]));
    }

    #[test]
    fn degenerate_images_do_not_panic() {
        let (_, p) = prepare(&RgbImage::new(0, 0));
        assert_eq!(p.scale, 1.0);
    }
}
