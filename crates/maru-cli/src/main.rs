//! `maru` — the command-line front end to the same engine the overlay uses.

mod evalreport;
mod render;
mod setup;
mod tools;
mod tts;
mod wizard;

use std::io::{IsTerminal, Read};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use maru_core::{Capture, Config, Event, Paths, Register};
use maru_engine::{BootOptions, DeepDiveRequest, Services, boot};
use render::Renderer;
use setup::fetch_hf;

#[derive(Parser)]
#[command(name = "maru", version, about = "Local Korean reading assistant: translate, break down, learn")]
struct Cli {
    /// Use this folder for config, data and models instead of the platform default.
    #[arg(long, global = true, value_name = "DIR")]
    home: Option<PathBuf>,
    /// Show debug logging.
    #[arg(short, long, global = true)]
    verbose: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Translate text (Korean → English with a word breakdown, or English → Korean). Use `-` to read stdin.
    Translate {
        text: Vec<String>,
        /// Politeness for English → Korean.
        #[arg(long, value_enum)]
        register: Option<RegisterArg>,
        /// Do not use the language model: show only the offline breakdown.
        #[arg(long)]
        no_llm: bool,
        /// Print raw events as JSON lines (what the overlay consumes).
        #[arg(long)]
        json: bool,
    },
    /// Explain one word of a sentence in depth (and record the lookup).
    Explain {
        /// The word as it appears, e.g. 공부했어요
        word: String,
        /// The sentence it came from.
        #[arg(long = "in", value_name = "SENTENCE")]
        sentence: String,
    },
    /// Check what is installed and whether it works.
    Doctor {
        /// Also run a real translation as a probe.
        #[arg(long)]
        deep: bool,
    },
    /// Measure translation latency. It times the language model alone: the speech server and the vision reader are not started.
    Bench {
        /// Text file with one sentence per line (default: a built-in set).
        #[arg(long)]
        file: Option<PathBuf>,
        #[arg(short = 'n', long, default_value_t = 3)]
        runs: usize,
        /// Time the live-subtitle lane (translation only, earlier lines as context) instead of the full translation.
        #[arg(long)]
        live: bool,
        /// Use the model served at this address (e.g. `http://192.168.1.20:8080`) for this run, instead of the one in the config.
        #[arg(long, value_name = "URL")]
        endpoint: Option<String>,
        /// The model name that server expects (Ollama chooses by name: what `ollama list` shows). Only with a server, not llama-server.
        #[arg(long, value_name = "NAME")]
        model: Option<String>,
    },
    /// Read a screenshot the way the screen scan does: list every Korean text found and how it is labelled.
    Scan {
        image: PathBuf,
        /// Also write a copy of the image with the found text outlined (green = labelled, blue = dictionary only, orange = no label).
        #[arg(long, value_name = "FILE")]
        boxes: Option<PathBuf>,
        /// Do not start the language model: dictionary labels only.
        #[arg(long)]
        no_llm: bool,
    },
    /// Score OCR engines on your own screenshots (name.png + name.txt pairs).
    EvalOcr {
        dir: PathBuf,
        /// Engines to compare.
        #[arg(long, value_enum, num_args = 1.., default_values_t = [EngineArg::Paddle, EngineArg::Windows])]
        engine: Vec<EngineArg>,
        /// Hugging Face repo for `gemma` (default: the model set in your config, else the recommended one).
        #[arg(long)]
        gemma_repo: Option<String>,
        /// Extra llama-server arguments for the vision models, as one string (e.g. "--jinja --chat-template-file x.jinja").
        #[arg(long)]
        vlm_args: Option<String>,
        /// Use a vision model you started yourself (OpenAI-compatible, e.g. `http://127.0.0.1:8080`) as engine `endpoint`.
        #[arg(long)]
        vlm_endpoint: Option<String>,
        /// Instruction sent to the `endpoint` engine (default: the PaddleOCR-VL `OCR:` prompt).
        #[arg(long)]
        vlm_prompt: Option<String>,
        /// Ask PP-OCR to enlarge each picture this many times first, as live subtitles do (`[continuous] ocr_scale`).
        #[arg(long, default_value_t = 1.0)]
        scale: f32,
        /// Do not score: read every picture that has no label yet (in the folder and the ones below it) and save what each engine
        /// read beside it as `name.<engine>.txt`, for `tools/label_review.html` to show as a second opinion.
        #[arg(long)]
        write_drafts: bool,
        /// Score only one half of the test set. The halves are fixed (a whole scene is always on one side), so tune on `dev` and
        /// judge on `held-out`.
        #[arg(long, value_enum, default_value_t = SplitArg::All)]
        split: SplitArg,
        /// Save every picture's result to this file, to compare a later run with it (`--compare`).
        #[arg(long)]
        save_report: Option<PathBuf>,
        /// Compare this run, subtitle by subtitle, with a report saved earlier by `--save-report` (before and after a change).
        #[arg(long)]
        compare: Option<PathBuf>,
    },
    /// Speak text with the voice from your settings (sentence by sentence) into a WAV file.
    Tts {
        text: Vec<String>,
        /// Where to write it.
        #[arg(short, long, default_value = "speech.wav")]
        out: PathBuf,
    },
    /// Listen to voices: speaks sentences that show sound changes, numbers and intonation, and writes a page to play them.
    TtsEval {
        /// Folder for the audio and index.html (default: inside Maru's data folder).
        #[arg(long)]
        out: Option<PathBuf>,
        /// Voice name to compare; repeat for several (default: the one in your settings).
        #[arg(long)]
        voice: Vec<String>,
        /// Use a speech server at this address instead of your settings (any OpenAI-style one).
        #[arg(long)]
        endpoint: Option<String>,
        #[arg(long)]
        api_key: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        language: Option<String>,
        #[arg(long)]
        instructions: Option<String>,
        /// Your own sentences, one per line.
        #[arg(long, value_name = "FILE")]
        sentences: Option<PathBuf>,
    },
    /// Install what Maru needs: with no subcommand, a wizard that picks it for this PC.
    Setup {
        #[command(subcommand)]
        cmd: Option<SetupCommand>,
    },
    /// Offline dictionary.
    #[command(subcommand)]
    Dict(DictCommand),
    /// Your vocabulary and reviews.
    #[command(subcommand)]
    Vocab(VocabCommand),
    /// Configuration file.
    #[command(subcommand)]
    Config(ConfigCommand),
}

