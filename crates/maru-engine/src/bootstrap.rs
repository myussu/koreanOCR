//! Assemble an [`Engine`] from whatever is installed on this machine.
//!
//! Nothing here is fatal. A missing Kiwi library, dictionary, OCR model or
//! language model downgrades one capability and adds a plain-language note
//! (shown by `maru doctor` and in the overlay's status line); the rest keeps working.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use maru_core::config::OcrBackend;
use maru_core::{Config, Paths};
use maru_dict::Dictionary;
use maru_lang::Analyzer;
use maru_llm::{LlamaServer, LlmClient, LlmStatus, ModelSource};
use maru_ocr::OcrEngine;
use maru_platform::Platform;
use maru_srs::{SrsConfig, Vocab};
use tokio::sync::watch;

use crate::engine::{Engine, EngineParts};
use crate::hybrid_ocr::HybridOcr;
use crate::vision;
use crate::vlm_ocr::{PADDLE_VL_PROMPT, VlmOcr};

#[derive(Debug, Clone, Copy)]
pub struct BootOptions {
    /// Spawn `llama-server` (in the background) when a model is configured.
    pub start_llm: bool,
    /// Interactive front ends leave GPU models unloaded until a feature requests them.
    pub defer_models: bool,
}

impl Default for BootOptions {
    fn default() -> Self {
        Self { start_llm: true, defer_models: false }
    }
}

/// The engine plus everything that must stay alive with it.
pub struct Services {
    pub engine: Engine,
    pub paths: Paths,
    /// Human-readable status lines, one per notable decision or problem.
    pub notes: Vec<String>,
    /// The lite profile is in effect.
    pub lite: bool,
    /// Updates as the model loads.
    pub llm_status: watch::Receiver<LlmStatus>,
    /// Updates as the OCR reading model (PaddleOCR-VL) loads; `Disabled` when it is not in use.
    pub vision_status: watch::Receiver<LlmStatus>,
    /// Speech, when it is switched on and set up (see `[tts]`); `None` otherwise.
    pub tts: Option<maru_tts::Tts>,
    /// Updates as the speech server starts (a remote one is just checked); `Disabled` when speech is off.
    pub tts_status: watch::Receiver<LlmStatus>,
    /// Keeps the local `tts-server` alive.
    pub(crate) tts_server: Arc<Mutex<Option<maru_tts::local::LocalTts>>>,
    /// Keeps `llama-server` alive; it is killed when `Services` is dropped.
    pub(crate) server: Arc<Mutex<Option<LlamaServer>>>,
    /// Same, for the OCR reading model.
    pub(crate) vision_server: Arc<Mutex<Option<LlamaServer>>>,
    pub(crate) model_runtime: Option<Arc<maru_llm::model_runtime::ModelRuntime>>,
    pub(crate) deferred_vision: Option<Arc<crate::deferred_models::DeferredModel>>,
    pub(crate) deferred_speech: Option<Arc<crate::deferred_models::DeferredModel>>,
    pub(crate) model_controls: crate::model_controls::ModelControls,
}

impl Services {
    pub fn llm_status(&self) -> LlmStatus {
        self.llm_status.borrow().clone()
    }
    pub fn live_llm_status(&self) -> LlmStatus {
        self.model_runtime.as_ref().map(|r| r.status()).unwrap_or_else(|| self.llm_status())
    }
    pub async fn wait_for_llm(&mut self) -> LlmStatus {
        let _ = self.llm_status.wait_for(|s| !matches!(s, LlmStatus::Starting)).await;
        self.llm_status()
    }
    pub fn vision_status(&self) -> LlmStatus {
        self.vision_status.borrow().clone()
    }
    pub async fn wait_for_vision(&mut self) -> LlmStatus {
        let _ = self.vision_status.wait_for(|s| !matches!(s, LlmStatus::Starting)).await;
        self.vision_status()
    }
    pub fn tts_status(&self) -> LlmStatus {
        self.tts_status.borrow().clone()
    }
    pub fn vision_on_demand(&self) -> bool { self.deferred_vision.is_some() }
    pub fn speech_on_demand(&self) -> bool { self.deferred_speech.is_some() }
    pub async fn ensure_speech(&self) -> Result<(), String> {
        if let Some(model) = &self.deferred_speech { model.ensure_loaded().await?; }
        match self.tts_status() {
            LlmStatus::Ready => Ok(()),
            LlmStatus::Starting => Err("The voice is still loading; try again in a moment.".into()),
            LlmStatus::Failed(why) => Err(format!("The voice is not available: {why}")),
            LlmStatus::Disabled => Err("Speech is off.".into()),
        }
    }
    /// The local speech server is up (always false for a remote one).
    pub fn tts_server_running(&self) -> bool {
        self.tts_server.lock().map(|mut s| s.as_mut().is_some_and(|s| s.is_running())).unwrap_or(false)
    }
    pub fn vision_server_running(&self) -> bool {
        self.vision_server.lock().map(|s| s.is_some()).unwrap_or(false)
    }
    pub fn llm_server_running(&self) -> bool {
        self.model_runtime.as_ref().is_some_and(|r| r.is_running()) || self.server.lock().map(|s| s.is_some()).unwrap_or(false)
    }
}

