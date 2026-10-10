//! Live subtitles: watch a fixed region (subtitles, dialogue box) and report each line of text as
//! it appears, and when it is gone.
//!
//! Speed matters here, so nothing waits for the picture to stop moving:
//! * a cheap fingerprint of the frame runs every tick, and OCR only runs when the picture differs
//!   from the one last read;
//! * a line is reported as soon as **two reads in a row agree** (a line that fades in, or that sits
//!   over a moving scene, never holds still), or at once when the picture has held still for
//!   `stable_ms` (a finished line needs only one read);
//! * a scene that keeps moving with no text in it is read at a slower rate, so a cutscene does not
//!   cost a core;
//! * a re-read that is just OCR jitter is not reported again.

use std::sync::Arc;
use std::time::{Duration, Instant};

use maru_core::config::ContinuousConfig;
use maru_core::{Capture, CaptureSource, Rect};
use maru_ocr::{OcrEngine, OcrOutput};
use maru_platform::ScreenGrabber;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::change::{Signature, changed_fraction, signature, similarity};

/// Cells must brighten/darken by more than this (0..255) to count as changed.
const CELL_THRESHOLD: u8 = 12;
/// Re-reads at least this similar to the line on screen are the same line.
const SAME_TEXT: f32 = 0.88;
/// Two reads at least this similar agree.
const AGREE: f32 = 0.9;
/// A picture that began changing this recently may be a line appearing: read it on every tick.
const HUNT_FOR: Duration = Duration::from_millis(600);

/// A new line, with how long each step took (for the latency readout).
#[derive(Debug, Clone)]
pub struct LiveLine {
    pub capture: Capture,
    /// When the picture first changed towards this line.
    pub appeared: Instant,
    /// From `appeared` until the line was read, in milliseconds.
    pub ready_ms: u32,
    /// What the last OCR took, in milliseconds.
    pub ocr_ms: u32,
}

#[derive(Debug, Clone)]
pub enum LiveEvent {
    /// A new line is on screen.
    Line(Box<LiveLine>),
    /// The line is gone.
    Cleared,
}

/// A read that is waiting for confirmation.
struct Pending {
    text: String,
    out: OcrOutput,
    ocr_ms: u32,
}

fn line_of(p: Pending, appeared: Instant, screen: &dyn ScreenGrabber) -> LiveLine {
    LiveLine {
        ready_ms: appeared.elapsed().as_millis().min(u128::from(u32::MAX)) as u32,
        ocr_ms: p.ocr_ms,
        appeared,
        capture: Capture {
            text: p.text,
            source: CaptureSource::Continuous,
            rect: p.out.bounds(),
            words: p.out.words(),
            cursor: None,
            app: screen.foreground_app(),
            confidence: Some(p.out.confidence()),
        },
    }
}

pub struct ContinuousHandle {
    pub rx: mpsc::Receiver<LiveEvent>,
    stop: CancellationToken,
}