#[derive(Subcommand)]
enum SetupCommand {
    /// Look at this PC, recommend what to install, and install it (this is what plain `maru setup` runs).
    Wizard(wizard::WizardArgs),
    /// Korean morphological analyser (accurate grammar breakdown).
    Kiwi {
        /// A specific Kiwi release, e.g. v0.22.2 (default: the newest with files for this platform).
        #[arg(long)]
        release: Option<String>,
    },
    /// PP-OCR Korean models for reading text from pixels.
    Ocr {
        #[arg(long, value_name = "DIR")]
        from_dir: Option<PathBuf>,
        #[arg(long)]
        base_url: Option<String>,
        #[arg(long)]
        det_url: Option<String>,
        #[arg(long)]
        rec_url: Option<String>,
        #[arg(long)]
        dict_url: Option<String>,
    },
    /// PaddleOCR-VL, the reading model that makes OCR text noticeably more accurate (about 1.8 GB).
    OcrVl,
    /// Text to speech: the local voice server and the Qwen3-TTS model (about 2.5 GB). Switches speech on in your settings.
    Tts {
        /// A smaller, slightly less natural model (about 1 GB).
        #[arg(long)]
        small: bool,
        /// Where to download tts-server from (a zip), instead of this repository's latest build.
        #[arg(long)]
        server_url: Option<String>,
        /// Do not start the server to try it afterwards.
        #[arg(long)]
        no_test: bool,
    },
    /// llama.cpp server, and the model configured in [llm].
    Llm {
        #[arg(long, value_enum, default_value = "cuda")]
        backend: setup::LlamaBackend,
        /// Pin a CUDA build, e.g. 12.4 or 13.1.
        #[arg(long)]
        cuda: Option<String>,
        /// Download llama-server again even if one already works (to switch from CUDA to Vulkan or CPU).
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum DictCommand {
    /// Download the Wiktionary Korean extract and import it.
    Download {
        #[arg(long)]
        url: Option<String>,
    },
    /// Import a Wiktionary JSON-lines file you already have.
    Import { kaikki: PathBuf },
    /// Import a `lemma<TAB>level` list (TOPIK 1-6, or 초급/중급/고급).
    Levels { file: PathBuf },
    /// Show what is in the dictionary.
    Stats,
}

#[derive(Subcommand)]
enum VocabCommand {
    /// Recently looked-up words.
    Recent {
        #[arg(short, long, default_value_t = 20)]
        limit: u32,
    },
    /// Words due for review now.
    Due {
        #[arg(short, long, default_value_t = 20)]
        limit: u32,
    },
    /// Record a review: rating 1 (again) to 4 (easy).
    Review { word: String, rating: u8 },
    /// Mark a word as known (glosses fade) or `--not` to undo.
    Known {
        word: String,
        #[arg(long = "not")]
        not_known: bool,
    },
    /// Mark every dictionary word up to this TOPIK level as known.
    KnownLevel { level: u8 },
    /// Add a word to the review queue.
    Save { word: String },
    /// Export saved words as a tab-separated file for Anki's importer.
    Export { path: PathBuf },
    Stats,
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Print where the config file is.
    Path,
    /// Write a config file with every default spelled out.
    Init {
        #[arg(long)]
        force: bool,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum RegisterArg {
    Polite,
    Formal,
    Casual,
}

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum SplitArg {
    /// Every picture.
    All,
    /// The half to tune on.
    Dev,
    /// The half to judge on.
    HeldOut,
}

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum EngineArg {
    /// PP-OCRv5 (detector + Korean recogniser), the default.
    Paddle,
    /// Windows.Media.Ocr.
    Windows,
    /// PaddleOCR-VL-1.5 through its own llama-server.
    PaddleVl,
    /// PP-OCR finds the lines, PaddleOCR-VL reads the whole image, and the two are matched by text (needs both).
    Hybrid,
    /// A general vision model (Gemma 4) through its own llama-server.
    Gemma,
    /// A vision model you already run; needs --vlm-endpoint.
    Endpoint,
}

/// PP-OCR with the spacing repair the overlay applies, so scores reflect real use.
fn load_pp_ocr(s: &Services, paths: &Paths) -> Option<(String, Arc<dyn maru_ocr::OcrEngine>)> {
    let pc = maru_ocr::paddle::PaddleConfig::resolve(&s.engine.config().ocr, paths);
    match maru_ocr::paddle::PaddleOcr::load(&pc) {
        Ok(o) => {
            let o: Arc<dyn maru_ocr::OcrEngine> = Arc::new(o);
            let o = maru_engine::respace::RespacingOcr::wrap(o, &s.engine.analyzer());
            Some((format!("PP-OCR ({})", pc.device), o))
        }
        Err(err) => {
            eprintln!("(PP-OCR unavailable: {err}; skipping)\n");
            None
        }
    }
}

/// Download PaddleOCR-VL's files (skipping any already there).
async fn fetch_paddle_vl(paths: &Paths) -> Result<()> {
    let dir = maru_engine::vision::model_dir(paths);
    for file in maru_engine::vision::FILES {
        fetch_hf(maru_engine::vision::REPO, file, &dir).await.with_context(|| format!("downloading {file}"))?;
    }
    Ok(())
}

fn with_args(base: &maru_core::config::LlmConfig, args: &[String], extra: &[String]) -> maru_core::config::LlmConfig {
    let mut c = base.clone();
    c.extra_args = args.iter().chain(extra).cloned().collect();
    c
}

/// How to launch PaddleOCR-VL (downloading it first if needed): see [`maru_engine::vision`].
async fn paddle_vl_attempts(paths: &Paths, llm: &maru_core::config::LlmConfig, extra: &[String]) -> Vec<maru_core::config::LlmConfig> {
    if let Err(e) = fetch_paddle_vl(paths).await {
        eprintln!("(could not download PaddleOCR-VL: {e:#})");
        return Vec::new();
    }
    maru_engine::vision::launch_plans(paths, llm, extra)
}

/// How to launch the general vision model (Gemma 4 by default).
///
/// The F16 projector is pinned explicitly: a llama.cpp discussion reports the BF16 one failing on
/// some GPUs. If the repo has no such file, llama.cpp is left to choose its own projector.
async fn gemma_attempts(paths: &Paths, repo: &str, base: &maru_core::config::LlmConfig, extra: &[String]) -> Vec<maru_core::config::LlmConfig> {
    let bare = repo.split(':').next().unwrap_or(repo);
    let dir = paths.models_dir.join("vlm").join("gemma");
    let mut base = base.clone();
    base.hf_repo = Some(repo.to_string());
    let auto = with_args(&base, &[], extra);
    match fetch_hf(bare, "mmproj-F16.gguf", &dir).await {
        Ok(mmproj) => vec![with_args(&base, &["--mmproj".to_string(), mmproj.display().to_string()], extra), auto],
        Err(e) => {
            eprintln!("(no explicit F16 projector for {bare}: {e:#}; letting llama.cpp choose)");
            vec![auto]
        }
    }
}

/// Try each launch plan in turn; the first server that becomes ready wins.
async fn start_vision_server(
    label: &str,
    attempts: Vec<maru_core::config::LlmConfig>,
    paths: &Paths,
    timeout: std::time::Duration,
) -> Option<(maru_llm::LlamaServer, maru_core::config::LlmConfig)> {
    let total = attempts.len();
    for (i, cfg) in attempts.into_iter().enumerate() {
        println!("Starting {label} (attempt {} of {total}); loading the model …", i + 1);
        match maru_llm::LlamaServer::start(&cfg, paths, timeout).await {
            Ok(server) => return Some((server, cfg)),
            Err(e) => eprintln!("  attempt {} failed: {e}", i + 1),
        }
    }
    None
}

fn paths_for(cli: &Cli) -> Paths {
    match &cli.home {
        Some(h) => Paths::under(h),
        None => Paths::discover(),
    }
}

fn read_text(parts: &[String]) -> Result<String> {
    if parts.len() == 1 && parts[0] == "-" {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        return Ok(s);
    }
    if parts.is_empty() {
        bail!("give some text, or `-` to read it from stdin");
    }
    Ok(parts.join(" "))
}

async fn services(paths: &Paths, cfg: Config, start_llm: bool) -> Services {
    boot(paths.clone(), cfg, BootOptions { start_llm }).await
}

async fn run_events(mut rx: maru_engine::EventRx, json: bool) -> usize {
    let mut r = Renderer::new(std::io::stdout(), std::io::stdout().is_terminal());
    while let Some(e) = rx.recv().await {
        if json {
            println!("{}", serde_json::to_string(&e).unwrap_or_default());
        } else {
            r.event(&e);
        }
        if matches!(e, Event::Done { .. }) {
            break;
        }
    }
    r.errors.len()
}

fn main() -> Result<()> {
    maru_engine::bootstrap::run_probe_if_requested();
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| if cli.verbose { "info,maru_engine=debug".into() } else { "warn".into() }))
        .with_writer(std::io::stderr)
        .init();
    let paths = paths_for(&cli);

    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let code = runtime.block_on(run(cli, paths))?;
    std::process::exit(code);
}

async fn run(cli: Cli, paths: Paths) -> Result<i32> {
    let mut cfg = Config::load(&paths.config_file).with_context(|| format!("reading {}", paths.config_file.display()))?;
    match cli.command {
        Command::Translate { text, register, no_llm, json } => {
            let text = read_text(&text)?;
            if no_llm {
                cfg.llm.endpoint = None;
                cfg.llm.model_path = None;
                cfg.llm.hf_repo = None;
            }
            if let Some(r) = register {
                cfg.learning.register = match r {
                    RegisterArg::Polite => Register::Polite,
                    RegisterArg::Formal => Register::Formal,
                    RegisterArg::Casual => Register::Casual,
                };
            }
            let s = services(&paths, cfg, true).await;
            let (_, rx) = s.engine.translate(Capture::typed(text));
            let errors = run_events(rx, json).await;
            Ok(i32::from(errors > 0 && !no_llm))
        }
        Command::Explain { word, sentence } => {
            let s = services(&paths, cfg, true).await;
            let eojeol = {
                let a = s.engine.analyze(&sentence);
                a.into_iter().find(|e| e.surface.contains(&word) || e.lemma.as_deref() == Some(word.as_str()))
            };
            let Some(eojeol) = eojeol else { bail!("`{word}` is not a word of that sentence") };
            let (_, rx) = s.engine.deep_dive(DeepDiveRequest { eojeol, sentence, translation: None, meaning: None, app: None });
            Ok(i32::from(run_events(rx, false).await > 0))
        }
        Command::Doctor { deep } => {
            eprintln!("Starting up: loading Kiwi, OCR and the dictionary (add --verbose to see each step) …");
            let mut s = services(&paths, cfg, true).await;
            tools::doctor(&mut s, deep).await?;
            Ok(0)
        }
        Command::Bench { file, runs, live, endpoint, model } => {
            let sentences: Vec<String> = match file {
                Some(f) => std::fs::read_to_string(&f)?.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect(),
                // Subtitles are Korean: the live lane has nothing to do with the English sentences.
                None if live => tools::KO_SENTENCES.iter().map(|s| s.to_string()).collect(),
                None => tools::KO_SENTENCES.iter().chain(tools::EN_SENTENCES).map(|s| s.to_string()).collect(),
            };
            if sentences.is_empty() {
                bail!("no sentences to run");
            }
            // This times the language model, so nothing else may compete for the GPU while it runs.
            cfg.ocr.vision_reader = false;
            cfg.tts.enabled = false;
            if let Some(url) = endpoint {
                cfg.llm.endpoint = Some(url);
            }
            if let Some(name) = model {
                cfg.llm.model_name = name;
            }
            let mut s = services(&paths, cfg, true).await;
            if live {
                tools::bench_live(&mut s, sentences, runs.max(1)).await?;
            } else {
                tools::bench(&mut s, sentences, runs.max(1)).await?;
            }
            Ok(0)
        }
        Command::Scan { image, boxes, no_llm } => {
            let picture = image::open(&image).with_context(|| format!("opening {}", image.display()))?.to_rgb8();
            let mut s = services(&paths, cfg, !no_llm).await;
            if !no_llm && s.engine.has_llm() {
                eprintln!("Waiting for the language model …");
                s.wait_for_llm().await;
            }
            let report = tools::collect_scan(s.engine.scan_image(picture.clone()).1).await;
            print!("{}", tools::format_scan(&report));
            if let Some(path) = boxes {
                tools::draw_scan(&picture, &report.items).save(&path).with_context(|| format!("writing {}", path.display()))?;
                println!("\nOutlines written to {}", path.display());
            }
            // Nothing read *because* something is wrong is a failure; an honestly empty screen is not.
            Ok(if report.items.is_empty() && !report.errors.is_empty() { 1 } else { 0 })
        }
        Command::EvalOcr { dir, engine, gemma_repo, vlm_args, vlm_endpoint, vlm_prompt, scale, write_drafts, split, save_report, compare } => {
            use maru_engine::vlm_ocr::{GENERAL_VLM_PROMPT, PADDLE_VL_PROMPT, SERVER_READY, VlmOcr};
            let s = services(&paths, cfg, false).await;
            let split = match split {
                SplitArg::All => None,
                SplitArg::Dev => Some(evalreport::Split::Dev),
                SplitArg::HeldOut => Some(evalreport::Split::Held),
            };
            let (cases, images) = if write_drafts { (Vec::new(), tools::load_images(&dir)?) } else { (tools::load_case_infos(&dir, split)?, Vec::new()) };
            println!("{} test image(s)\n", if write_drafts { images.len() } else { cases.len() });
            let mut scores = Vec::new();
            let extra: Vec<String> = vlm_args.as_deref().unwrap_or("").split_whitespace().map(str::to_string).collect();
            // Vision-model servers take turns, so they never compete for VRAM (or for timing).
            let vlm_cfg = {
                let mut c = s.engine.config().llm.clone();
                c.endpoint = None;
                c.model_path = None;
                c.draft_model_path = None;
                c.hf_repo = None;
                c.vision = true;
                c.port = 18090;
                c.parallel = 1;
                c.ctx_size = 8192;
                c.extra_args = Vec::new();
                c
            };
            for e in engine {
                let (name, ocr, _server): (String, Arc<dyn maru_ocr::OcrEngine>, Option<maru_llm::LlamaServer>) = match e {
                    EngineArg::Windows => match s.engine.platform().os_ocr.clone() {
                        Some(o) => ("Windows OCR".into(), o, None),
                        None => {
                            eprintln!("(Windows OCR is not available here; skipping)\n");
                            continue;
                        }
                    },
                    EngineArg::Paddle => match load_pp_ocr(&s, &paths) {
                        Some(pair) => (pair.0, pair.1, None),
                        None => continue,
                    },
                    EngineArg::Hybrid => {
                        let Some((_, detector)) = load_pp_ocr(&s, &paths) else { continue };
                        println!("Preparing PaddleOCR-VL-1.5 for the hybrid engine (the first run downloads the model) …");
                        let attempts = paddle_vl_attempts(&paths, &s.engine.config().llm, &extra).await;
                        match start_vision_server("PaddleOCR-VL-1.5", attempts, &paths, SERVER_READY).await {
                            Some((server, cfg)) => {
                                let reader: Arc<dyn maru_ocr::OcrEngine> = Arc::new(VlmOcr::new(&format!("http://127.0.0.1:{}", cfg.port), &cfg, PADDLE_VL_PROMPT));
                                let hybrid = maru_engine::hybrid_ocr::HybridOcr::new(detector, reader);
                                ("PP-OCR + PaddleOCR-VL (hybrid)".to_string(), Arc::new(hybrid) as Arc<dyn maru_ocr::OcrEngine>, Some(server))
                            }
                            None => {
                                eprintln!("(PaddleOCR-VL could not start; skipping the hybrid engine)\n");
                                continue;
                            }
                        }
                    }
                    EngineArg::PaddleVl | EngineArg::Gemma => {
                        let label = if e == EngineArg::PaddleVl { "PaddleOCR-VL-1.5" } else { "Gemma 4 (vision)" };
                        println!("Preparing {label} (the first run downloads the model; this can take a while) …");
                        let (attempts, prompt) = match e {
                            EngineArg::PaddleVl => (paddle_vl_attempts(&paths, &s.engine.config().llm, &extra).await, PADDLE_VL_PROMPT),
                            _ => {
                                let repo = gemma_repo
                                    .clone()
                                    .or_else(|| s.engine.config().llm.hf_repo.clone())
                                    .unwrap_or_else(|| setup::RECOMMENDED_MODEL.to_string());
                                (gemma_attempts(&paths, &repo, &vlm_cfg, &extra).await, GENERAL_VLM_PROMPT)
                            }
                        };
                        match start_vision_server(label, attempts, &paths, SERVER_READY).await {
                            Some((server, cfg)) => {
                                let ocr: Arc<dyn maru_ocr::OcrEngine> = Arc::new(VlmOcr::new(&format!("http://127.0.0.1:{}", cfg.port), &cfg, prompt));
                                (label.to_string(), ocr, Some(server))
                            }
                            None => {
                                eprintln!("({label} could not start; skipping)\n  Tip: start it yourself and use --engine endpoint --vlm-endpoint <url>; see `maru eval-ocr --help`.\n");
                                continue;
                            }
                        }
                    }
                    EngineArg::Endpoint => {
                        let Some(url) = vlm_endpoint.clone() else {
                            eprintln!("(--engine endpoint needs --vlm-endpoint <url>; skipping)\n");
                            continue;
                        };
                        let mut cfg = s.engine.config().llm.clone();
                        cfg.endpoint = Some(url.clone());
                        let prompt = vlm_prompt.clone().unwrap_or_else(|| PADDLE_VL_PROMPT.to_string());
                        ("Vision model (endpoint)".into(), Arc::new(VlmOcr::new(&url, &cfg, &prompt)) as Arc<dyn maru_ocr::OcrEngine>, None)
                    }
                };
                if write_drafts {
                    // clap names the engine the way the command line spells it (`paddle-vl`), which is what the review page lists it as.
                    let tag = clap::ValueEnum::to_possible_value(&e).map(|v| v.get_name().to_string()).unwrap_or_else(|| "draft".into());
                    println!("{name}: reading the pictures that have no label yet …");
                    let (tag2, images) = (tag.clone(), images.clone());
                    match tokio::task::spawn_blocking(move || tools::write_drafts(&tag2, ocr.as_ref(), &images, scale)).await? {
                        Ok(n) => println!("  wrote {n} draft(s) as name.{tag}.txt\n"),
                        Err(err) => eprintln!("({name} failed: {err:#})\n"),
                    }
                    continue;
                }
                // Recognition is blocking (and for vision models waits on HTTP): keep the runtime free.
                let (task_name, cases) = (name.clone(), cases.clone());
                let result = tokio::task::spawn_blocking(move || tools::eval_engine(&task_name, ocr.as_ref(), &cases, scale)).await?;
                match result {
                    Ok(score) => scores.push(score),
                    Err(err) => eprintln!("({name} failed: {err:#})\n"),
                }
            }
            if write_drafts {
                return Ok(0);
            }
            if scores.is_empty() {
                bail!("no OCR engine could be evaluated; run `maru setup ocr` first");
            }
            tools::print_scores(&scores);
            println!();
            // Several engines in one run: each later one against the first, subtitle by subtitle.
            for later in scores.iter().skip(1) {
                evalreport::print_comparison(&scores[0].name, &later.name, &evalreport::compare(&scores[0].results, &later.results));
            }
            if let Some(path) = compare {
                let saved: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?)?;
                let before: Vec<(String, Vec<evalreport::CaseResult>)> = saved["engines"].as_array().map(|a| a.iter().filter_map(evalreport::from_json).collect()).unwrap_or_default();
                if before.is_empty() {
                    bail!("{} is not a report saved by --save-report", path.display());
                }
                for now in &scores {
                    // The same engine if the file has it, else its first one.
                    let (name, results) = before.iter().find(|(n, _)| *n == now.name).unwrap_or(&before[0]);
                    evalreport::print_comparison(&format!("{name} (saved)"), &now.name, &evalreport::compare(results, &now.results));
                }
            }
            if let Some(path) = save_report {
                let report = serde_json::json!({ "engines": scores.iter().map(|s| evalreport::to_json(&s.name, s.scale, &s.results)).collect::<Vec<_>>() });
                std::fs::write(&path, serde_json::to_string_pretty(&report)?).with_context(|| format!("writing {}", path.display()))?;
                println!("Saved this run's results to {}. Next time: --compare {}", path.display(), path.display());
            }
            Ok(0)
        }
        Command::Tts { text, out } => {
            tts::speak(&paths, &cfg, &read_text(&text)?, &out).await?;
            Ok(0)
        }
        Command::TtsEval { out, voice, endpoint, api_key, model, language, instructions, sentences } => {
            tts::eval(&paths, &cfg, tts::EvalOptions { out, voices: voice, endpoint, api_key, model, language, instructions, sentences_file: sentences }).await?;
            Ok(0)
        }
        Command::Setup { cmd } => {
            match cmd.unwrap_or(SetupCommand::Wizard(wizard::WizardArgs::default())) {
                SetupCommand::Wizard(args) => return wizard::run(&paths, args).await,
                SetupCommand::Kiwi { release } => setup::kiwi(&paths, release).await?,
                SetupCommand::Ocr { from_dir, base_url, det_url, rec_url, dict_url } => {
                    setup::ocr(&paths, &cfg, setup::OcrSources { from_dir, base_url, det_url, rec_url, dict_url }).await?
                }
                SetupCommand::OcrVl => {
                    fetch_paddle_vl(&paths).await?;
                    println!("PaddleOCR-VL is installed. Maru starts it automatically next to the translation model (OCR uses it once it has loaded).");
                    println!("Compare it on your own screenshots: maru eval-ocr <folder> --engine paddle hybrid");
                }
                SetupCommand::Tts { small, server_url, no_test } => tts::setup(&paths, &cfg, tts::SetupOptions { small, server_url, no_test }).await?,
                // With `lite` on, the model to fetch is the small one that will actually run.
                SetupCommand::Llm { backend, cuda, force } => setup::llm(&paths, &cfg.clone().effective().0, backend, cuda, force).await?,
            }
            Ok(0)
        }
        Command::Dict(cmd) => {
            paths.ensure()?;
            let dict = maru_dict::Dictionary::open(&paths.dict_db())?;
            match cmd {
                DictCommand::Download { url } => download_dictionary(&paths, &dict, url).await?,
                DictCommand::Import { kaikki } => {
                    let stats = import_jsonl(&dict, &kaikki)?;
                    println!("Imported {} entries ({} inflected forms; {} lines skipped).", stats.entries, stats.forms, stats.skipped);
                }
                DictCommand::Levels { file } => {
                    let n = dict.import_levels(std::io::BufReader::new(std::fs::File::open(&file)?))?;
                    println!("Imported levels for {n} words.");
                }
                DictCommand::Stats => println!("{} dictionary entries in {}", dict.entry_count()?, paths.dict_db().display()),
            }
            Ok(0)
        }
        Command::Vocab(cmd) => {
            let s = services(&paths, cfg, false).await;
            let e = &s.engine;
            match cmd {
                VocabCommand::Recent { limit } => {
                    for w in e.recent_words(limit)? {
                        println!("{:<12} {:?}  looked up {}×  {}", w.key, w.state, w.lookups, w.meaning.unwrap_or_default());
                    }
                }
                VocabCommand::Due { limit } => {
                    for w in e.due_words(limit)? {
                        println!("{:<12} {:?}  {}", w.key, w.state, w.meaning.unwrap_or_default());
                    }
                }
                VocabCommand::Review { word, rating } => {
                    let r = e.review_word(&word, rating)?;
                    println!("{} → next review in {} day(s)", word, r.interval_days);
                }
                VocabCommand::Known { word, not_known } => println!("{:?}", e.set_known(&word, !not_known)?.state),
                VocabCommand::KnownLevel { level } => println!("Marked {} words as known.", e.mark_known_up_to_level(level)?),
                VocabCommand::Save { word } => println!("{:?}", e.save_word(&word)?),
                VocabCommand::Export { path } => println!("Exported {} card(s) to {}", e.export_tsv(&path)?, path.display()),
                VocabCommand::Stats => println!("{:#?}", e.vocab_stats()?),
            }
            Ok(0)
        }
        Command::Config(cmd) => {
            match cmd {
                ConfigCommand::Path => println!("{}", paths.config_file.display()),
                ConfigCommand::Init { force } => {
                    if paths.config_file.exists() && !force {
                        bail!("{} already exists (use --force to overwrite)", paths.config_file.display());
                    }
                    let mut cfg = Config::default();
                    cfg.llm.hf_repo = Some(setup::RECOMMENDED_MODEL.to_string());
                    cfg.save(&paths.config_file)?;
                    println!("Wrote {}\nLanguage model set to {} (edit [llm] to change it).", paths.config_file.display(), setup::RECOMMENDED_MODEL);
                }
            }
            Ok(0)
        }
    }
}

/// Fetch the Wiktionary Korean extract and import it.
async fn download_dictionary(paths: &Paths, dict: &maru_dict::Dictionary, url: Option<String>) -> Result<()> {
    let url = url.unwrap_or_else(|| setup::KAIKKI_KOREAN_URL.to_string());
    let tmp = paths.data_dir.join("kaikki-korean.jsonl");
    setup::download(&url, &tmp, "Korean dictionary").await?;
    let stats = import_jsonl(dict, &tmp)?;
    std::fs::remove_file(&tmp).ok();
    println!("Imported {} entries ({} inflected forms; {} lines skipped).", stats.entries, stats.forms, stats.skipped);
    Ok(())
}

fn import_jsonl(dict: &maru_dict::Dictionary, path: &std::path::Path) -> Result<maru_dict::ImportStats> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let total = file.metadata().map(|m| m.len()).unwrap_or(0);
    eprintln!("Importing {} ({:.0} MB) …", path.display(), total as f64 / 1_048_576.0);
    Ok(dict.import_kaikki(std::io::BufReader::new(file), |n| eprint!("\r  {n} entries   "))?)
}

