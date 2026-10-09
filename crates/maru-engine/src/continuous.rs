//! Continuous mode: watch a fixed region (subtitles, dialogue box) and emit a
//! [`Capture`] each time its *text* changes.
//!
//! Cost control is the whole point. A cheap fingerprint of the frame runs every
//! tick; OCR only runs after the picture has changed *and then held still* (so we
//! never read half-faded subtitles); and a re-read that is just OCR jitter is
//! not reported again.

use std::sync::Arc;
use std::time::{Duration, Instant};

use maru_core::config::ContinuousConfig;
use maru_core::{Capture, CaptureSource, Rect};
use maru_ocr::OcrEngine;
use maru_platform::ScreenGrabber;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::change::{changed_fraction, signature, similarity};

/// Cells must brighten/darken by more than this (0..255) to count as changed.
const CELL_THRESHOLD: u8 = 12;
/// Re-reads at least this similar to the last text are the same line.
const SAME_TEXT: f32 = 0.88;

pub struct ContinuousHandle {
    pub rx: mpsc::Receiver<Capture>,
    stop: CancellationToken,
}

impl ContinuousHandle {
    /// Take the receiving end (leaving a closed one behind) so another task can own it
    /// while this handle keeps the stop switch.
    pub fn take_receiver(&mut self) -> mpsc::Receiver<Capture> {
        std::mem::replace(&mut self.rx, mpsc::channel(1).1)
    }

    pub fn stop(&self) {
        self.stop.cancel();
    }
}

