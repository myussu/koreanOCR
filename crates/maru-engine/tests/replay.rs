//! Replays a recorded video of subtitles through the live watcher, as if it were the screen, and reports how soon
//! each line was read. This is the tool for tuning live subtitles on a real cutscene without the game.
//!
//! It needs `ffmpeg` and a clip, so it is ignored by default:
//!
//! ```text
//! MARU_CLIP=clip.mp4 MARU_CLIP_TRUTH=truth.txt cargo test -p maru-engine --test replay -- --ignored --nocapture
//! ```
//!
//! * `MARU_CLIP_TRUTH`: one subtitle per line, `onset_seconds off_seconds` (when it is on screen in the clip).
//! * `MARU_CLIP_CROP`: `x:y:w:h` of the subtitle band in the video (default `160:545:960:130`, for 1280×720).
//! * `MARU_CLIP_START` / `MARU_CLIP_END`: seconds of the clip to play (default 9 to the end).
//! * `MARU_OCR_MS`: how long the stand-in OCR takes (default 350, about PP-OCR on a CPU).
//!
//! The stand-in OCR knows the truth: it answers with that subtitle's text if one is on screen when the frame was
//! taken, and with nothing otherwise. So what is measured is the watcher's timing, not recognition quality.

use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use image::{Rgb, RgbImage};
use maru_core::config::ContinuousConfig;
use maru_core::{AppInfo, Point, Rect};
use maru_engine::continuous::{LiveEvent, spawn};
use maru_ocr::{OcrEngine, OcrLine, OcrOutput};
use maru_platform::ScreenGrabber;

const FPS: f64 = 10.0;

struct Replay {
    frames: Vec<RgbImage>,
    truth: Vec<(f64, f64)>,
    start: f64,
    began: Instant,
}

impl Replay {
    fn video_time(&self) -> f64 {
        self.start + self.began.elapsed().as_secs_f64()
    }
}

impl ScreenGrabber for Replay {
    fn grab(&self, _region: Rect) -> maru_platform::Result<RgbImage> {
        let t = self.video_time();
        let i = (((t - self.start) * FPS) as usize).min(self.frames.len() - 1);
        let mut frame = self.frames[i].clone();
        // The stand-in OCR reads which subtitle is on screen from this pixel.
        let page = self.truth.iter().position(|(on, off)| t >= *on && t < *off).map_or(0, |p| p as u8 + 1);
        frame.put_pixel(0, 0, Rgb([page, 0, 0]));
        Ok(frame)
    }
    fn cursor(&self) -> Option<Point> {
        None
    }
    fn foreground_app(&self) -> Option<AppInfo> {
        None
    }
    fn screen_bounds(&self) -> Rect {
        Rect::new(0.0, 0.0, 1280.0, 720.0)
    }
}

struct StandIn {
    ms: u64,
}

/// Distinct text per subtitle, so that consecutive lines never look like the same one.
fn text_of(page: u8) -> String {
    (0..8).map(|k| char::from_u32(0xAC00 + u32::from(page) * 197 + k * 31).unwrap_or('가')).collect()
}

impl OcrEngine for StandIn {
    fn name(&self) -> &'static str {
        "stand-in"
    }
    fn recognize(&self, image: &RgbImage) -> maru_ocr::Result<OcrOutput> {
        std::thread::sleep(Duration::from_millis(self.ms));
        let page = image.get_pixel(0, 0)[0];
        if page == 0 {
            return Ok(OcrOutput::default());
        }
        Ok(OcrOutput { lines: vec![OcrLine { text: text_of(page), rect: Rect::new(300.0, 50.0, 360.0, 30.0), words: vec![], confidence: 0.95 }] })
    }
}