#[cfg(test)]
mod cli_tests {
    use clap::{CommandFactory, Parser};

    use super::*;

    #[test]
    fn the_command_line_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn bench_can_time_the_live_lane_against_another_pc() {
        let b = Cli::try_parse_from(["maru", "bench", "--live", "--endpoint", "http://192.168.1.20:8080", "-n", "2"]).unwrap();
        assert!(matches!(b.command, Command::Bench { live: true, runs: 2, endpoint: Some(ref e), .. } if e == "http://192.168.1.20:8080"));
        let o = Cli::try_parse_from(["maru", "bench", "--live", "--endpoint", "http://nas:11434", "--model", "hf.co/x/y:Q4"]).unwrap();
        assert!(matches!(o.command, Command::Bench { model: Some(ref m), .. } if m == "hf.co/x/y:Q4"));
        assert!(matches!(Cli::try_parse_from(["maru", "bench"]).unwrap().command, Command::Bench { live: false, endpoint: None, .. }));
    }

    #[test]
    fn plain_setup_is_the_wizard_and_the_installers_are_still_there() {
        let wizard = Cli::try_parse_from(["maru", "setup"]).unwrap();
        assert!(matches!(wizard.command, Command::Setup { cmd: None }));
        let yes = Cli::try_parse_from(["maru", "setup", "wizard", "--yes", "--backend", "vulkan", "--force"]).unwrap();
        assert!(matches!(yes.command, Command::Setup { cmd: Some(SetupCommand::Wizard(ref a)) } if a.yes && a.force && a.backend == Some(setup::LlamaBackend::Vulkan)));
        for args in [["maru", "setup", "llm"].as_slice(), &["maru", "setup", "kiwi"], &["maru", "setup", "ocr"], &["maru", "setup", "ocr-vl"], &["maru", "setup", "tts"]] {
            assert!(Cli::try_parse_from(args).is_ok(), "{args:?}");
        }
    }
}

