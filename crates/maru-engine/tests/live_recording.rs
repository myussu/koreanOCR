//! Optional real OCR/model comparison, using a recording as the screen. No window is controlled.
//! Set MARU_REAL_HOME to an isolated, already installed portable Maru home, MARU_CLIP to a video,
//! and MARU_REAL_OUT to an output JSON file. Requires ffmpeg and local model files; downloads are
//! refused. MARU_CLIP_CROP is x:y:w:h, MARU_CLIP_START/END select seconds (defaults 0/60).
//! MARU_REAL_CASES optionally selects comma-separated case names from the comparison below.
//! Run: cargo test -p maru-engine --release --test live_recording -- --ignored --nocapture

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use image::RgbImage;
use maru_core::{AppInfo, Config, Event, Paths, Point, Rect};
use maru_engine::{BootOptions, Engine, EngineParts, LiveEvent, boot};
use maru_llm::{LlmClient, LlmStatus};
use maru_ocr::OcrEngine;
use maru_ocr::paddle::{PaddleConfig, PaddleOcr};
use maru_platform::{Platform, ScreenGrabber};
use maru_platform::testing::FakeSelection;
use maru_srs::{SrsConfig, Vocab};
use serde_json::{Value, json};

const FPS: f64 = 10.0;

struct Recording {
    frames: Arc<Vec<RgbImage>>,
    began: Instant,
}

impl ScreenGrabber for Recording {
    fn grab(&self, _: Rect) -> maru_platform::Result<RgbImage> {
        let index = (self.began.elapsed().as_secs_f64() * FPS) as usize;
        Ok(self.frames[index.min(self.frames.len() - 1)].clone())
    }
    fn cursor(&self) -> Option<Point> { None }
    fn foreground_app(&self) -> Option<AppInfo> { None }
    fn screen_bounds(&self) -> Rect { Rect::new(0.0, 0.0, 1920.0, 1080.0) }
}

fn seconds(name: &str, default: f64) -> f64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn decode() -> (Arc<Vec<RgbImage>>, Rect, f64) {
    let clip = std::env::var("MARU_CLIP").expect("MARU_CLIP points to an existing recording");
    let crop = std::env::var("MARU_CLIP_CROP").unwrap_or_else(|_| "0:730:1920:350".into());
    let dims: Vec<u32> = crop.split(':').map(|n| n.parse().expect("crop is x:y:w:h")).collect();
    assert_eq!(dims.len(), 4);
    let (x, y, w, h) = (dims[0], dims[1], dims[2], dims[3]);
    let start = seconds("MARU_CLIP_START", 0.0);
    let length = seconds("MARU_CLIP_END", 60.0) - start;
    assert!(length > 0.0 && w > 0 && h > 0);
    let out = Command::new("ffmpeg").args([
        "-v", "error", "-ss", &start.to_string(), "-i", &clip, "-t", &length.to_string(),
        "-vf", &format!("fps={FPS},crop={w}:{h}:{x}:{y}"), "-f", "rawvideo", "-pix_fmt", "rgb24", "-",
    ]).output().expect("ffmpeg is installed");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let size = (w * h * 3) as usize;
    let frames: Vec<RgbImage> = out.stdout.chunks_exact(size).map(|c| RgbImage::from_raw(w, h, c.to_vec()).unwrap()).collect();
    assert!(!frames.is_empty());
    (Arc::new(frames), Rect::new(0.0, 0.0, w as f32, h as f32), start)
}