fn kiwi_library_name() -> &'static str {
    if cfg!(windows) {
        "kiwi.dll"
    } else if cfg!(target_os = "macos") {
        "libkiwi.dylib"
    } else {
        "libkiwi.so"
    }
}

/// Where `maru setup kiwi` puts Kiwi (`KIWI_RS_CACHE_DIR` points here during setup).
pub fn kiwi_cache_dir(paths: &Paths) -> PathBuf {
    paths.bin_dir.join("kiwi")
}

/// Marker written next to a Kiwi install once it has loaded and tokenized in a child process.
/// Kiwi is a native library bound through a C ABI; a mismatched build crashes the process
/// (an access violation), so only installs that passed the check are ever loaded in-process.
pub const PROBE_MARKER: &str = "probe.ok";
/// Hidden command-line flag: run the Kiwi self-test and exit (see [`run_probe_if_requested`]).
pub const PROBE_FLAG: &str = "--maru-probe-kiwi";

/// Call first thing in `main` of every Maru program: if this process was started as the Kiwi
/// self-test, run it and exit; otherwise return immediately.
pub fn run_probe_if_requested() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some(PROBE_FLAG) || args.len() < 4 {
        return;
    }
    let code = match maru_lang::kiwi::KiwiAnalyzer::spawn_from_paths(PathBuf::from(&args[2]), PathBuf::from(&args[3])) {
        Ok(analyzer) => {
            use maru_lang::Morphology;
            let samples = ["안녕하세요, 한국어를 공부해요.", "저는 학교에서 친구랑 같이 영화를 봤어요!", "열심히 노력하면 꼭 성공할 수 있을 거예요", "Hello 2024 한국어"];
            match samples.iter().map(|t| analyzer.tokenize(t)).collect::<Result<Vec<_>, _>>() {
                Ok(all) if all.iter().all(|t| !t.is_empty()) => {
                    println!("OK");
                    0
                }
                Ok(_) => {
                    eprintln!("Kiwi returned no tokens");
                    3
                }
                Err(e) => {
                    eprintln!("{e}");
                    4
                }
            }
        }
        Err(e) => {
            eprintln!("{e}");
            2
        }
    };
    std::process::exit(code);
}

/// Run the Kiwi self-test in a separate process so a crash cannot take this one down.
pub fn probe_kiwi(library: &Path, model: &Path) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut child = std::process::Command::new(exe)
        .arg(PROBE_FLAG)
        .arg(library)
        .arg(model)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start the Kiwi self-test: {e}"))?;
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if std::time::Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                let _ = child.kill();
                return Err("the Kiwi self-test did not finish within 60 seconds".into());
            }
            Err(e) => return Err(e.to_string()),
        }
    };
    if status.success() {
        return Ok(());
    }
    let mut stderr = String::new();
    if let Some(mut e) = child.stderr.take() {
        let _ = std::io::Read::read_to_string(&mut e, &mut stderr);
    }
    let code = status.code().map(|c| format!("exit code {c} (0x{:08X})", c as u32)).unwrap_or_else(|| "killed by a signal".into());
    Err(format!("the Kiwi self-test crashed or failed: {code} {}", stderr.trim()))
}

/// Newest installed Kiwi under our cache that passed the self-test: `(library, model_dir)`.
pub fn find_kiwi(paths: &Paths) -> Option<(PathBuf, PathBuf)> {
    let root = kiwi_cache_dir(paths).join("kiwi-rs");
    let mut versions: Vec<PathBuf> = std::fs::read_dir(root).ok()?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()).collect();
    versions.sort();
    versions.into_iter().rev().find_map(|v| {
        let lib = v.join("lib").join(kiwi_library_name());
        let model = v.join("models").join("cong").join("base");
        (lib.exists() && model.exists() && v.join(PROBE_MARKER).exists()).then_some((lib, model))
    })
}

fn load_analyzer(paths: &Paths, notes: &mut Vec<String>) -> Analyzer {
    use maru_lang::kiwi::KiwiAnalyzer;
    let Some((lib, model)) = find_kiwi(paths) else {
        notes.push("Korean analyser: simplified rules (word breakdown is approximate). Run `maru setup kiwi` for accurate grammar analysis.".into());
        return Analyzer::heuristic();
    };
    match KiwiAnalyzer::spawn_from_paths(lib, model) {
        Ok(k) => {
            notes.push("Korean analyser: Kiwi".into());
            Analyzer::new(Arc::new(k))
        }
        Err(e) => {
            notes.push(format!("Korean analyser: Kiwi failed to load ({e}); using simplified rules. Run `maru setup kiwi` again."));
            Analyzer::heuristic()
        }
    }
}

