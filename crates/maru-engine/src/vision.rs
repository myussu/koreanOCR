//! The PaddleOCR-VL reading model: where its files live, how to launch `llama-server` for it, and
//! the gate that lets OCR keep working (with PP-OCR alone) while it loads.
//!
//! PaddleOCR-VL's repo ships three files with no quantisation label in their names, so llama.cpp's
//! `-hf repo` lookup cannot be trusted to find them. We download them ourselves (`maru setup
//! ocr-vl`) and start the server the way the model card does: `-m … --mmproj … --temp 0`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use image::RgbImage;
use maru_core::Paths;
use maru_core::config::{LlmConfig, OcrConfig};
use maru_llm::LlamaServer;
use maru_ocr::{OcrEngine, OcrError, OcrOutput, Result};

pub const REPO: &str = "PaddlePaddle/PaddleOCR-VL-1.5-GGUF";
pub const MODEL_FILE: &str = "PaddleOCR-VL-1.5.gguf";
pub const MMPROJ_FILE: &str = "PaddleOCR-VL-1.5-mmproj.gguf";
pub const TEMPLATE_FILE: &str = "chat_template.jinja";
pub const FILES: [&str; 3] = [MODEL_FILE, MMPROJ_FILE, TEMPLATE_FILE];

/// Its own port, so it can run next to the translation model.
pub const PORT: u16 = 18090;
/// Downloads plus load; the first start is the slow one.
pub const READY_TIMEOUT: Duration = Duration::from_secs(900);

pub fn model_dir(paths: &Paths) -> PathBuf {
    paths.models_dir.join("vlm").join("paddleocr-vl-1.5")
}

pub fn installed(paths: &Paths) -> bool {
    let dir = model_dir(paths);
    FILES.iter().all(|f| dir.join(f).metadata().is_ok_and(|m| m.len() > 0))
}

/// The server settings shared by every vision-model launch: its own port, one slot, the
/// projector loaded, no draft model, and not the translation model's weights.
pub fn base_config(llm: &LlmConfig) -> LlmConfig {
    LlmConfig {
        endpoint: None,
        model_path: None,
        draft_model_path: None,
        hf_repo: None,
        vision: true,
        port: PORT,
        parallel: 1,
        ctx_size: 8192,
        extra_args: Vec::new(),
        ..llm.clone()
    }
}

/// The files of a reading model: its weights, its projector and, if it ships one, its chat template.
#[derive(Debug, Clone, PartialEq)]
pub struct ReaderFiles {
    pub model: PathBuf,
    pub mmproj: PathBuf,
    pub template: Option<PathBuf>,
}

/// The reading model in `dir` (`[ocr] reader_dir`): the one `.gguf` with `mmproj` in its name is the projector, the other
/// `.gguf` the model. None unless there is exactly one of each (a folder with two models would be a guess).
pub fn reader_in(dir: &Path) -> Option<ReaderFiles> {
    let mut ggufs: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("gguf")) && p.metadata().is_ok_and(|m| m.len() > 0))
        .collect();
    ggufs.sort();
    let is_mmproj = |p: &PathBuf| p.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase().contains("mmproj"));
    let (mmproj, model): (Vec<_>, Vec<_>) = ggufs.into_iter().partition(is_mmproj);
    let ([mmproj], [model]) = (mmproj.as_slice(), model.as_slice()) else { return None };
    let template = Some(dir.join(TEMPLATE_FILE)).filter(|t| t.metadata().is_ok_and(|m| m.len() > 0));
    Some(ReaderFiles { model: model.clone(), mmproj: mmproj.clone(), template })
}

/// The reading model Maru pairs with PP-OCR: the folder in `[ocr] reader_dir` when one is set, else the installed
/// PaddleOCR-VL-1.5. None when its files are not there.
pub fn reader(paths: &Paths, ocr: &OcrConfig) -> Option<ReaderFiles> {
    match &ocr.reader_dir {
        Some(dir) => reader_in(dir),
        None => installed(paths).then(|| {
            let dir = model_dir(paths);
            ReaderFiles { model: dir.join(MODEL_FILE), mmproj: dir.join(MMPROJ_FILE), template: Some(dir.join(TEMPLATE_FILE)) }
        }),
    }
}