fn percentile(lines: &[Value], key: &str, p: f64) -> Option<u64> {
    let mut values: Vec<u64> = lines.iter().filter_map(|v| v[key].as_u64()).collect();
    values.sort_unstable();
    (!values.is_empty()).then(|| values[((values.len() - 1) as f64 * p).round() as usize])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires an isolated installed home, real recording, ffmpeg and local GPU models"]
async fn compare_live_settings_with_real_ocr_and_translation_on_a_recording() {
    let home = std::env::var("MARU_REAL_HOME").expect("use an isolated portable Maru home");
    let output = std::env::var("MARU_REAL_OUT").expect("MARU_REAL_OUT is the JSON output file");
    let paths = Paths::under(Path::new(&home));
    let base = Config::load(&paths.config_file).unwrap();
    assert!(base.llm.endpoint.is_none() && base.llm.model_path.as_ref().is_some_and(|p| p.is_file()), "translation must use already installed local weights");
    assert!(maru_engine::vision::installed(&paths), "install the reader separately before this test");
    assert!(!base.learning.anki.enabled && base.tts.endpoint.is_none(), "this comparison uses no external services");
    let (frames, region, start) = decode();
    let length = frames.len() as f64 / FPS;
    let mut services = boot(paths.clone(), base.clone(), BootOptions::default()).await;
    assert_eq!(services.wait_for_llm().await, LlmStatus::Ready);
    assert_eq!(services.wait_for_vision().await, LlmStatus::Ready);
    if base.tts.enabled {
        assert_eq!(*services.tts_status.wait_for(|s| !matches!(s, LlmStatus::Starting)).await.unwrap(), LlmStatus::Ready);
    }
    let analyzer = services.engine.analyzer();
    let mut paddle = PaddleConfig::resolve(&base.ocr, &paths);
    paddle.device = "cpu".into();
    let cpu: Arc<dyn OcrEngine> = maru_engine::respace::RespacingOcr::wrap(Arc::new(PaddleOcr::load(&paddle).unwrap()), &analyzer);
    let normal = services.engine.ocr_engine().expect("the configured OCR is installed");
    assert_eq!(normal.name(), "hybrid", "baseline needs the configured CPU detector and vision reader");
    let llm = LlmClient::new(&format!("http://127.0.0.1:{}", base.llm.port), &base.llm);
    llm.warm_up().await;
    cpu.recognize(&frames[0]).unwrap();
    normal.recognize(&frames[0]).unwrap();
    let mut results = Vec::new();
    // Repeat the baseline last to expose drift from temperature, game load or server caches.
    let selected = std::env::var("MARU_REAL_CASES").ok();
    for (name, cpu_ocr, max_in_flight) in [("baseline", false, 0), ("hybrid-one", false, 1), ("cpu-unbounded", true, 0), ("cpu-one", true, 1), ("cpu-two", true, 2), ("baseline-repeat", false, 0)] {
        if selected.as_ref().is_some_and(|s| !s.split(',').any(|v| v.trim() == name)) { continue; }
        println!("recording comparison: {name} ({length:.1} seconds)");
        let mut config = base.clone();
        config.continuous.cpu_ocr = cpu_ocr;
        config.continuous.max_in_flight = max_in_flight;
        let began = Instant::now();
        let engine = Engine::new(EngineParts {
            config: config.clone(), analyzer: analyzer.clone(), dict: None,
            vocab: Arc::new(Vocab::open_in_memory(SrsConfig::from_learning(&config.learning)).unwrap()),
            llm: Some(llm.clone()), llm_status: None,
            platform: Platform { screen: Arc::new(Recording { frames: frames.clone(), began }), selection: Arc::new(FakeSelection::default()), os_ocr: None },
            ocr: Some(normal.clone()), scan_ocr: None, live_ocr: Some(cpu.clone()), labels: None,
        });
        let mut watcher = engine.start_continuous(region).unwrap();
        let mut requests = tokio::task::JoinSet::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs_f64(length);
        while let Ok(Some(event)) = tokio::time::timeout_at(deadline, watcher.rx.recv()).await {
            if let LiveEvent::Line(line) = event {
                let mut record = json!({ "text": line.capture.text, "video_s": start + began.elapsed().as_secs_f64(), "read_ms": line.ready_ms, "ocr_ms": line.ocr_ms });
                let appeared = line.appeared;
                let (id, mut rx) = engine.translate_live(*line);
                record["id"] = json!(id);
                requests.spawn(async move {
                    let mut first = None;
                    let mut errors = Vec::new();
                    while let Some(event) = rx.recv().await {
                        match event {
                            Event::Started { cached, .. } => record["cached"] = json!(cached),
                            Event::Translation { ref delta, .. } if !delta.is_empty() => { first.get_or_insert(appeared.elapsed().as_millis() as u64); }
                            Event::TranslationDone { text, .. } => record["translation"] = json!(text),
                            Event::Error { message, .. } => errors.push(message),
                            Event::Done { .. } => break,
                            _ => {}
                        }
                    }
                    record["first_ms"] = json!(first);
                    record["total_ms"] = json!(appeared.elapsed().as_millis() as u64);
                    record["errors"] = json!(errors);
                    record
                });
            }
        }
        watcher.stop();
        let mut lines = Vec::new();
        tokio::time::timeout(Duration::from_secs(90), async {
            while let Some(record) = requests.join_next().await { lines.push(record.unwrap()); }
        }).await.expect("pending translations finish within 90 seconds");
        lines.sort_by_key(|v| v["id"].as_u64());
        let errors = lines.iter().map(|v| v["errors"].as_array().unwrap().len()).sum::<usize>();
        let summary = json!({ "name": name, "cpu_ocr": cpu_ocr, "max_in_flight": max_in_flight, "lines": lines.len(), "errors": errors,
            "first_p50_ms": percentile(&lines, "first_ms", 0.5), "first_p95_ms": percentile(&lines, "first_ms", 0.95),
            "total_p50_ms": percentile(&lines, "total_ms", 0.5), "total_p95_ms": percentile(&lines, "total_ms", 0.95),
            "read_p50_ms": percentile(&lines, "read_ms", 0.5), "ocr_p50_ms": percentile(&lines, "ocr_ms", 0.5) });
        println!("{summary}");
        results.push(json!({ "summary": summary, "lines": lines, "history": engine.history() }));
        std::fs::write(&output, serde_json::to_vec_pretty(&results).unwrap()).unwrap();
        assert_eq!(errors, 0, "real OCR/model errors in {name}; inspect {output}");
    }
    // Services owns only this diagnostic run's children. Dropping it leaves the user's app alone.
    drop(services);
}
