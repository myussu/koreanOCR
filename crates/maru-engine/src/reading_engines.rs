//! Which engine each task reads the screen with (`[reading]`, see `maru_core::reading`).
//!
//! Live subtitles and captures each get PP-OCR with their own line finder and letter reader, paired with the reading
//! model when their route asks for it. Two tasks that name the same pieces share one PP-OCR. The reading model is one
//! server shared by every task that uses it, started by the first request that needs it (`deferred_models.rs` in the
//! app), so live subtitles on PP-OCR alone never put it in video memory. A program with its own `[profile.reading]`
//! gets its engine the first time live subtitles start in it; its PP-OCR loads on its first read.

use std::sync::{Arc, Mutex};

use maru_core::config::{Config, OcrConfig};
use maru_core::reading::{self, ReadTask};
use maru_core::Paths;
use maru_ocr::OcrEngine;

use crate::hybrid_ocr::HybridOcr;

/// Builds a PP-OCR (ready to use, spacing repaired) from `[ocr]` settings.
pub(crate) type Loader = Arc<dyn Fn(&OcrConfig) -> Result<Arc<dyn OcrEngine>, String> + Send + Sync>;

/// What makes two PP-OCR instances the same.
#[derive(Debug, Clone, PartialEq)]
struct Pieces {
    det: Option<std::path::PathBuf>,
    rec: Option<std::path::PathBuf>,
    dict: Option<std::path::PathBuf>,
    tuning: (Option<f32>, Option<f32>, Option<f32>),
    device: String,
    threads: u32,
}

impl Pieces {
    fn of(o: &OcrConfig) -> Self {
        Self { det: o.det_model.clone(), rec: o.rec_model.clone(), dict: o.dict_path.clone(), tuning: (o.det_score_threshold, o.det_box_threshold, o.det_unclip_ratio), device: o.device.clone(), threads: o.threads }
    }
}

pub struct ReadingEngines {
    cfg: Config,
    paths: Paths,
    load: Loader,
    reader: Mutex<Option<Arc<dyn OcrEngine>>>,
    detectors: Mutex<Vec<(Pieces, Arc<dyn OcrEngine>)>>,
    programs: Mutex<Vec<(String, Arc<dyn OcrEngine>)>>,
}

impl ReadingEngines {
    pub(crate) fn new(cfg: Config, paths: Paths, load: Loader) -> Self {
        Self { cfg, paths, load, reader: Mutex::new(None), detectors: Mutex::new(Vec::new()), programs: Mutex::new(Vec::new()) }
    }

    /// `[ocr]` as `task` reads in `exe`. Live subtitles keep `[continuous] cpu_ocr`'s meaning: PP-OCR on the processor.
    pub fn settings(&self, task: ReadTask, exe: Option<&str>) -> reading::Resolved {
        let mut r = reading::resolve(&self.cfg, &self.paths, task, exe);
        if task == ReadTask::Live && self.cfg.continuous.cpu_ocr {
            r.ocr.device = "cpu".into();
        }
        r
    }

    /// A PP-OCR already loaded for these settings is used by the others that name the same pieces.
    pub(crate) fn remember(&self, ocr: &OcrConfig, engine: Arc<dyn OcrEngine>) {
        crate::engine::lock(&self.detectors).push((Pieces::of(ocr), engine));
    }

    /// The reading model every task that uses one shares (set once it exists).
    pub(crate) fn set_reader(&self, reader: Arc<dyn OcrEngine>) {
        *crate::engine::lock(&self.reader) = Some(reader);
    }

    /// PP-OCR for these settings: one already loaded with the same pieces, else a new one (built now, or on its first
    /// read when `lazy`, so starting live subtitles does not wait for it).
    pub(crate) fn detector(&self, ocr: &OcrConfig, lazy: bool) -> Result<Arc<dyn OcrEngine>, String> {
        let key = Pieces::of(ocr);
        if let Some((_, engine)) = crate::engine::lock(&self.detectors).iter().find(|(k, _)| *k == key) {
            return Ok(engine.clone());
        }
        let engine: Arc<dyn OcrEngine> = if lazy {
            let (load, ocr) = (self.load.clone(), ocr.clone());
            Arc::new(maru_ocr::LazyOcr::new(move || load(&ocr)))
        } else {
            (self.load)(ocr)?
        };
        self.remember(ocr, engine.clone());
        Ok(engine)
    }