/// How to launch the reading model the settings ask for (see [`reader`]), with its extra arguments; empty if it is not there.
pub fn configured_plans(paths: &Paths, ocr: &OcrConfig, llm: &LlmConfig) -> Vec<LlmConfig> {
    reader(paths, ocr).map(|files| launch_plans_for(&files, llm, &ocr.reader_args)).unwrap_or_default()
}

/// How to launch PaddleOCR-VL, most likely to work first; empty if it is not installed.
pub fn launch_plans(paths: &Paths, llm: &LlmConfig, extra: &[String]) -> Vec<LlmConfig> {
    reader(paths, &OcrConfig::default()).map(|files| launch_plans_for(&files, llm, extra)).unwrap_or_default()
}

/// How to launch a reading model, most likely to work first.
///
/// The second plan adds the chat template shipped with the model, when there is one, and drops flash attention (older
/// llama.cpp builds crashed without the template). `extra` is appended to both.
pub fn launch_plans_for(files: &ReaderFiles, llm: &LlmConfig, extra: &[String]) -> Vec<LlmConfig> {
    let mut base = base_config(llm);
    base.model_path = Some(files.model.clone());
    let with = |args: &[String]| {
        let mut c = base.clone();
        c.extra_args = args.iter().chain(extra).cloned().collect();
        c
    };
    let common = ["--mmproj".to_string(), files.mmproj.display().to_string(), "--temp".into(), "0".into()];
    let mut plans = vec![with(&common)];
    if let Some(template) = &files.template {
        let mut with_template = with(&common);
        with_template.extra_args.extend(["--jinja".to_string(), "--chat-template-file".into(), template.display().to_string()]);
        with_template.flash_attn = false;
        plans.push(with_template);
    }
    plans
}

/// Try each launch plan in turn; the first server that becomes ready wins.
/// `on_attempt(n, total)` runs before each try (for progress output).
pub async fn start(paths: &Paths, plans: Vec<LlmConfig>, on_attempt: impl Fn(usize, usize)) -> Option<(LlamaServer, LlmConfig)> {
    let total = plans.len();
    for (i, cfg) in plans.into_iter().enumerate() {
        on_attempt(i + 1, total);
        match LlamaServer::start(&cfg, paths, READY_TIMEOUT).await {
            Ok(server) => return Some((server, cfg)),
            Err(e) => tracing::warn!("PaddleOCR-VL launch attempt {} of {total} failed: {e}", i + 1),
        }
    }
    None
}

/// An OCR engine that refuses until its server is up, so a hybrid built around it behaves as the
/// plain detector while the model loads (and if it never does) instead of waiting or erroring.
pub struct GatedOcr {
    inner: Arc<dyn OcrEngine>,
    ready: Arc<AtomicBool>,
}

impl GatedOcr {
    pub fn new(inner: Arc<dyn OcrEngine>, ready: Arc<AtomicBool>) -> Self {
        Self { inner, ready }
    }
}

impl OcrEngine for GatedOcr {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn recognize(&self, image: &RgbImage) -> Result<OcrOutput> {
        if !self.ready.load(Ordering::Acquire) {
            return Err(OcrError::Run("the reading model is not ready".into()));
        }
        self.inner.recognize(image)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> Paths {
        let root = std::env::temp_dir().join(format!("maru-vision-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        Paths::under(&root)
    }

    fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
        args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str)
    }

    fn install_fake(paths: &Paths) {
        let dir = model_dir(paths);
        std::fs::create_dir_all(&dir).unwrap();
        for f in FILES {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
    }

    #[test]
    fn nothing_launches_until_every_file_is_there() {
        let paths = scratch("missing");
        assert!(!installed(&paths) && launch_plans(&paths, &LlmConfig::default(), &[]).is_empty());
        let dir = model_dir(&paths);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MODEL_FILE), b"x").unwrap();
        assert!(!installed(&paths), "a lone model file is not an install");
        std::fs::write(dir.join(MMPROJ_FILE), b"x").unwrap();
        std::fs::write(dir.join(TEMPLATE_FILE), b"").unwrap();
        assert!(!installed(&paths), "an empty (failed) download does not count");
    }

