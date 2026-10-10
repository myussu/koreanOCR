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
}

impl Default for BootOptions {
    fn default() -> Self {
        Self { start_llm: true }
    }
}

/// The engine plus everything that must stay alive with it.
pub struct Services {
    pub engine: Engine,
    pub paths: Paths,
    /// Human-readable status lines, one per notable decision or problem.
    pub notes: Vec<String>,
    /// Updates as the model loads.
    pub llm_status: watch::Receiver<LlmStatus>,
    /// Updates as the OCR reading model (PaddleOCR-VL) loads; `Disabled` when it is not in use.
    pub vision_status: watch::Receiver<LlmStatus>,
    /// Keeps `llama-server` alive; it is killed when `Services` is dropped.
    server: Arc<Mutex<Option<LlamaServer>>>,
    /// Same, for the OCR reading model.
    vision_server: Arc<Mutex<Option<LlamaServer>>>,
}

impl Services {
    pub fn llm_status(&self) -> LlmStatus {
        self.llm_status.borrow().clone()
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
    pub fn vision_server_running(&self) -> bool {
        self.vision_server.lock().map(|s| s.is_some()).unwrap_or(false)
    }
    pub fn llm_server_running(&self) -> bool {
        self.server.lock().map(|s| s.is_some()).unwrap_or(false)
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

#[cfg(feature = "paddle")]
fn load_paddle(cfg: &Config, paths: &Paths) -> Result<Arc<dyn OcrEngine>, String> {
    use maru_ocr::paddle::{PaddleConfig, PaddleOcr};
    let pc = PaddleConfig::resolve(&cfg.ocr, paths);
    // ONNX Runtime failing to load can panic inside `ort`; never let that take the app down.
    match std::panic::catch_unwind(|| PaddleOcr::load(&pc)) {
        Ok(Ok(engine)) => Ok(Arc::new(engine)),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("ONNX Runtime could not be loaded (set ORT_DYLIB_PATH, or place onnxruntime.dll next to the program)".into()),
    }
}

#[cfg(not(feature = "paddle"))]
fn load_paddle(_: &Config, _: &Paths) -> Result<Arc<dyn OcrEngine>, String> {
    Err("this build has no PP-OCR support".into())
}

fn choose_ocr(cfg: &Config, paths: &Paths, platform: &Platform, notes: &mut Vec<String>) -> Option<Arc<dyn OcrEngine>> {
    let windows = platform.os_ocr.clone();
    let paddle = match cfg.ocr.backend {
        OcrBackend::Windows => Err("not selected".to_string()),
        _ => load_paddle(cfg, paths),
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
    let mut notes = Vec::new();
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
    let platform = {
        let blocking = tokio::task::spawn_blocking(Platform::native).await;
        blocking.unwrap_or_else(|_| Platform::native())
    };
    tracing::info!("boot: loading OCR");
    let ocr = {
        let (cfg, paths) = (cfg.clone(), paths.clone());
        let platform = platform.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut n = Vec::new();
            let o = choose_ocr(&cfg, &paths, &platform, &mut n);
            (o, n)
        })
        .await;
        match result {
            Ok((o, n)) => {
                notes.extend(n);
                o
            }
            Err(_) => None,
        }
        .map(|o| crate::respace::RespacingOcr::wrap(o, &analyzer))
    };

    // Vision reader: PP-OCR finds the lines, PaddleOCR-VL reads them better. It loads in the
    // background; until it is ready (or if it never is) the gate makes OCR plain PP-OCR.
    let (vision_tx, vision_rx) = watch::channel(LlmStatus::Disabled);
    let vision_server: Arc<Mutex<Option<LlamaServer>>> = Arc::new(Mutex::new(None));
    let vision_ready = Arc::new(AtomicBool::new(false));
    let use_vision = opts.start_llm && cfg.ocr.vision_reader && ocr.is_some() && vision::installed(&paths);
    if cfg.ocr.vision_reader && ocr.is_some() && !vision::installed(&paths) && opts.start_llm {
        notes.push("OCR: PaddleOCR-VL is not installed; run `maru setup ocr-vl` for noticeably more accurate letters.".into());
    }
    let ocr = if use_vision {
        let _ = vision_tx.send(LlmStatus::Starting);
        notes.push("OCR: PaddleOCR-VL reader is loading in the background (PP-OCR alone until it is ready)".into());
        ocr.map(|detector| {
            let url = format!("http://127.0.0.1:{}", vision::PORT);
            let reader = Arc::new(VlmOcr::new(&url, &vision::base_config(&cfg.llm), PADDLE_VL_PROMPT));
            let gated = Arc::new(vision::GatedOcr::new(reader, vision_ready.clone()));
            Arc::new(HybridOcr::new(detector, gated)) as Arc<dyn OcrEngine>
        })
    } else {
        ocr
    };

    // Language model: external endpoint, or our own supervised llama-server.
    let source = ModelSource::from_config(&cfg.llm);
    let (status_tx, status_rx) = watch::channel(LlmStatus::Disabled);
    let server: Arc<Mutex<Option<LlamaServer>>> = Arc::new(Mutex::new(None));
    let llm = if cfg.llm.endpoint.is_some() {
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
    });

    if opts.start_llm && cfg.llm.endpoint.is_none() && source.is_some() && cfg.llm.autostart {
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

    if use_vision {
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

    Services { engine, paths, notes, llm_status: status_rx, vision_status: vision_rx, server, vision_server }
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