    /// `detector` paired with the shared reading model when `ocr` asks for one and there is one.
    pub(crate) fn with_reader(&self, detector: Arc<dyn OcrEngine>, ocr: &OcrConfig) -> Arc<dyn OcrEngine> {
        match (ocr.vision_reader, crate::engine::lock(&self.reader).clone()) {
            (true, Some(reader)) => Arc::new(HybridOcr::new(detector, reader)),
            _ => detector,
        }
    }

    /// The engine live subtitles read with in `exe`, when its `[[profile]]` reads differently from the usual live
    /// route; `None` means the usual one.
    pub fn for_program(&self, exe: &str) -> Option<Arc<dyn OcrEngine>> {
        let own = self.cfg.profile_for(exe).is_some_and(|p| !p.reading.route(ReadTask::Live).eq(&Default::default()));
        if !own {
            return None;
        }
        let there = self.settings(ReadTask::Live, Some(exe));
        if there.ocr == self.settings(ReadTask::Live, None).ocr {
            return None;
        }
        let exe = exe.to_ascii_lowercase();
        if let Some((_, engine)) = crate::engine::lock(&self.programs).iter().find(|(e, _)| *e == exe) {
            return Some(engine.clone());
        }
        for note in &there.notes {
            tracing::warn!("{note}");
        }
        let detector = self.detector(&there.ocr, true).map_err(|e| tracing::warn!("live subtitles in {exe}: {e}")).ok()?;
        let engine = self.with_reader(detector, &there.ocr);
        tracing::info!(program = %exe, ocr = engine.name(), "live subtitles read as this program's profile says");
        crate::engine::lock(&self.programs).push((exe, engine.clone()));
        Some(engine)
    }
}

/// Whether `task` reads with the reading model by default (before a program's profile; installed or not).
pub fn uses_reader(cfg: &Config, task: ReadTask) -> bool {
    reading::table(cfg).route(task).reader.unwrap_or(cfg.ocr.vision_reader)
}

/// One line of Settings' "Read the screen" step: what reads for a task and where it runs, in plain words.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ReadingRow {
    /// `live`, `capture` or `scan`.
    pub task: &'static str,
    pub what: &'static str,
    /// "Line finder: Small and quick · letters: Standard".
    pub uses: String,
    /// `None`: PP-OCR alone.
    pub reader: Option<String>,
    pub place: String,
}

fn place(device: &str, reader: bool) -> String {
    let pp = if device.trim().eq_ignore_ascii_case("cpu") || device.trim().is_empty() { "processor" } else { "graphics card" };
    match (pp, reader) {
        ("processor", true) => "This PC · processor and graphics card".into(),
        (pp, _) => format!("This PC · {pp}"),
    }
}

impl ReadingEngines {
    /// What each task reads with, for Settings.
    pub fn rows(&self) -> Vec<ReadingRow> {
        let row = |task: &'static str, what: &'static str, r: &reading::Resolved, reader: bool| ReadingRow {
            task,
            what,
            uses: format!("Line finder: {} · letters: {}", r.detector_label, r.recogniser_label),
            reader: if reader { r.reader_label.clone() } else { None },
            place: place(&r.ocr.device, reader && r.reader_label.is_some()),
        };
        let live = self.settings(ReadTask::Live, None);
        let capture = self.settings(ReadTask::Capture, None);
        vec![row("live", "Live subtitles", &live, true), row("capture", "Captures", &capture, true), row("scan", "Screen scan", &capture, false)]
    }
}