impl Drop for ContinuousHandle {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

pub fn spawn(screen: Arc<dyn ScreenGrabber>, ocr: Arc<dyn OcrEngine>, cfg: ContinuousConfig, region: Rect) -> ContinuousHandle {
    let (tx, rx) = mpsc::channel(8);
    let stop = CancellationToken::new();
    tokio::spawn(watch(screen, ocr, cfg, region, tx, stop.clone()));
    ContinuousHandle { rx, stop }
}

async fn watch(
    screen: Arc<dyn ScreenGrabber>,
    ocr: Arc<dyn OcrEngine>,
    cfg: ContinuousConfig,
    region: Rect,
    tx: mpsc::Sender<Capture>,
    stop: CancellationToken,
) {
    let interval = Duration::from_millis(cfg.interval_ms.max(10));
    let stable = Duration::from_millis(cfg.stable_ms);
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut last_sig = None;
    let mut last_change = Instant::now();
    let mut dirty = true; // read the first frame
    let mut last_text = String::new();

    loop {
        tokio::select! {
            _ = stop.cancelled() => return,
            _ = ticker.tick() => {}
        }

        let grab = {
            let screen = screen.clone();
            tokio::task::spawn_blocking(move || screen.grab(region)).await
        };
        let Ok(Ok(frame)) = grab else { continue };
        let sig = signature(&frame);
        if let Some(prev) = &last_sig
            && changed_fraction(prev, &sig, CELL_THRESHOLD) > cfg.diff_threshold
        {
            last_change = Instant::now();
            dirty = true;
        }
        last_sig = Some(sig);

        if !dirty || last_change.elapsed() < stable {
            continue;
        }
        dirty = false;

        let ocr = ocr.clone();
        let Ok(Ok(out)) = tokio::task::spawn_blocking(move || ocr.recognize(&frame)).await else { continue };
        let out = out.to_screen(maru_core::Point { x: region.x, y: region.y }, 1.0);
        let text = out.lines.iter().map(|l| l.text.trim()).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            last_text.clear(); // the line disappeared; the same line may come back later
            continue;
        }
        if !last_text.is_empty() && similarity(&last_text, &text) >= SAME_TEXT {
            continue;
        }
        last_text = text.clone();
        let capture = Capture {
            text,
            source: CaptureSource::Continuous,
            rect: out.bounds(),
            words: out.words(),
            cursor: None,
            app: screen.foreground_app(),
            confidence: Some(out.confidence()),
        };
        if tx.send(capture).await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use image::{Rgb, RgbImage};
    use maru_ocr::OcrOutput;
    use maru_platform::testing::FakeScreen;

    use super::*;
    use crate::capture::testing::{FakeOcr, line};

    fn frame(page: u8, shade: u8) -> RgbImage {
        // red channel of pixel (0,0) selects the OCR page; the rest sets overall brightness
        let mut img = RgbImage::from_pixel(400, 80, Rgb([shade, shade, shade]));
        img.put_pixel(0, 0, Rgb([page, shade, shade]));
        img
    }

    fn cfg() -> ContinuousConfig {
        ContinuousConfig { interval_ms: 15, stable_ms: 45, diff_threshold: 0.01, ..Default::default() }
    }

    fn pages() -> Vec<(u8, OcrOutput)> {
        vec![
            (10, OcrOutput { lines: vec![line("안녕하세요 여러분", 5.0, 5.0, 200.0)] }),
            (11, OcrOutput { lines: vec![line("안녕하세요 여러분.", 5.0, 5.0, 205.0)] }), // jitter only
            (20, OcrOutput { lines: vec![line("오늘도 만나서 반가워요", 5.0, 5.0, 250.0)] }),
        ]
    }

    async fn next(h: &mut ContinuousHandle, ms: u64) -> Option<Capture> {
        tokio::time::timeout(Duration::from_millis(ms), h.rx.recv()).await.ok().flatten()
    }

    #[tokio::test]
    async fn emits_each_new_line_once_and_ignores_ocr_jitter() {
        // blank → line A (held, then jittered re-read) → line B (held forever)
        let mut frames = vec![frame(0, 20); 3];
        frames.extend(vec![frame(10, 120); 12]);
        frames.extend(vec![frame(11, 121); 6]);
        frames.extend(vec![frame(20, 200); 200]);
        let screen = Arc::new(FakeScreen::new(frames));
        let ocr = Arc::new(FakeOcr::new(pages()));
        let mut h = spawn(screen, ocr, cfg(), Rect::new(100.0, 800.0, 400.0, 80.0));

        let a = next(&mut h, 2000).await.expect("first line");
        assert_eq!(a.text, "안녕하세요 여러분");
        assert_eq!(a.source, CaptureSource::Continuous);
        assert!(a.rect.unwrap().x >= 100.0, "boxes are in screen coordinates");

        let b = next(&mut h, 2000).await.expect("second line");
        assert_eq!(b.text, "오늘도 만나서 반가워요", "the jittered re-read must not be reported");

        assert!(next(&mut h, 400).await.is_none(), "a static screen produces nothing more");
    }

    #[tokio::test]
    async fn does_not_run_ocr_while_the_picture_is_still_changing() {
        // brightness keeps changing every frame, so the stable window never elapses
        let frames: Vec<RgbImage> = (0..400).map(|i| frame(10, if i % 2 == 0 { 40 } else { 200 })).collect();
        let ocr = Arc::new(FakeOcr::new(pages()));
        let mut h = spawn(Arc::new(FakeScreen::new(frames)), ocr.clone(), cfg(), Rect::new(0.0, 0.0, 400.0, 80.0));
        assert!(next(&mut h, 350).await.is_none());
        assert_eq!(*ocr.calls.lock().unwrap(), 0, "OCR must wait for the frame to settle");
    }

    #[tokio::test]
    async fn the_receiver_can_be_moved_out_while_the_handle_keeps_control() {
        let ocr = Arc::new(FakeOcr::new(pages()));
        let mut h = spawn(Arc::new(FakeScreen::new(vec![frame(10, 100)])), ocr, cfg(), Rect::new(0.0, 0.0, 400.0, 80.0));
        let mut rx = h.take_receiver();
        assert!(tokio::time::timeout(Duration::from_secs(1), rx.recv()).await.unwrap().is_some());
        h.stop();
        assert!(tokio::time::timeout(Duration::from_secs(1), rx.recv()).await.expect("closes after stop").is_none());
    }

    #[tokio::test]
    async fn stopping_ends_the_stream() {
        let ocr = Arc::new(FakeOcr::new(pages()));
        let mut h = spawn(Arc::new(FakeScreen::new(vec![frame(10, 100)])), ocr, cfg(), Rect::new(0.0, 0.0, 400.0, 80.0));
        assert!(next(&mut h, 1000).await.is_some());
        h.stop();
        assert!(tokio::time::timeout(Duration::from_secs(1), h.rx.recv()).await.expect("channel closes").is_none());
    }
}