fn open_dictionary(paths: &Paths, notes: &mut Vec<String>) -> Option<Arc<Dictionary>> {
    let db = paths.dict_db();
    if !db.exists() {
        notes.push("Dictionary: not installed. Run `maru dict download` (or `maru dict import --kaikki <file>`).".into());
        return None;
    }
    match Dictionary::open(&db) {
        Ok(d) if !d.is_empty() => {
            notes.push(format!("Dictionary: {} entries", d.entry_count().unwrap_or(0)));
            Some(Arc::new(d))
        }
        Ok(_) => {
            notes.push("Dictionary: the database is empty. Run `maru dict download`.".into());
            None
        }
        Err(e) => {
            notes.push(format!("Dictionary: could not open {}: {e}", db.display()));
            None
        }
    }
}

/// Longest image side the screen-scan detector sees at full size: a 1080p screen plus padding.
const SCAN_DETECT_SIDE: u32 = 2048;

#[cfg(feature = "paddle")]
fn load_paddle(cfg: &Config, paths: &Paths, detect_side: Option<u32>) -> Result<Arc<dyn OcrEngine>, String> {
    use maru_ocr::paddle::{PaddleConfig, PaddleOcr};
    let mut pc = PaddleConfig::resolve(&cfg.ocr, paths);
    pc.detect_side = detect_side;
    // ONNX Runtime failing to load can panic inside `ort`; never let that take the app down.
    match std::panic::catch_unwind(|| PaddleOcr::load(&pc)) {
        Ok(Ok(engine)) => Ok(Arc::new(engine)),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("ONNX Runtime could not be loaded (set ORT_DYLIB_PATH, or place onnxruntime.dll next to the program)".into()),
    }
}

#[cfg(not(feature = "paddle"))]
fn load_paddle(_: &Config, _: &Paths, _: Option<u32>) -> Result<Arc<dyn OcrEngine>, String> {
    Err("this build has no PP-OCR support".into())
}

fn choose_ocr(cfg: &Config, paths: &Paths, platform: &Platform, notes: &mut Vec<String>) -> Option<Arc<dyn OcrEngine>> {
    let windows = platform.os_ocr.clone();
    let paddle = match cfg.ocr.backend {
        OcrBackend::Windows => Err("not selected".to_string()),
        _ => load_paddle(cfg, paths, None),
    };
    let chosen = match (cfg.ocr.backend, paddle) {
        (OcrBackend::Windows, _) => windows,
        (_, Ok(p)) => {
            notes.push(format!("OCR: PP-OCR on {}", cfg.ocr.device));
            Some(p)
        }
        (OcrBackend::Paddle, Err(why)) => {
            notes.push(format!("OCR: PP-OCR is unavailable ({why}). Run `maru setup ocr`."));
            return None;
        }
        (OcrBackend::Auto, Err(why)) => {
            if windows.is_none() {
                notes.push(format!("OCR: unavailable — {why}. Selecting text still works; OCR is needed for images, video and games. Run `maru setup ocr`."));
            }
            windows
        }
    };
    if chosen.as_ref().is_some_and(|e| e.name() == "windows") {
        notes.push("OCR: Windows built-in OCR".into());
    } else if chosen.is_none() && cfg.ocr.backend == OcrBackend::Windows {
        notes.push("OCR: Windows OCR is unavailable (add the Korean OCR language pack in Windows settings).".into());
    }
    chosen
}

/// Build the engine. Never fails: problems become entries in [`Services::notes`].
pub async fn boot(paths: Paths, cfg: Config, opts: BootOptions) -> Services {
    boot_with_platform(paths, cfg, opts, None).await
}

