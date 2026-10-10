//! End-to-end check against real ONNX Runtime + PP-OCR models.
//!
//! Skipped unless `MARU_TEST_OCR_DIR` points at a folder with `det.onnx`,
//! `rec.onnx` and `dict.txt` and `ORT_DYLIB_PATH` points at the ONNX Runtime
//! shared library. (CI/dev machines without models still run everything else.)
#![cfg(feature = "paddle")]

use ab_glyph::{FontRef, PxScale};
use image::{Rgb, RgbImage};
use imageproc::drawing::draw_text_mut;
use maru_core::Point;
use maru_ocr::OcrEngine;
use maru_ocr::paddle::{PaddleConfig, PaddleOcr};

fn engine() -> Option<PaddleOcr> {
    let dir = std::path::PathBuf::from(std::env::var_os("MARU_TEST_OCR_DIR")?);
    let cfg = PaddleConfig {
        det_model: dir.join("det.onnx"),
        rec_model: dir.join("rec.onnx"),
        dict_path: dir.join("dict.txt"),
        device: "cpu".into(),
        detect_side: None,
    };
    Some(PaddleOcr::load(&cfg).expect("models load"))
}

fn font_bytes() -> Option<Vec<u8>> {
    ["/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", "C:\\Windows\\Fonts\\arial.ttf", "/System/Library/Fonts/Helvetica.ttc"]
        .iter()
        .find_map(|p| std::fs::read(p).ok())
}

#[allow(clippy::too_many_arguments)]
fn render(text: &str, w: u32, h: u32, x: i32, y: i32, size: f32, fg: [u8; 3], bg: [u8; 3]) -> RgbImage {
    let bytes = font_bytes().expect("a system font");
    let font = FontRef::try_from_slice(&bytes).unwrap();
    let mut img = RgbImage::from_pixel(w, h, Rgb(bg));
    draw_text_mut(&mut img, Rgb(fg), x, y, PxScale::from(size), &font, text);
    img
}

#[test]
fn reads_text_and_maps_word_boxes_back_to_pixels() {
    let (Some(ocr), Some(_)) = (engine(), font_bytes()) else {
        eprintln!("skipped: set MARU_TEST_OCR_DIR and ORT_DYLIB_PATH to run");
        return;
    };
    let img = render("Hello world from Maru", 760, 120, 30, 36, 40.0, [20, 20, 20], [255, 255, 255]);
    let out = ocr.recognize(&img).unwrap();
    eprintln!("text={:?} conf={:.2} lines={}", out.text(), out.confidence(), out.lines.len());
    let text = out.text().to_lowercase();
    assert!(text.contains("hello") && text.contains("world") && text.contains("maru"), "got {text:?}");
    assert!(out.confidence() > 0.6, "confidence {}", out.confidence());

    let words = out.words();
    eprintln!("words: {:?}", words.iter().map(|w| (&w.text, w.rect.x.round(), w.rect.w.round())).collect::<Vec<_>>());
    assert!(words.len() >= 3, "{words:?}");
    // inside the image, left-to-right, and the first word starts near where we drew it (x=30)
    assert!(words.iter().all(|w| w.rect.x >= -2.0 && w.rect.right() <= 762.0 && w.rect.y >= -2.0 && w.rect.bottom() <= 122.0));
    assert!(words.windows(2).all(|p| p[0].rect.x <= p[1].rect.x));
    assert!((words[0].rect.x - 30.0).abs() < 25.0, "first word x = {}", words[0].rect.x);
    assert!(words.last().unwrap().rect.right() > 400.0, "last word should reach the right part of the text");
}

#[test]
fn tiny_ui_text_on_dark_background_is_enlarged_and_still_read() {
    let (Some(ocr), Some(_)) = (engine(), font_bytes()) else {
        eprintln!("skipped: set MARU_TEST_OCR_DIR and ORT_DYLIB_PATH to run");
        return;
    };
    // 15 px text, light on dark, in a tight 230×28 crop (what a selection rectangle gives us)
    let img = render("Settings and options", 230, 28, 4, 4, 16.0, [235, 235, 235], [32, 33, 36]);
    let out = ocr.recognize(&img).unwrap();
    eprintln!("tiny: {:?} conf={:.2}", out.text(), out.confidence());
    let text = out.text().to_lowercase();
    assert!(text.contains("settings") && text.contains("options"), "got {text:?}");

    // Coordinates must come back in *original* pixels despite internal enlargement + padding.
    let b = out.bounds().expect("some text");
    assert!(b.x >= -3.0 && b.right() <= 233.0 && b.bottom() <= 31.0, "bounds {b:?}");
    let screen = out.to_screen(Point { x: 1000.0, y: 400.0 }, 1.0);
    assert!(screen.bounds().unwrap().x >= 997.0);
}

#[test]
fn blank_image_yields_nothing() {
    let Some(ocr) = engine() else {
        eprintln!("skipped: set MARU_TEST_OCR_DIR and ORT_DYLIB_PATH to run");
        return;
    };
    let out = ocr.recognize(&RgbImage::from_pixel(200, 80, Rgb([255, 255, 255]))).unwrap();
    assert!(out.is_empty(), "{out:?}");
}