/// What to tell the person when a capture has to wait for the reading model to load: only when captures use it, it
/// loads on demand, and it is not loaded yet.
pub fn reader_wait_notice(uses_reader: bool, on_demand: bool, status: &maru_llm::LlmStatus) -> Option<&'static str> {
    let waiting = matches!(status, maru_llm::LlmStatus::Disabled | maru_llm::LlmStatus::Starting);
    (uses_reader && on_demand && waiting).then_some("Loading the reading model for captures. This first capture waits for it (a few seconds); the next ones are quick.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbImage;
    use maru_ocr::{OcrLine, OcrOutput};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Reads one line naming the detector file it was built with.
    struct Fake(String);
    impl OcrEngine for Fake {
        fn name(&self) -> &'static str {
            "paddle"
        }
        fn recognize(&self, _: &RgbImage) -> maru_ocr::Result<OcrOutput> {
            Ok(OcrOutput { lines: vec![OcrLine { text: self.0.clone(), rect: maru_core::Rect::new(0.0, 0.0, 10.0, 10.0), confidence: 1.0, words: vec![] }] })
        }
    }

    fn engines(cfg: Config, loads: Arc<AtomicUsize>) -> (ReadingEngines, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("maru-reading-engines-{}-{}", std::process::id(), loads.as_ptr() as usize));
        for rel in ["ocr/det.onnx", "ocr/rec.onnx", "ocr/dict.txt", "ocr/pp-ocrv6-small/det.onnx"] {
            let f = root.join("models").join(rel);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, b"x").unwrap();
        }
        let load: Loader = Arc::new(move |o: &OcrConfig| {
            loads.fetch_add(1, Ordering::SeqCst);
            let name = o.det_model.as_ref().map(|p| p.file_name().unwrap().to_string_lossy().to_string() + " " + &p.parent().unwrap().file_name().unwrap().to_string_lossy()).unwrap_or_else(|| "standard".into());
            Ok(Arc::new(Fake(name)) as Arc<dyn OcrEngine>)
        });
        (ReadingEngines::new(cfg, Paths::under(&root), load), root)
    }

    fn read(engine: &Arc<dyn OcrEngine>) -> String {
        engine.recognize(&RgbImage::new(4, 4)).unwrap().text()
    }

    #[test]
    fn live_subtitles_and_captures_read_with_their_own_finders_and_only_captures_use_the_reading_model() {
        let loads = Arc::new(AtomicUsize::new(0));
        let (e, root) = engines(Config::default(), loads.clone());
        e.set_reader(Arc::new(Fake("reader".into())));
        let capture = e.settings(ReadTask::Capture, None).ocr;
        let live = e.settings(ReadTask::Live, None).ocr;
        let capture_engine = e.with_reader(e.detector(&capture, false).unwrap(), &capture);
        let live_engine = e.with_reader(e.detector(&live, false).unwrap(), &live);
        assert_eq!(capture_engine.name(), "hybrid");
        assert_eq!(live_engine.name(), "paddle", "PP-OCR alone: the reading model is never asked");
        assert_eq!(read(&live_engine), "det.onnx pp-ocrv6-small");
        assert_eq!(loads.load(Ordering::SeqCst), 2);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn tasks_that_name_the_same_pieces_share_one_pp_ocr() {
        let loads = Arc::new(AtomicUsize::new(0));
        let cfg = Config { reading: reading::ReadingConfig { version: 1, live: reading::ReadingRoute { detector: "standard".into(), reader: Some(false), ..Default::default() }, capture: reading::ReadingRoute { detector: "standard".into(), reader: Some(true), ..Default::default() } }, ..Default::default() };
        let (e, root) = engines(cfg, loads.clone());
        let capture = e.settings(ReadTask::Capture, None).ocr;
        let live = e.settings(ReadTask::Live, None).ocr;
        let a = e.detector(&capture, false).unwrap();
        let b = e.detector(&live, false).unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_program_whose_profile_keeps_the_reading_model_gets_its_own_live_engine_and_the_others_the_usual_one() {
        let loads = Arc::new(AtomicUsize::new(0));
        let mut cfg = Config::default();
        let keeps_reader = reading::ProfileReading { live: reading::ReadingRoute { reader: Some(true), ..Default::default() }, ..Default::default() };
        cfg.profiles.push(maru_core::config::AppProfile { name: "Xenoblade".into(), exe: vec!["yuzu.exe".into()], reading: keeps_reader, ..Default::default() });
        cfg.profiles.push(maru_core::config::AppProfile { name: "Plain".into(), exe: vec!["plain.exe".into()], hint: Some("a game".into()), ..Default::default() });
        let (e, root) = engines(cfg, loads.clone());
        e.set_reader(Arc::new(Fake("reader".into())));
        assert!(e.for_program("eldenring.exe").is_none(), "no profile");
        assert!(e.for_program("plain.exe").is_none(), "a profile that says nothing about reading");
        let xeno = e.for_program("Yuzu.exe").expect("its own engine");
        assert_eq!(xeno.name(), "hybrid");
        assert_eq!(loads.load(Ordering::SeqCst), 0, "its PP-OCR loads on the first read, not when live subtitles start");
        assert_eq!(read(&xeno), "det.onnx pp-ocrv6-small", "the detector's line, with the reading model asked beside it");
        assert!(Arc::ptr_eq(&xeno, &e.for_program("yuzu.exe").unwrap()), "built once");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn the_older_quick_reader_switch_still_keeps_live_pp_ocr_on_the_processor() {
        let mut cfg = Config::default();
        cfg.ocr.device = "directml:0".into();
        cfg.continuous.cpu_ocr = true;
        let (e, root) = engines(cfg, Arc::new(AtomicUsize::new(0)));
        assert_eq!(e.settings(ReadTask::Live, None).ocr.device, "cpu");
        assert_eq!(e.settings(ReadTask::Capture, None).ocr.device, "directml:0");
        std::fs::remove_dir_all(root).ok();
    }

    /// A reading model that counts how often it is asked.
    struct CountingReader(Arc<AtomicUsize>);
    impl OcrEngine for CountingReader {
        fn name(&self) -> &'static str {
            "reader"
        }
        fn recognize(&self, _: &RgbImage) -> maru_ocr::Result<OcrOutput> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(maru_ocr::OcrError::NotReady("loading".into()))
        }
    }

    #[test]
    fn live_reads_never_ask_the_reading_model_and_the_first_capture_does() {
        let (e, root) = engines(Config::default(), Arc::new(AtomicUsize::new(0)));
        let asked = Arc::new(AtomicUsize::new(0));
        e.set_reader(Arc::new(CountingReader(asked.clone())));
        let (live, capture) = (e.settings(ReadTask::Live, None).ocr, e.settings(ReadTask::Capture, None).ocr);
        let live_engine = e.with_reader(e.detector(&live, false).unwrap(), &live);
        for _ in 0..5 {
            read(&live_engine);
        }
        assert_eq!(asked.load(Ordering::SeqCst), 0, "a reading model that loads on its first request stays unloaded");
        let capture_engine = e.with_reader(e.detector(&capture, false).unwrap(), &capture);
        assert_eq!(read(&capture_engine), "standard", "while it loads, the detector's own text answers");
        assert_eq!(asked.load(Ordering::SeqCst), 1);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn settings_say_per_task_what_reads_and_where_it_runs() {
        let (e, root) = engines(Config::default(), Arc::new(AtomicUsize::new(0)));
        let rows = e.rows();
        assert_eq!(rows.iter().map(|r| r.what).collect::<Vec<_>>(), ["Live subtitles", "Captures", "Screen scan"]);
        assert_eq!(rows[0].uses, "Line finder: Small and quick · letters: Standard");
        assert_eq!((rows[0].reader.clone(), rows[0].place.as_str()), (None, "This PC · processor"));
        assert_eq!((rows[1].reader.as_deref(), rows[1].place.as_str()), (Some("PaddleOCR-VL 1.5"), "This PC · processor and graphics card"));
        assert_eq!((rows[2].reader.clone(), rows[2].uses.as_str()), (None, "Line finder: Standard · letters: Standard"));
        for r in &rows {
            assert!(!r.uses.contains("onnx") && !r.uses.contains("pp-ocr"), "no file or internal names: {}", r.uses);
        }
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_capture_says_it_waits_only_while_the_reading_model_it_uses_is_not_loaded() {
        use maru_llm::LlmStatus;
        assert!(reader_wait_notice(true, true, &LlmStatus::Disabled).is_some_and(|t| t.contains("first capture waits")));
        assert!(reader_wait_notice(true, true, &LlmStatus::Starting).is_some());
        assert_eq!(reader_wait_notice(true, true, &LlmStatus::Ready), None);
        assert_eq!(reader_wait_notice(false, true, &LlmStatus::Disabled), None, "captures without the reading model");
        assert_eq!(reader_wait_notice(true, false, &LlmStatus::Disabled), None, "started at boot, not on demand");
    }

    #[test]
    fn whether_a_task_uses_the_reading_model_follows_the_table_then_the_older_switch() {
        let cfg = Config::default();
        assert!(!uses_reader(&cfg, ReadTask::Live));
        assert!(uses_reader(&cfg, ReadTask::Capture));
        let mut off = Config::default();
        off.ocr.vision_reader = false;
        assert!(!uses_reader(&off, ReadTask::Capture));
    }
}