impl ContinuousHandle {
    /// Take the receiving end (leaving a closed one behind) so another task can own it
    /// while this handle keeps the stop switch.
    pub fn take_receiver(&mut self) -> mpsc::Receiver<LiveEvent> {
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

/// Start watching `region` on `runtime`.
///
/// The runtime is passed in rather than found with `tokio::spawn` because the overlay calls this
/// from a window-event callback, which is not a Tokio thread: `tokio::spawn` there panics, and a
/// panic inside a native callback aborts the whole process.
pub fn spawn(runtime: &tokio::runtime::Handle, screen: Arc<dyn ScreenGrabber>, ocr: Arc<dyn OcrEngine>, cfg: ContinuousConfig, region: Rect) -> ContinuousHandle {
    let (tx, rx) = mpsc::channel(8);
    let stop = CancellationToken::new();
    runtime.spawn(watch(screen, ocr, cfg, region, tx, stop.clone()));
    ContinuousHandle { rx, stop }
}

async fn watch(screen: Arc<dyn ScreenGrabber>, ocr: Arc<dyn OcrEngine>, cfg: ContinuousConfig, region: Rect, tx: mpsc::Sender<LiveEvent>, stop: CancellationToken) {
    let interval = Duration::from_millis(cfg.interval_ms.max(10));
    let stable = Duration::from_millis(cfg.stable_ms);
    // A scene that keeps moving without any text in it is read at this slower rate.
    let idle_gap = interval * 3;
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut prev_sig: Option<Signature> = None;
    let mut read_sig: Option<Signature> = None; // the picture that was read last
    let mut read_at: Option<Instant> = None;
    let mut still_since = Instant::now(); // the last time the picture changed
    let mut burst: Option<Instant> = None; // when the picture began changing towards the line in progress
    let mut pending: Option<Pending> = None; // read once, waiting for a second read to agree
    let mut shown = String::new(); // the line on screen now
    let mut empty_reads = 0u8;

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
        let now = Instant::now();
        let sig = signature(&frame);
        let changed = prev_sig.as_ref().is_none_or(|p| changed_fraction(p, &sig, CELL_THRESHOLD) > cfg.diff_threshold);
        prev_sig = Some(sig.clone());
        if changed {
            still_since = now;
            burst.get_or_insert(now);
        }
        let still = now.duration_since(still_since) >= stable;

        // Nothing to read unless the picture differs from the one read last.
        let unread = read_sig.as_ref().is_none_or(|r| changed_fraction(r, &sig, CELL_THRESHOLD) > cfg.diff_threshold);
        if !unread {
            // The picture has not moved since it was read: a line waiting for a second read is
            // confirmed by the picture holding still.
            if let Some(p) = pending.take() {
                let appeared = burst.unwrap_or(now);
                shown = p.text.clone();
                if tx.send(LiveEvent::Line(Box::new(line_of(p, appeared, screen.as_ref())))).await.is_err() {
                    return;
                }
            }
            // ...and so is a picture that held still after a read that found nothing: the line is gone.
            if empty_reads > 0 && !shown.is_empty() {
                shown.clear();
                if tx.send(LiveEvent::Cleared).await.is_err() {
                    return;
                }
            }
            if still {
                burst = None;
            }
            continue;
        }
        let hunting = pending.is_some() || burst.is_some_and(|b| now.duration_since(b) < HUNT_FOR);
        if !still && !hunting && read_at.is_some_and(|t| now.duration_since(t) < idle_gap) {
            continue;
        }

        let ocr_started = Instant::now();
        let ocr = ocr.clone();
        let Ok(Ok(out)) = tokio::task::spawn_blocking(move || ocr.recognize(&frame)).await else { continue };
        let ocr_ms = ocr_started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
        read_sig = Some(sig);
        read_at = Some(Instant::now());

        let out = out.to_screen(maru_core::Point { x: region.x, y: region.y }, 1.0);
        let text = out.lines.iter().map(|l| l.text.trim()).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" ");

        if text.is_empty() {
            pending = None;
            empty_reads = empty_reads.saturating_add(1);
            // Gone once the picture has settled on nothing, or twice in a row (one missed read
            // over a busy background must not take the line down and bring it back).
            if !shown.is_empty() && (still || empty_reads >= 2) {
                shown.clear();
                burst = None;
                if tx.send(LiveEvent::Cleared).await.is_err() {
                    return;
                }
            }
            if still {
                burst = None;
            }
            continue;
        }
        empty_reads = 0;

        if !shown.is_empty() && similarity(&shown, &text) >= SAME_TEXT {
            pending = None; // the line already on screen, read again
            if still {
                burst = None;
            }
            continue;
        }
        let agrees = pending.as_ref().is_some_and(|c| similarity(&c.text, &text) >= AGREE);
        let read = Pending { text, out, ocr_ms };
        if !(still || agrees) {
            pending = Some(read); // wait for the next read (or the picture holding still) to confirm it
            continue;
        }

        let appeared = burst.take().unwrap_or(now);
        pending = None;
        shown = read.text.clone();
        if tx.send(LiveEvent::Line(Box::new(line_of(read, appeared, screen.as_ref())))).await.is_err() {
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

    async fn next(h: &mut ContinuousHandle, ms: u64) -> Option<LiveEvent> {
        tokio::time::timeout(Duration::from_millis(ms), h.rx.recv()).await.ok().flatten()
    }

    async fn next_line(h: &mut ContinuousHandle, ms: u64) -> Option<LiveLine> {
        match next(h, ms).await {
            Some(LiveEvent::Line(l)) => Some(*l),
            _ => None,
        }
    }

    fn watch_frames(frames: Vec<RgbImage>) -> (ContinuousHandle, Arc<FakeOcr>) {
        let ocr = Arc::new(FakeOcr::new(pages()));
        let h = spawn(&tokio::runtime::Handle::current(), Arc::new(FakeScreen::new(frames)), ocr.clone(), cfg(), Rect::new(100.0, 800.0, 400.0, 80.0));
        (h, ocr)
    }

    #[tokio::test]
    async fn emits_each_new_line_once_and_ignores_ocr_jitter() {
        // blank → line A (held, then jittered re-read) → line B (held forever)
        let mut frames = vec![frame(0, 20); 3];
        frames.extend(vec![frame(10, 120); 12]);
        frames.extend(vec![frame(11, 121); 6]);
        frames.extend(vec![frame(20, 200); 200]);
        let (mut h, _) = watch_frames(frames);

        let a = next_line(&mut h, 2000).await.expect("first line");
        assert_eq!(a.capture.text, "안녕하세요 여러분");
        assert_eq!(a.capture.source, CaptureSource::Continuous);
        assert!(a.capture.rect.unwrap().x >= 100.0, "boxes are in screen coordinates");

        let b = next_line(&mut h, 2000).await.expect("second line");
        assert_eq!(b.capture.text, "오늘도 만나서 반가워요", "the jittered re-read must not be reported");

        assert!(next(&mut h, 400).await.is_none(), "a static screen produces nothing more");
    }

    #[tokio::test]
    async fn a_line_over_a_moving_scene_is_read_without_waiting_for_stillness() {
        // the background flickers on every frame, so the picture never holds still
        let frames: Vec<RgbImage> = (0..400).map(|i| frame(10, if i % 2 == 0 { 40 } else { 200 })).collect();
        let (mut h, _) = watch_frames(frames);
        let line = next_line(&mut h, 1000).await.expect("the line is found although nothing holds still");
        assert_eq!(line.capture.text, "안녕하세요 여러분");
        assert!(line.ready_ms < 500, "two agreeing reads, not a long wait: {} ms", line.ready_ms);
    }

    #[tokio::test]
    async fn a_line_that_is_gone_is_reported_and_may_come_back() {
        let mut frames = vec![frame(10, 120); 10];
        frames.extend(vec![frame(0, 20); 10]); // gone
        frames.extend(vec![frame(10, 120); 200]); // the same words again
        let (mut h, _) = watch_frames(frames);

        assert!(next_line(&mut h, 2000).await.is_some(), "first time");
        assert!(matches!(next(&mut h, 2000).await, Some(LiveEvent::Cleared)), "then it goes away");
        let again = next_line(&mut h, 2000).await.expect("the same words after a gap are a new line");
        assert_eq!(again.capture.text, "안녕하세요 여러분");
    }

    #[tokio::test]
    async fn one_missed_read_over_a_busy_background_does_not_take_the_line_down() {
        // flickering background; one frame in the middle has no readable text
        let mut frames: Vec<RgbImage> = (0..12).map(|i| frame(10, if i % 2 == 0 { 40 } else { 200 })).collect();
        frames.push(frame(0, 120)); // a miss (the picture differs from the flicker, as a real frame with a missed word would)
        frames.extend((0..300).map(|i| frame(10, if i % 2 == 0 { 40 } else { 200 })));
        let (mut h, _) = watch_frames(frames);
        assert!(next_line(&mut h, 2000).await.is_some());
        assert!(next(&mut h, 600).await.is_none(), "no Cleared and no second report for the same line");
    }

    #[tokio::test]
    async fn a_quiet_screen_costs_nothing_and_a_moving_empty_scene_is_read_slowly() {
        // blank and static: one read of the first frame, then nothing
        let (mut h, ocr) = watch_frames(vec![frame(0, 20); 400]);
        assert!(next(&mut h, 300).await.is_none());
        assert!(*ocr.calls.lock().unwrap() <= 2, "{} reads of a static blank screen", ocr.calls.lock().unwrap());
        h.stop();

        // moving but without text: far fewer reads than ticks
        let frames: Vec<RgbImage> = (0..400).map(|i| frame(0, if i % 2 == 0 { 40 } else { 200 })).collect();
        let (mut h, ocr) = watch_frames(frames);
        // a picture that has just started moving is read on every tick for a moment, in case a line appears
        assert!(next(&mut h, 800).await.is_none());
        let before = *ocr.calls.lock().unwrap();
        assert!(next(&mut h, 450).await.is_none());
        let reads = *ocr.calls.lock().unwrap() - before;
        assert!(reads <= 14, "a moving empty scene was read {reads} times in 450 ms (15 ms ticks)");
    }

    /// The overlay starts live mode from a window-event callback, which is not a Tokio thread.
    /// Using `tokio::spawn` there panicked ("there is no reactor running") and the panic, inside a
    /// native callback, killed the whole app.
    #[test]
    fn can_be_started_from_a_thread_that_is_not_inside_a_runtime() {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        assert!(tokio::runtime::Handle::try_current().is_err(), "this test thread must have no runtime");
        let ocr = Arc::new(FakeOcr::new(pages()));
        let mut h = spawn(rt.handle(), Arc::new(FakeScreen::new(vec![frame(10, 100)])), ocr, cfg(), Rect::new(0.0, 0.0, 400.0, 80.0));
        let mut rx = h.take_receiver();
        let got = rt.block_on(async { tokio::time::timeout(Duration::from_secs(2), rx.recv()).await }).expect("a line within 2s");
        let Some(LiveEvent::Line(l)) = got else { panic!("expected a line") };
        assert_eq!(l.capture.text, "안녕하세요 여러분");
        h.stop();
    }

    #[tokio::test]
    async fn the_receiver_can_be_moved_out_while_the_handle_keeps_control() {
        let (mut h, _) = watch_frames(vec![frame(10, 100)]);
        let mut rx = h.take_receiver();
        assert!(tokio::time::timeout(Duration::from_secs(1), rx.recv()).await.unwrap().is_some());
        h.stop();
        assert!(tokio::time::timeout(Duration::from_secs(1), rx.recv()).await.expect("closes after stop").is_none());
    }

    #[tokio::test]
    async fn stopping_ends_the_stream() {
        let (mut h, _) = watch_frames(vec![frame(10, 100)]);
        assert!(next(&mut h, 1000).await.is_some());
        h.stop();
        assert!(tokio::time::timeout(Duration::from_secs(1), h.rx.recv()).await.expect("channel closes").is_none());
    }
}