    #[test]
    fn it_starts_like_the_model_card_then_retries_with_the_chat_template() {
        let paths = scratch("plans");
        install_fake(&paths);
        let llm = LlmConfig { hf_repo: Some("unsloth/translation-model".into()), port: 18080, parallel: 2, ..Default::default() };
        let plans = launch_plans(&paths, &llm, &["--no-warmup".to_string()]);
        assert_eq!(plans.len(), 2);
        let args = |c: &LlmConfig| maru_llm::build_args(c, &maru_llm::ModelSource::from_config(c).unwrap());

        let first = args(&plans[0]);
        assert!(flag(&first, "-m").unwrap().ends_with(MODEL_FILE), "{first:?}");
        assert!(flag(&first, "--mmproj").unwrap().ends_with(MMPROJ_FILE));
        assert_eq!(flag(&first, "--temp"), Some("0"));
        assert_eq!(flag(&first, "--port"), Some("18090"), "its own port, not the translation model's");
        assert_eq!(flag(&first, "-np"), Some("1"));
        assert!(!first.contains(&"-hf".to_string()), "the translation model's repo must not leak in");
        assert!(first.contains(&"--no-warmup".to_string()), "user arguments are kept");
        assert!(!first.contains(&"--chat-template-file".to_string()));

        let second = args(&plans[1]);
        assert!(flag(&second, "--chat-template-file").unwrap().ends_with(TEMPLATE_FILE));
        assert!(second.contains(&"--jinja".to_string()) && second.contains(&"--no-warmup".to_string()));
        assert_eq!(flag(&second, "-fa"), Some("off"));
    }

    #[test]
    fn another_reading_model_is_found_in_its_folder_and_launched_with_its_own_files() {
        let paths = scratch("other-reader");
        let dir = paths.models_dir.join("vlm").join("other");
        std::fs::create_dir_all(&dir).unwrap();
        let ocr = OcrConfig { reader_dir: Some(dir.clone()), reader_args: vec!["--special".into()], ..Default::default() };
        assert!(reader(&paths, &ocr).is_none() && configured_plans(&paths, &ocr, &LlmConfig::default()).is_empty(), "an empty folder is not a model");
        std::fs::write(dir.join("Reader-Q8_0.gguf"), b"x").unwrap();
        assert!(reader(&paths, &ocr).is_none(), "a model without its projector is not usable");
        std::fs::write(dir.join("mmproj-Reader-Q8_0.gguf"), b"x").unwrap();
        let files = reader(&paths, &ocr).unwrap();
        assert!(files.model.ends_with("Reader-Q8_0.gguf") && files.mmproj.ends_with("mmproj-Reader-Q8_0.gguf") && files.template.is_none());

        let plans = configured_plans(&paths, &ocr, &LlmConfig::default());
        assert_eq!(plans.len(), 1, "no chat template in the folder, so no second way to start it");
        let args = maru_llm::build_args(&plans[0], &maru_llm::ModelSource::from_config(&plans[0]).unwrap());
        assert!(flag(&args, "-m").unwrap().ends_with("Reader-Q8_0.gguf") && flag(&args, "--mmproj").unwrap().ends_with("mmproj-Reader-Q8_0.gguf"));
        assert!(args.contains(&"--special".to_string()), "its extra arguments are passed on");

        std::fs::write(dir.join(TEMPLATE_FILE), b"{{ x }}").unwrap();
        assert_eq!(configured_plans(&paths, &ocr, &LlmConfig::default()).len(), 2);
        std::fs::write(dir.join("Second-Q4.gguf"), b"x").unwrap();
        assert!(reader(&paths, &ocr).is_none(), "two models in one folder: Maru does not guess");
        assert!(reader(&paths, &OcrConfig::default()).is_none(), "and the installed PaddleOCR-VL is still judged by its own files");
    }

    #[test]
    fn the_gate_refuses_until_opened() {
        struct Echo;
        impl OcrEngine for Echo {
            fn name(&self) -> &'static str {
                "echo"
            }
            fn recognize(&self, _: &RgbImage) -> Result<OcrOutput> {
                Ok(OcrOutput::default())
            }
        }
        let ready = Arc::new(AtomicBool::new(false));
        let gated = GatedOcr::new(Arc::new(Echo), ready.clone());
        let img = RgbImage::new(1, 1);
        assert!(gated.recognize(&img).is_err());
        ready.store(true, Ordering::Release);
        assert!(gated.recognize(&img).is_ok());
    }
}