#[cfg(test)]
mod vision_launch_tests {
    use super::*;

    /// Pre-seeded model files make `fetch_hf` skip the network, so the launch plans can be checked offline.
    fn seeded(dir: &std::path::Path, files: &[&str]) {
        std::fs::create_dir_all(dir).unwrap();
        for f in files {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
    }

    fn scratch(name: &str) -> Paths {
        let root = std::env::temp_dir().join(format!("maru-vlm-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        Paths::under(&root)
    }

    fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
        args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str)
    }

    #[tokio::test]
    async fn gemma_pins_the_f16_projector_and_keeps_the_quantisation_in_the_repo_spec() {
        let paths = scratch("gemma");
        seeded(&paths.models_dir.join("vlm").join("gemma"), &["mmproj-F16.gguf"]);
        let base = maru_core::config::LlmConfig { vision: true, ..Default::default() };
        let plans = gemma_attempts(&paths, "unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL", &base, &[]).await;
        assert_eq!(plans.len(), 2, "second plan lets llama.cpp choose its own projector");

        let pinned = maru_llm::build_args(&plans[0], &maru_llm::ModelSource::from_config(&plans[0]).unwrap());
        assert_eq!(flag(&pinned, "-hf"), Some("unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL"));
        assert!(flag(&pinned, "--mmproj").unwrap().ends_with("mmproj-F16.gguf"));
        assert!(!pinned.contains(&"--no-mmproj".to_string()), "vision needs its projector");

        let auto = maru_llm::build_args(&plans[1], &maru_llm::ModelSource::from_config(&plans[1]).unwrap());
        assert!(flag(&auto, "--mmproj").is_none() && !auto.contains(&"--no-mmproj".to_string()));
    }
}