pub(crate) async fn boot_with_platform(paths: Paths, cfg: Config, opts: BootOptions, platform: Option<Platform>) -> Services {
    let mut notes = Vec::new();
    let (cfg, lite) = cfg.effective();
    if lite {
        notes.push(format!(
            "Lite profile: no OCR reading model, {} model slot(s), context {}{}.",
            cfg.llm.parallel,
            cfg.llm.ctx_size,
            cfg.llm.hf_repo.as_deref().map(|r| format!(", translation model {r}")).unwrap_or_default()
        ));
    }
    if let Err(e) = paths.ensure() {
        notes.push(format!("Could not create {}: {e}", paths.data_dir.display()));
    }

    tracing::info!("boot: loading the Korean analyser");
    let analyzer = {
        let paths = paths.clone();
        match tokio::task::spawn_blocking(move || {
            let mut n = Vec::new();
            let a = load_analyzer(&paths, &mut n);
            (a, n)
        })
        .await
        {
            Ok((a, n)) => {
                notes.extend(n);
                a
            }
            Err(_) => Analyzer::heuristic(),
        }
    };

    tracing::info!("boot: opening the dictionary");
    let dict = open_dictionary(&paths, &mut notes);
    tracing::info!("boot: opening the vocabulary");

    let srs = SrsConfig::from_learning(&cfg.learning);
    let vocab = match Vocab::open(&paths.vocab_db(), srs) {
        Ok(v) => v,
        Err(e) => {
            notes.push(format!("Vocabulary: could not open the database ({e}); this session will not be saved."));
            Vocab::open_in_memory(srs).expect("in-memory database always opens")
        }
    };

    tracing::info!("boot: setting up screen access (UI Automation, Windows OCR)");
    let platform = if let Some(platform) = platform { platform } else {
        let blocking = tokio::task::spawn_blocking(Platform::native).await;
        blocking.unwrap_or_else(|_| Platform::native())
    };
    tracing::info!("boot: loading OCR");
    let (ocr, scan_ocr) = {
        let (blocking_cfg, blocking_paths) = (cfg.clone(), paths.clone());
        let platform = platform.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut n = Vec::new();
            let o = choose_ocr(&blocking_cfg, &blocking_paths, &platform, &mut n);
            (o, n)
        })
        .await;
        let chosen = match result {
            Ok((o, n)) => {
                notes.extend(n);
                o
            }
            Err(_) => None,
        };
        // Whole-screen scans use their own copy of PP-OCR that looks at the full resolution. It is
        // built on first use, so sessions that never scan pay nothing. (With Windows OCR as the
        // engine there is nothing to tune: scans simply use it.)
        let scan = chosen.as_ref().filter(|o| o.name() == "paddle").map(|_| {
            let (cfg, paths) = (cfg.clone(), paths.clone());
            Arc::new(maru_ocr::LazyOcr::new(move || load_paddle(&cfg, &paths, Some(SCAN_DETECT_SIDE)))) as Arc<dyn OcrEngine>
        });
        (chosen.map(|o| crate::respace::RespacingOcr::wrap(o, &analyzer)), scan)
    };

    let live_ocr = if cfg.continuous.cpu_ocr {
        if cfg.ocr.device == "cpu" && ocr.as_ref().is_some_and(|o| o.name() == "paddle") {
            ocr.clone()
        } else {
            let (mut live_cfg, live_paths) = (cfg.clone(), paths.clone());
            live_cfg.ocr.device = "cpu".into();
            let loaded = tokio::task::spawn_blocking(move || load_paddle(&live_cfg, &live_paths, None)).await.unwrap_or_else(|e| Err(e.to_string()));
            match loaded {
                Ok(o) => Some(crate::respace::RespacingOcr::wrap(o, &analyzer)),
                Err(why) => {
                    notes.push(format!("Live CPU PP-OCR is unavailable ({why}); turn off continuous.cpu_ocr or run `maru setup ocr`."));
                    None
                }
            }
        }
    } else {
        None
    };

    // Screen-scan labels are remembered between sessions.
    let labels = match crate::labels::LabelCache::open(&paths.labels_db()) {
        Ok(c) => Arc::new(c),
        Err(e) => {
            notes.push(format!("Screen labels: could not open {} ({e}); they will not be remembered.", paths.labels_db().display()));
            Arc::new(crate::labels::LabelCache::open_in_memory().expect("an in-memory database always opens"))
        }
    };

    // Vision reader: interactive boot waits until snapshot OCR needs it; CLI boot stays eager.
    let (vision_tx, vision_rx) = watch::channel(LlmStatus::Disabled);
    let vision_server: Arc<Mutex<Option<LlamaServer>>> = Arc::new(Mutex::new(None));
    let vision_ready = Arc::new(AtomicBool::new(false));
    let use_vision = opts.start_llm && cfg.ocr.vision_reader && ocr.is_some() && vision::installed(&paths);
    let deferred_vision = (use_vision && opts.defer_models).then(|| {
        let (paths, llm, server) = (paths.clone(), cfg.llm.clone(), vision_server.clone());
        let stop_server = server.clone();
        crate::deferred_models::DeferredModel::restartable(vision_tx.clone(), move || {
            let (paths, llm, server) = (paths.clone(), llm.clone(), server.clone());
            async move {
                match vision::start(&paths, vision::launch_plans(&paths, &llm, &[]), |_, _| {}).await {
                    Some((model, _)) => {
                        *server.lock().unwrap_or_else(|p| p.into_inner()) = Some(model);
                        Ok(())
                    }
                    None => Err("PaddleOCR-VL did not start (run `maru --verbose doctor` for details)".into()),
                }
            }
        }, move || {
            let server = stop_server.clone();
            async move {
                let owned = server.lock().unwrap_or_else(|p| p.into_inner()).take();
                if let Some(mut owned) = owned { owned.stop().await.map_err(|e| e.to_string())?; }
                Ok(())
            }
        })
    });
    if cfg.ocr.vision_reader && ocr.is_some() && !vision::installed(&paths) && opts.start_llm {
        notes.push("OCR: PaddleOCR-VL is not installed; run `maru setup ocr-vl` for noticeably more accurate letters.".into());
    }
    let ocr = if use_vision {
        if opts.defer_models {
            notes.push("OCR: PaddleOCR-VL loads on the first OCR request; the optional CPU live reader bypasses it.".into());
        } else {
            let _ = vision_tx.send(LlmStatus::Starting);
            notes.push("OCR: PaddleOCR-VL reader is loading in the background (PP-OCR alone until it is ready)".into());
        }
        ocr.map(|detector| {
            let url = format!("http://127.0.0.1:{}", vision::PORT);
            let reader = Arc::new(VlmOcr::new(&url, &vision::base_config(&cfg.llm), PADDLE_VL_PROMPT));
            let gated: Arc<dyn OcrEngine> = match &deferred_vision {
                Some(model) => Arc::new(crate::deferred_models::DeferredOcr { inner: reader, model: model.clone() }),
                None => Arc::new(vision::GatedOcr::new(reader, vision_ready.clone())),
            };
            Arc::new(HybridOcr::new(detector, gated)) as Arc<dyn OcrEngine>
        })
    } else {
        ocr
    };

    // Language model: external endpoint, or our own supervised llama-server.
    let source = ModelSource::from_config(&cfg.llm);
    let (status_tx, status_rx) = watch::channel(LlmStatus::Disabled);
    let server: Arc<Mutex<Option<LlamaServer>>> = Arc::new(Mutex::new(None));
    let mut llm = if cfg.llm.endpoint.is_some() {
        notes.push(format!("Language model: external endpoint {}", cfg.llm.endpoint.as_deref().unwrap_or("")));
        let _ = status_tx.send(LlmStatus::Ready);
        Some(LlmClient::from_config(&cfg.llm))
    } else if source.is_some() && cfg.llm.autostart {
        let _ = status_tx.send(if opts.start_llm { LlmStatus::Starting } else { LlmStatus::Disabled });
        opts.start_llm.then(|| LlmClient::from_config(&cfg.llm))
    } else {
        notes.push("Language model: not configured. Set [llm] hf_repo or model_path in config.toml, then run `maru setup llm`.".into());
        None
    };

    let mut model_runtime = None;
    let mut live_llm = None;
    let alternate = cfg.continuous.translation.model != maru_core::live_model::LiveModel::Main;
    if alternate && opts.start_llm {
        match crate::live_models::local_config(&cfg) {
            Ok(local) => {
                let main = (cfg.llm.endpoint.is_none() && source.is_some() && cfg.llm.autostart).then(|| cfg.llm.clone());
                // A local live model's lifecycle must not gate the independent remote main client.
                let runtime_status = if cfg.llm.endpoint.is_some() { watch::channel(LlmStatus::Disabled).0 } else { status_tx.clone() };
                let runtime = maru_llm::model_runtime::ModelRuntime::new(main, local.clone(), paths.clone(), runtime_status);
                if cfg.llm.endpoint.is_none() {
                    llm = llm.map(|c| c.with_runtime(runtime.clone(), maru_llm::model_runtime::ModelRole::Main));
                }
                live_llm = Some(LlmClient::from_config(&local).with_runtime(runtime.clone(), maru_llm::model_runtime::ModelRole::Live));
                if cfg.llm.endpoint.is_none() { let _ = status_tx.send(LlmStatus::Disabled); }
                notes.push("Live translation model: loaded on demand; local Gemma is unloaded when switching. Snapshots return to the normal model when live is off.".into());
                model_runtime = Some(runtime);
            }
            Err(why) => {
                notes.push(format!("Live translation model unavailable: {why}"));
                if cfg.llm.endpoint.is_none() { let _ = status_tx.send(LlmStatus::Failed(why)); }
            }
        }
    }
    if !alternate && opts.start_llm && opts.defer_models && cfg.llm.endpoint.is_none() && source.is_some() && cfg.llm.autostart {
        let runtime = maru_llm::model_runtime::ModelRuntime::new(Some(cfg.llm.clone()), cfg.llm.clone(), paths.clone(), status_tx.clone());
        llm = llm.map(|c| c.with_runtime(runtime.clone(), maru_llm::model_runtime::ModelRole::Main));
        let _ = status_tx.send(LlmStatus::Disabled);
        notes.push("Language model: configured; loads on the first translation request.".into());
        model_runtime = Some(runtime);
    }

    tracing::info!("boot: starting the engine");
    let engine = Engine::new(EngineParts {
        config: cfg.clone(),
        analyzer,
        dict,
        vocab: Arc::new(vocab),
        llm,
        llm_status: Some(status_rx.clone()),
        platform,
        ocr,
        scan_ocr,
        live_ocr,
        live_llm,
        labels: Some(labels),
    });

    if !alternate && !opts.defer_models && opts.start_llm && cfg.llm.endpoint.is_none() && source.is_some() && cfg.llm.autostart {
        let (engine, cfg, paths, server) = (engine.clone(), cfg.clone(), paths.clone(), server.clone());
        tokio::spawn(async move {
            let timeout = Duration::from_secs(cfg.llm.ready_timeout_secs);
            match LlamaServer::start(&cfg.llm, &paths, timeout).await {
                Ok(s) => {
                    if let Ok(mut slot) = server.lock() {
                        *slot = Some(s);
                    }
                    let _ = status_tx.send(LlmStatus::Ready);
                    // Fill the KV cache with the task prompts so the first real request is quick.
                    if let Some(h) = engine.warm_up() {
                        let _ = h.await;
                    }
                }
                Err(e) => {
                    tracing::warn!("language model did not start: {e}");
                    let _ = status_tx.send(LlmStatus::Failed(e.to_string()));
                }
            }
        });
    }

    if use_vision && !opts.defer_models {
        let (paths, llm, server, ready) = (paths.clone(), cfg.llm.clone(), vision_server.clone(), vision_ready.clone());
        tokio::spawn(async move {
            let plans = vision::launch_plans(&paths, &llm, &[]);
            match vision::start(&paths, plans, |_, _| {}).await {
                Some((s, _)) => {
                    if let Ok(mut slot) = server.lock() {
                        *slot = Some(s);
                    }
                    ready.store(true, Ordering::Release);
                    let _ = vision_tx.send(LlmStatus::Ready);
                }
                None => {
                    tracing::warn!("the OCR reading model did not start; OCR stays PP-OCR only");
                    let _ = vision_tx.send(LlmStatus::Failed("PaddleOCR-VL did not start (run `maru --verbose doctor` for details)".into()));
                }
            }
        });
    }

    // Speech. A remote server costs nothing here, so it is only checked; the local one loads a model,
    // so it starts in the background and is left out of game mode unless asked for.
    let (tts_tx, tts_rx) = watch::channel(LlmStatus::Disabled);
    let tts_server: Arc<Mutex<Option<maru_tts::local::LocalTts>>> = Arc::new(Mutex::new(None));
    let (tts, deferred_speech) = start_speech(&cfg, &paths, lite, opts, &mut notes, tts_tx, tts_server.clone());

    Services { engine, paths, notes, lite, tts, tts_status: tts_rx, tts_server, llm_status: status_rx, vision_status: vision_rx, server, vision_server, model_runtime, deferred_vision, deferred_speech, model_controls: Default::default() }
}