fn env_f64(name: &str, default: f64) -> f64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn decode(clip: &str, crop: &str, start: f64, end: Option<f64>) -> (Vec<RgbImage>, u32, u32) {
    let dims: Vec<u32> = crop.split(':').map(|n| n.parse().expect("MARU_CLIP_CROP is x:y:w:h")).collect();
    assert_eq!(dims.len(), 4, "MARU_CLIP_CROP is x:y:w:h");
    let (x, y, w, h) = (dims[0], dims[1], dims[2], dims[3]);
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-v", "error", "-ss", &start.to_string(), "-i", clip]);
    if let Some(e) = end {
        cmd.args(["-t", &(e - start).to_string()]);
    }
    // ffmpeg's crop filter takes width:height:x:y.
    cmd.args(["-vf", &format!("fps={FPS},crop={w}:{h}:{x}:{y}"), "-f", "rawvideo", "-pix_fmt", "rgb24", "-"]);
    let out = cmd.stdout(Stdio::piped()).stderr(Stdio::inherit()).output().expect("ffmpeg must be installed");
    let size = (w * h * 3) as usize;
    let frames = out.stdout.chunks_exact(size).map(|c| RgbImage::from_raw(w, h, c.to_vec()).expect("frame size")).collect();
    (frames, w, h)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs ffmpeg and a recorded clip: see the file header"]
async fn replay_a_recorded_clip_through_the_live_watcher() {
    let clip = std::env::var("MARU_CLIP").expect("MARU_CLIP=path/to/clip.mp4");
    let truth_file = std::env::var("MARU_CLIP_TRUTH").expect("MARU_CLIP_TRUTH=path/to/truth.txt");
    let crop = std::env::var("MARU_CLIP_CROP").unwrap_or_else(|_| "160:545:960:130".into());
    let start = env_f64("MARU_CLIP_START", 9.0);
    let end = std::env::var("MARU_CLIP_END").ok().and_then(|v| v.parse().ok());
    let ocr_ms = env_f64("MARU_OCR_MS", 350.0) as u64;

    let truth: Vec<(f64, f64)> = std::fs::read_to_string(truth_file)
        .expect("truth file")
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
        })
        .collect();
    let (frames, w, h) = decode(&clip, &crop, start, end);
    assert!(!frames.is_empty(), "no frames decoded");
    let length = frames.len() as f64 / FPS;
    let replay = Arc::new(Replay { frames, truth: truth.clone(), start, began: Instant::now() });

    let mut handle = spawn(&tokio::runtime::Handle::current(), replay.clone(), Arc::new(StandIn { ms: ocr_ms }), ContinuousConfig::default(), Rect::new(0.0, 0.0, w as f32, h as f32));
    let mut lines: Vec<(f64, String, u32)> = Vec::new(); // (video time received, text, ready_ms as the watcher measured it)
    let mut cleared: Vec<f64> = Vec::new();
    let deadline = Instant::now() + Duration::from_secs_f64(length + 1.0);
    while let Ok(Some(event)) = tokio::time::timeout_at(deadline.into(), handle.rx.recv()).await {
        let t = replay.video_time();
        match event {
            LiveEvent::Line(l) => lines.push((t, l.capture.text.clone(), l.ready_ms)),
            LiveEvent::Cleared => cleared.push(t),
        }
    }
    handle.stop();

    println!("\nstand-in OCR {ocr_ms} ms; clip {start:.0}s to {:.0}s\n", start + length);
    println!("{:>3} {:>7} {:>7} {:>6}   {:>12} {:>14}   verdict", "#", "on", "off", "shown", "read after", "watcher says");
    let (mut missed, mut late, mut worst) = (0, 0, 0.0f64);
    for (i, (on, off)) in truth.iter().enumerate() {
        if *on < start || *on > start + length {
            continue;
        }
        let text = text_of(i as u8 + 1);
        let hit = lines.iter().find(|(t, s, _)| *s == text && *t >= *on - 0.05);
        match hit {
            Some((t, _, ready)) => {
                let after = t - on;
                worst = worst.max(after);
                let verdict = if *t > *off {
                    late += 1;
                    "READ AFTER IT WAS GONE"
                } else {
                    "ok"
                };
                println!("{:>3} {:>7.2} {:>7.2} {:>5.2}s   {:>11.2}s {:>13.2}s   {verdict}", i + 1, on, off, off - on, after, f64::from(*ready) / 1000.0);
            }
            None => {
                missed += 1;
                println!("{:>3} {:>7.2} {:>7.2} {:>5.2}s   {:>12} {:>14}   MISSED", i + 1, on, off, off - on, "-", "-");
            }
        }
    }
    let extra = lines.len().saturating_sub(truth.len() - missed);
    println!("\n{} lines reported for {} subtitles ({} missed, {} read after they were gone, {} extra reports); {} clears; slowest read {:.2}s after appearing", lines.len(), truth.len(), missed, late, extra, cleared.len(), worst);
    let fast = std::env::var("MARU_REPLAY_ASSERT").is_ok();
    if fast {
        assert_eq!(missed, 0, "every subtitle must be read");
        assert_eq!(late, 0, "no subtitle may be read after it is gone");
        assert!(worst < 1.3, "every subtitle must be read within 1.3 s of appearing (slowest {worst:.2}s)");
    }
}
