//! `maru` — the command-line front end to the same engine the overlay uses.

mod render;
mod setup;
mod tools;

use std::io::{IsTerminal, Read};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use maru_core::{Capture, Config, Event, Paths, Register};
use maru_engine::{BootOptions, DeepDiveRequest, Services, boot};
use render::Renderer;

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
    /// Measure translation latency.
    Bench {
        /// Text file with one sentence per line (default: a built-in set).
        #[arg(long)]
        file: Option<PathBuf>,
        #[arg(short = 'n', long, default_value_t = 3)]
        runs: usize,
    },
    /// Score OCR engines on your own screenshots (name.png + name.txt pairs).
    EvalOcr {
        dir: PathBuf,
        /// Engines to compare.
        #[arg(long, value_enum, num_args = 1.., default_values_t = [EngineArg::Paddle, EngineArg::Windows])]
        engine: Vec<EngineArg>,
        /// Hugging Face repo for `paddle-vl` (llama-server `-hf` syntax, e.g. `repo:Q8_0`).
        #[arg(long, default_value = "PaddlePaddle/PaddleOCR-VL-1.5-GGUF")]
        paddle_vl_repo: String,
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
    },
    /// Install things Maru does not bundle.
    #[command(subcommand)]
    Setup(SetupCommand),
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
    /// llama.cpp server, and the model configured in [llm].
    Llm {
        #[arg(long, value_enum, default_value = "cuda")]
        backend: setup::LlamaBackend,
        /// Pin a CUDA build, e.g. 12.4 or 13.1.
        #[arg(long)]
        cuda: Option<String>,
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
enum EngineArg {
    /// PP-OCRv5 (detector + Korean recogniser), the default.
    Paddle,
    /// Windows.Media.Ocr.
    Windows,
    /// PaddleOCR-VL-1.5 through its own llama-server.
    PaddleVl,
    /// A general vision model (Gemma 4) through its own llama-server.
    Gemma,
    /// A vision model you already run; needs --vlm-endpoint.
    Endpoint,
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
        Command::Bench { file, runs } => {
            let sentences: Vec<String> = match file {
                Some(f) => std::fs::read_to_string(&f)?.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect(),
                None => tools::KO_SENTENCES.iter().chain(tools::EN_SENTENCES).map(|s| s.to_string()).collect(),
            };
            if sentences.is_empty() {
                bail!("no sentences to run");
            }
            let mut s = services(&paths, cfg, true).await;
            tools::bench(&mut s, sentences, runs.max(1)).await?;
            Ok(0)
        }
        Command::EvalOcr { dir, engine, paddle_vl_repo, gemma_repo, vlm_args, vlm_endpoint, vlm_prompt } => {
            use maru_engine::vlm_ocr::{GENERAL_VLM_PROMPT, PADDLE_VL_PROMPT, SERVER_READY, VlmOcr};
            let s = services(&paths, cfg, false).await;
            let cases = tools::load_cases(&dir)?;
            println!("{} test image(s)\n", cases.len());
            let mut scores = Vec::new();
            let extra: Vec<String> = vlm_args.as_deref().unwrap_or("").split_whitespace().map(str::to_string).collect();
            // Vision-model servers take turns, so they never compete for VRAM (or for timing).
            let vlm_cfg = |repo: String| {
                let mut c = s.engine.config().llm.clone();
                c.endpoint = None;
                c.model_path = None;
                c.draft_model_path = None;
                c.hf_repo = Some(repo);
                c.vision = true;
                c.port = 18090;
                c.parallel = 1;
                c.ctx_size = 8192;
                c.extra_args = extra.clone();
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
                    EngineArg::Paddle => {
                        let pc = maru_ocr::paddle::PaddleConfig::resolve(&s.engine.config().ocr, &paths);
                        match maru_ocr::paddle::PaddleOcr::load(&pc) {
                            Ok(o) => {
                                let o: Arc<dyn maru_ocr::OcrEngine> = Arc::new(o);
                                // Same spacing repair the overlay applies, so the score reflects real use.
                                let o = maru_engine::respace::RespacingOcr::wrap(o, &s.engine.analyzer());
                                (format!("PP-OCR ({})", pc.device), o, None)
                            }
                            Err(err) => {
                                eprintln!("(PP-OCR unavailable: {err}; skipping)\n");
                                continue;
                            }
                        }
                    }
                    EngineArg::PaddleVl | EngineArg::Gemma => {
                        let (label, repo, prompt) = if e == EngineArg::PaddleVl {
                            ("PaddleOCR-VL-1.5", paddle_vl_repo.clone(), PADDLE_VL_PROMPT)
                        } else {
                            let repo = gemma_repo
                                .clone()
                                .or_else(|| s.engine.config().llm.hf_repo.clone())
                                .unwrap_or_else(|| setup::RECOMMENDED_MODEL.to_string());
                            ("Gemma 4 (vision)", repo, GENERAL_VLM_PROMPT)
                        };
                        println!("Starting {label} from {repo} (the first run downloads the model; this can take a while) …");
                        let cfg = vlm_cfg(repo);
                        match maru_llm::LlamaServer::start(&cfg, &paths, SERVER_READY).await {
                            Ok(server) => {
                                let ocr: Arc<dyn maru_ocr::OcrEngine> = Arc::new(VlmOcr::new(&format!("http://127.0.0.1:{}", cfg.port), &cfg, prompt));
                                (label.to_string(), ocr, Some(server))
                            }
                            Err(err) => {
                                eprintln!("({label} could not start: {err}; skipping)\n  Tip: start it yourself and use --engine endpoint --vlm-endpoint <url>; see `maru eval-ocr --help`.\n");
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
                // Recognition is blocking (and for vision models waits on HTTP): keep the runtime free.
                let (task_name, cases) = (name.clone(), cases.clone());
                let result = tokio::task::spawn_blocking(move || tools::eval_engine(&task_name, ocr.as_ref(), &cases)).await?;
                match result {
                    Ok(score) => scores.push(score),
                    Err(err) => eprintln!("({name} failed: {err:#})\n"),
                }
            }
            if scores.is_empty() {
                bail!("no OCR engine could be evaluated; run `maru setup ocr` first");
            }
            tools::print_scores(&scores);
            Ok(0)
        }
        Command::Setup(cmd) => {
            match cmd {
                SetupCommand::Kiwi { release } => setup::kiwi(&paths, release).await?,
                SetupCommand::Ocr { from_dir, base_url, det_url, rec_url, dict_url } => {
                    setup::ocr(&paths, &cfg, setup::OcrSources { from_dir, base_url, det_url, rec_url, dict_url }).await?
                }
                SetupCommand::Llm { backend, cuda } => setup::llm(&paths, &cfg, backend, cuda).await?,
            }
            Ok(0)
        }
        Command::Dict(cmd) => {
            paths.ensure()?;
            let dict = maru_dict::Dictionary::open(&paths.dict_db())?;
            match cmd {
                DictCommand::Download { url } => {
                    let url = url.unwrap_or_else(|| setup::KAIKKI_KOREAN_URL.to_string());
                    let tmp = paths.data_dir.join("kaikki-korean.jsonl");
                    setup::download(&url, &tmp, "Korean dictionary").await?;
                    let stats = import_jsonl(&dict, &tmp)?;
                    std::fs::remove_file(&tmp).ok();
                    println!("Imported {} entries ({} inflected forms; {} lines skipped).", stats.entries, stats.forms, stats.skipped);
                }
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

fn import_jsonl(dict: &maru_dict::Dictionary, path: &std::path::Path) -> Result<maru_dict::ImportStats> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let total = file.metadata().map(|m| m.len()).unwrap_or(0);
    eprintln!("Importing {} ({:.0} MB) …", path.display(), total as f64 / 1_048_576.0);
    Ok(dict.import_kaikki(std::io::BufReader::new(file), |n| eprint!("\r  {n} entries   "))?)
}