fn start_speech(
    cfg: &Config,
    paths: &Paths,
    lite: bool,
    opts: BootOptions,
    notes: &mut Vec<String>,
    status: watch::Sender<LlmStatus>,
    server: Arc<Mutex<Option<maru_tts::local::LocalTts>>>,
) -> (Option<maru_tts::Tts>, Option<Arc<crate::deferred_models::DeferredModel>>) {
    use maru_core::config::TtsProvider;
    if !cfg.tts.enabled {
        return (None, None);
    }
    let tts = match maru_tts::Tts::new(&cfg.tts, paths) {
        Ok(t) => t,
        Err(e) => {
            notes.push(format!("Speech: {e}"));
            let _ = status.send(LlmStatus::Failed(e.to_string()));
            return (None, None);
        }
    };
    match cfg.tts.provider {
        TtsProvider::Endpoint => {
            notes.push(format!("Speech: from {}", tts.client().root()));
            let _ = status.send(LlmStatus::Starting);
            let client = tts.clone();
            if opts.start_llm {
                tokio::spawn(async move {
                    let _ = status.send(match client.client().check().await {
                        Ok(()) => LlmStatus::Ready,
                        Err(e) => LlmStatus::Failed(e.to_string()),
                    });
                });
            }
        }
        TtsProvider::Local => {
            if lite && !cfg.tts.run_in_lite {
                notes.push("Speech: not started in game mode (it would use the GPU). Set [tts] run_in_lite = true, or use a speech server on another PC.".into());
                let _ = status.send(LlmStatus::Failed("not started in game mode".into()));
            } else if !maru_tts::local::installed(&cfg.tts, paths) {
                notes.push("Speech: the local voice is not installed; run `maru setup tts`.".into());
                let _ = status.send(LlmStatus::Failed("not installed (run `maru setup tts`)".into()));
            } else if opts.start_llm && opts.defer_models {
                let (tts_cfg, paths) = (cfg.tts.clone(), paths.clone());
                let stop_server = server.clone();
                let model = crate::deferred_models::DeferredModel::restartable(status, move || {
                    let (tts_cfg, paths, server) = (tts_cfg.clone(), paths.clone(), server.clone());
                    async move {
                        let model = maru_tts::local::LocalTts::start(&tts_cfg, &paths).await.map_err(|e| e.to_string())?;
                        *server.lock().unwrap_or_else(|p| p.into_inner()) = Some(model);
                        Ok(())
                    }
                }, move || {
                    let server = stop_server.clone();
                    async move {
                        let owned = server.lock().unwrap_or_else(|p| p.into_inner()).take();
                        if let Some(mut owned) = owned { owned.stop().await.map_err(|e| e.to_string())?; }
                        Ok(())
                    }
                });
                notes.push("Speech: local voice loads on the first speech request.".into());
                return (Some(tts), Some(model));
            } else if opts.start_llm {
                let _ = status.send(LlmStatus::Starting);
                let (tts_cfg, paths) = (cfg.tts.clone(), paths.clone());
                tokio::spawn(async move {
                    match maru_tts::local::LocalTts::start(&tts_cfg, &paths).await {
                        Ok(s) => {
                            if let Ok(mut slot) = server.lock() {
                                *slot = Some(s);
                            }
                            let _ = status.send(LlmStatus::Ready);
                        }
                        Err(e) => {
                            tracing::warn!("speech server did not start: {e}");
                            let _ = status.send(LlmStatus::Failed(e.to_string()));
                        }
                    }
                });
            }
        }
    }
    (Some(tts), None)
}

/// Convenience: `<MARU_HOME or platform dir>/config.toml` → [`Config`].
pub fn load_config(paths: &Paths) -> maru_core::Result<Config> {
    Config::load(&paths.config_file)
}

#[allow(dead_code)]
fn _assert_send(_: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_paths(name: &str) -> Paths {
        let dir = std::env::temp_dir().join(format!("maru-boot-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Paths::under(&dir)
    }

    #[test]
    fn kiwi_is_found_in_our_cache_layout_and_newest_version_wins() {
        let paths = temp_paths("kiwi");
        assert!(find_kiwi(&paths).is_none());
        for v in ["v0.21.0", "v0.22.2"] {
            let base = kiwi_cache_dir(&paths).join("kiwi-rs").join(v);
            std::fs::create_dir_all(base.join("lib")).unwrap();
            std::fs::create_dir_all(base.join("models/cong/base")).unwrap();
            std::fs::write(base.join("lib").join(kiwi_library_name()), b"x").unwrap();
        }
        assert!(find_kiwi(&paths).is_none(), "an install that never passed the self-test is not used");
        for v in ["v0.21.0", "v0.22.2"] {
            std::fs::write(kiwi_cache_dir(&paths).join("kiwi-rs").join(v).join(PROBE_MARKER), b"ok").unwrap();
        }
        let (lib, model) = find_kiwi(&paths).unwrap();
        assert!(lib.to_string_lossy().contains("v0.22.2") && model.ends_with("models/cong/base"));
        // a half-installed version (no model) is skipped
        std::fs::remove_dir_all(kiwi_cache_dir(&paths).join("kiwi-rs/v0.22.2/models")).unwrap();
        assert!(find_kiwi(&paths).unwrap().0.to_string_lossy().contains("v0.21.0"));
        std::fs::remove_dir_all(paths.config_file.parent().unwrap()).ok();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_empty_machine_still_boots_with_explanatory_notes() {
        let paths = temp_paths("empty");
        let services = boot(paths.clone(), Config::default(), BootOptions::default()).await;
        let text = services.notes.join("\n");
        for needle in ["simplified rules", "Dictionary: not installed", "Language model: not configured"] {
            assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
        }
        assert_eq!(services.llm_status(), LlmStatus::Disabled);
        assert!(!services.engine.has_llm());

        // and the offline part of the product works
        let (_, mut rx) = services.engine.translate(maru_core::Capture::typed("학교에서 공부했어요"));
        let mut saw_breakdown = false;
        while let Some(e) = rx.recv().await {
            saw_breakdown |= matches!(e, maru_core::Event::Breakdown { .. });
            if matches!(e, maru_core::Event::Done { .. }) {
                break;
            }
        }
        assert!(saw_breakdown);
        std::fs::remove_dir_all(paths.config_file.parent().unwrap()).ok();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn interactive_boot_leaves_configured_gpu_models_unloaded_until_their_features_request_them() {
        let paths = temp_paths("interactive");
        paths.ensure().unwrap();
        let mut cfg = Config::default();
        cfg.ocr.backend = OcrBackend::Windows;
        cfg.llm.hf_repo = Some("scripted/model".into());
        cfg.llm.server_path = Some(paths.bin_dir.join("missing-translation-server"));
        cfg.tts.enabled = true;
        cfg.tts.server_path = Some(paths.bin_dir.join("missing-speech-server"));
        // Installed placeholders suffice for startup discovery; no executable or weights run.
        for file in [maru_tts::local::server_binary(&cfg.tts, &paths), maru_tts::local::talker_path(&cfg.tts, &paths), maru_tts::local::codec_path(&cfg.tts, &paths)] {
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, b"placeholder").unwrap();
        }
        for file in vision::FILES {
            let file = vision::model_dir(&paths).join(file);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, b"placeholder").unwrap();
        }
        let services = boot(paths.clone(), cfg, BootOptions { defer_models: true, ..Default::default() }).await;
        assert!(services.engine.has_llm());
        assert_eq!(services.llm_status(), LlmStatus::Disabled);
        assert_eq!(services.tts_status(), LlmStatus::Disabled);
        assert!(services.speech_on_demand());
        assert_eq!(services.vision_status(), LlmStatus::Disabled);
        if services.engine.has_ocr() { assert!(services.vision_on_demand()); }
        assert!(!services.llm_server_running() && !services.vision_server_running() && !services.tts_server_running());
        assert!(services.resource_targets().iter().filter(|t| t.role != "app").all(|t| t.pid.is_none()));
        // Reads and an idle interval cannot start the configured (unusable) executables.
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(services.llm_status(), LlmStatus::Disabled);
        let (_, mut events) = services.engine.translate(maru_core::Capture::typed("안녕하세요"));
        let mut reported = false;
        while let Some(event) = events.recv().await {
            reported |= matches!(event, maru_core::Event::Error { .. });
            if matches!(event, maru_core::Event::Done { .. }) { break; }
        }
        assert!(reported);
        assert!(matches!(services.llm_status(), LlmStatus::Failed(_)));
        assert_eq!(services.tts_status(), LlmStatus::Disabled, "translation never requests speech");
        assert!(services.ensure_speech().await.is_err());
        assert!(matches!(services.tts_status(), LlmStatus::Failed(_)));
        drop(services);
        std::fs::remove_dir_all(paths.config_file.parent().unwrap()).ok();
    }

    // The stand-in server is a `#!` script, which Windows cannot launch; Linux CI covers this path.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_language_model_is_started_in_the_background_and_reported_ready() {
        if std::process::Command::new("python3").arg("--version").output().is_err() {
            return;
        }
        let paths = temp_paths("llm");
        paths.ensure().unwrap();
        let script = paths.bin_dir.join("fake-llama-server");
        std::fs::write(
            &script,
            r#"#!/usr/bin/env python3
import sys, time, http.server
args = sys.argv[1:]
port = int(args[args.index("--port") + 1])
time.sleep(0.5)
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200); self.end_headers(); self.wfile.write(b'{"status":"ok"}')
    def log_message(self, *a): pass
http.server.HTTPServer(("127.0.0.1", port), H).serve_forever()
"#,
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let mut cfg = Config::default();
        cfg.llm.port = port;
        cfg.llm.server_path = Some(script);
        cfg.llm.hf_repo = Some("user/model:Q4_K_M".into());

        let mut services = boot(paths.clone(), cfg, BootOptions::default()).await;
        assert_eq!(services.llm_status(), LlmStatus::Starting, "boot returns immediately; the model loads in the background");
        assert_eq!(services.wait_for_llm().await, LlmStatus::Ready);
        assert!(services.llm_server_running());
        drop(services);
        std::fs::remove_dir_all(paths.config_file.parent().unwrap()).ok();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_model_that_cannot_start_reports_failure_with_its_output() {
        let paths = temp_paths("fail");
        let mut cfg = Config::default();
        cfg.llm.server_path = Some(PathBuf::from("/definitely/not/llama-server"));
        cfg.llm.hf_repo = Some("user/model".into());
        let mut services = boot(paths.clone(), cfg, BootOptions::default()).await;
        let LlmStatus::Failed(why) = services.wait_for_llm().await else { panic!("expected failure") };
        assert!(why.contains("llama-server"), "{why}");
        std::fs::remove_dir_all(paths.config_file.parent().unwrap()).ok();
    }
}
