//! `maru setup`: look at this PC, propose what to install, ask the few questions that matter, then
//! run the same installers as `maru setup llm|kiwi|ocr|…` and finish with `maru doctor --deep`.
//!
//! The work is split so the decisions can be tested without a terminal or a network:
//! [`decide`] asks questions through a [`Ui`] and returns a [`Decision`]; [`apply_config`] writes it
//! to `config.toml`; [`execute`] runs the installers. Every installer is safe to run again, so a
//! failed run is finished by running `maru setup` again.

use std::io::{BufRead, Write};

use anyhow::{Context, Result};
use maru_core::config::Config;
use maru_core::{Paths, settings};
use maru_engine::hardware::{self, GpuClass, Hardware, Vendor};
use maru_engine::plan::{self, Backend, Plan, Step};

use crate::setup::{self, LlamaBackend};

#[derive(clap::Args, Default, Clone)]
pub struct WizardArgs {
    /// Accept every recommendation without asking (for scripts and repeat runs).
    #[arg(long)]
    pub yes: bool,
    /// Show what this PC has and what would be installed, then stop.
    #[arg(long)]
    pub dry_run: bool,
    /// Download llama.cpp again even if a llama-server already works (to change the backend).
    #[arg(long)]
    pub force: bool,
    /// Use this llama.cpp build instead of the recommended one.
    #[arg(long, value_enum)]
    pub backend: Option<LlamaBackend>,
}

fn backend_of(b: LlamaBackend) -> Backend {
    match b {
        LlamaBackend::Cuda => Backend::Cuda,
        LlamaBackend::Vulkan => Backend::Vulkan,
        LlamaBackend::Cpu => Backend::Cpu,
    }
}

fn llama_of(b: Backend) -> LlamaBackend {
    match b {
        Backend::Cuda => LlamaBackend::Cuda,
        Backend::Vulkan => LlamaBackend::Vulkan,
        Backend::Cpu => LlamaBackend::Cpu,
    }
}

// ─────────────────────────────── asking ───────────────────────────────

/// How the wizard talks to the person (and how tests answer for them).
pub trait Ui {
    fn say(&mut self, text: &str);
    fn yes_no(&mut self, question: &str, default: bool) -> bool;
    /// Returns the index of the chosen option.
    fn choose(&mut self, question: &str, options: &[String], default: usize) -> usize;
    /// The input ended (Ctrl+D, a closed pipe): nobody is there to confirm anything, so nothing may be installed.
    fn aborted(&self) -> bool {
        false
    }
}

/// A real terminal: prompts on stdout, answers on stdin. An empty answer takes the default.
#[derive(Default)]
pub struct Terminal {
    ended: bool,
}

impl Terminal {
    /// One line of input; `None` once the input has ended.
    fn read_answer(&mut self) -> Option<String> {
        let mut line = String::new();
        match std::io::stdin().lock().read_line(&mut line) {
            Ok(0) | Err(_) => {
                self.ended = true;
                println!();
                None
            }
            Ok(_) => Some(line.trim().to_ascii_lowercase()),
        }
    }
}

impl Ui for Terminal {
    fn aborted(&self) -> bool {
        self.ended
    }

    fn say(&mut self, text: &str) {
        println!("{text}");
    }

    fn yes_no(&mut self, question: &str, default: bool) -> bool {
        loop {
            print!("{question} {} ", if default { "[Y/n]" } else { "[y/N]" });
            let _ = std::io::stdout().flush();
            match self.read_answer().as_deref() {
                None => return false,
                Some("") => return default,
                Some("y" | "yes") => return true,
                Some("n" | "no") => return false,
                Some(_) => println!("  Please answer y or n."),
            }
        }
    }

    fn choose(&mut self, question: &str, options: &[String], default: usize) -> usize {
        println!("{question}");
        for (i, o) in options.iter().enumerate() {
            println!("  {}) {o}{}", i + 1, if i == default { "  (recommended)" } else { "" });
        }
        loop {
            print!("Choose 1-{} [{}]: ", options.len(), default + 1);
            let _ = std::io::stdout().flush();
            let Some(answer) = self.read_answer() else { return default };
            if answer.is_empty() {
                return default;
            }
            match answer.parse::<usize>() {
                Ok(n) if (1..=options.len()).contains(&n) => return n - 1,
                _ => println!("  Please type a number from 1 to {}.", options.len()),
            }
        }
    }
}

/// `--yes`: prints each question with the answer it takes.
pub struct Automatic;

impl Ui for Automatic {
    fn say(&mut self, text: &str) {
        println!("{text}");
    }

    fn yes_no(&mut self, question: &str, default: bool) -> bool {
        println!("{question} -> {}", if default { "yes" } else { "no" });
        default
    }

    fn choose(&mut self, question: &str, options: &[String], default: usize) -> usize {
        println!("{question} -> {}", options[default]);
        default
    }
}

// ─────────────────────────────── describing ───────────────────────────────

/// Memory and disk sizes (binary units, as Windows shows them).
fn gb(mb: u64) -> String {
    format!("{:.1} GB", mb as f64 / 1024.0)
}

/// A download size in the decimal units download pages use: "150 MB", "2.5 GB".
pub fn size(mb: u64) -> String {
    if mb < 1000 { format!("{mb} MB") } else { format!("{:.1} GB", mb as f64 / 1000.0) }
}

/// What was found, in a few lines.
pub fn describe(hw: &Hardware) -> String {
    let mut out = String::new();
    if hw.gpus.is_empty() {
        out.push_str("  Graphics:  none found\n");
    }
    for g in &hw.gpus {
        let kind = match (g.vendor, g.class) {
            (_, GpuClass::None) => " (no real graphics card; not used)".to_string(),
            (_, GpuClass::Integrated) => " (shares the computer's memory)".to_string(),
            (Vendor::Nvidia, _) => g.vram_mb.map(|m| format!(", {}", gb(m))).unwrap_or_default(),
            _ => " (memory size not reported by Windows)".to_string(),
        };
        out.push_str(&format!("  Graphics:  {}{kind}\n", g.name));
    }
    if let Some(n) = &hw.nvidia {
        let cuda = n.cuda.map(|(a, b)| format!(", runs CUDA up to {a}.{b}")).unwrap_or_default();
        out.push_str(&format!("  Driver:    NVIDIA {}{cuda}\n", n.driver));
    }
    out.push_str(&format!("  Memory:    {}\n", hw.ram_mb.map(gb).unwrap_or_else(|| "unknown".into())));
    out.push_str(&format!("  Free disk: {}\n", hw.free_disk_mb.map(gb).unwrap_or_else(|| "unknown".into())));
    out
}

fn model_name(plan: &Plan) -> &'static str {
    if plan.lite { "the small model (faster, lighter, a little less accurate)" } else { "the full 12B model (best quality)" }
}

pub fn describe_plan(plan: &Plan) -> String {
    let mut out = format!("  Run the model with: {}\n  Model:              {}\n", plan.backend.label(), model_name(plan));
    for r in &plan.reasons {
        out.push_str(&format!("    - {r}\n"));
    }
    for w in &plan.warnings {
        out.push_str(&format!("    ! {w}\n"));
    }
    out
}

// ─────────────────────────────── deciding ───────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub plan: Plan,
    pub ocr_vl: bool,
    pub tts: bool,
    /// The model comes from `[llm] endpoint` (another PC, LM Studio…): nothing to install for it.
    pub external_model: bool,
    pub proceed: bool,
}

impl Decision {
    pub fn steps(&self) -> Vec<Step> {
        let mut steps = plan::steps(&self.plan, self.ocr_vl, self.tts);
        if self.external_model {
            steps.retain(|s| *s != Step::Llm);
        }
        steps
    }
}

pub fn decide(hw: &Hardware, cfg: &Config, backend_override: Option<LlamaBackend>, ui: &mut dyn Ui) -> Decision {
    ui.say("This PC:");
    ui.say(describe(hw).trim_end());

    let mut plan = plan::recommend(hw);
    if let Some(b) = backend_override {
        plan.backend = backend_of(b);
        plan.reasons.push("The llama.cpp build was chosen on the command line.".to_string());
    }
    let external_model = cfg.llm.endpoint.is_some();
    ui.say("\nRecommended setup:");
    ui.say(describe_plan(&plan).trim_end());
    if external_model {
        ui.say("  The translation model is read from [llm] endpoint (another PC or program), so none is installed here.");
    }

    if !ui.yes_no("\nUse this setup?", true) {
        let backends = [Backend::Cuda, Backend::Vulkan, Backend::Cpu];
        let labels: Vec<String> = backends.iter().map(|b| b.label().to_string()).collect();
        let default = backends.iter().position(|b| *b == plan.backend).unwrap_or(0);
        plan.backend = backends[ui.choose("\nWhich llama.cpp build should run the model?", &labels, default)];
        let models = ["The full 12B model (best quality; wants about 10 GB of video memory)".to_string(), "The small model (faster and lighter; fine for most things)".to_string()];
        plan.lite = ui.choose("\nWhich translation model?", &models, usize::from(plan.lite)) == 1;
        plan.ocr_vl = !plan.lite;
    }

    // Optional extras.
    ui.say("\nOptional extras:");
    let ocr_vl = if plan.lite {
        false
    } else {
        ui.yes_no(&format!("  OCR reading model (about {}): fixes most letters PP-OCR misreads. Install it?", size(Step::OcrVl.download_mb(&plan))), plan.ocr_vl)
    };
    let tts = ui.yes_no(&format!("  Korean voice (about {}): read text aloud. It uses the GPU, so skip it if you play on this PC; `maru setup tts` adds it later. Install it?", size(Step::Tts.download_mb(&plan))), false);

    let mut decision = Decision { plan, ocr_vl, tts, external_model, proceed: false };
    let steps = decision.steps();
    ui.say("\nThis will download and install:");
    for s in &steps {
        ui.say(&format!("  - {} (about {})", s.label(), size(s.download_mb(&decision.plan))));
    }
    let total = plan::total_download_mb(&steps, &decision.plan);
    ui.say(&format!("  Total: about {}", size(total)));
    if let Some(w) = plan::disk_warning(total, hw.free_disk_mb) {
        ui.say(&format!("  ! {w}"));
    }
    decision.proceed = ui.yes_no("\nStart the installation?", true) && !ui.aborted();
    decision
}

/// Write what the wizard decided into `config.toml`: a new file, or the existing one edited in place
/// (comments and your other settings stay). Returns the configuration as it now is.
pub fn apply_config(paths: &Paths, plan: &Plan) -> Result<Config> {
    paths.ensure()?;
    let file = &paths.config_file;
    if !file.exists() {
        let mut cfg = Config::default();
        cfg.llm.hf_repo = Some(setup::RECOMMENDED_MODEL.to_string());
        cfg.lite = plan.lite;
        cfg.save(file)?;
        return Ok(cfg);
    }
    let text = std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let current = Config::load(file)?;
    let mut changes = plan::config_changes(plan);
    if current.llm.hf_repo.is_none() && current.llm.model_path.is_none() && current.llm.endpoint.is_none() {
        changes.insert("llm.hf_repo".into(), setup::RECOMMENDED_MODEL.into());
    }
    let patched = settings::patch(&text, &changes).with_context(|| format!("updating {}", file.display()))?;
    if patched.text != text {
        std::fs::write(file, &patched.text).with_context(|| format!("writing {}", file.display()))?;
    }
    Ok(patched.config)
}

// ─────────────────────────────── running ───────────────────────────────

async fn run_step(paths: &Paths, cfg: &Config, step: Step, decision: &Decision, force: bool) -> Result<()> {
    match step {
        Step::Llm => {
            // With `lite` on, the model to fetch is the small one that will actually run.
            let effective = cfg.clone().effective().0;
            setup::llm(paths, &effective, llama_of(decision.plan.backend), None, force).await
        }
        Step::Kiwi => {
            if maru_engine::bootstrap::find_kiwi(paths).is_some() {
                println!("Kiwi is already installed.");
                return Ok(());
            }
            setup::kiwi(paths, None).await
        }
        Step::Ocr => {
            let out = paths.models_dir.join("ocr");
            if ["det.onnx", "rec.onnx", "dict.txt"].iter().all(|f| out.join(f).metadata().is_ok_and(|m| m.len() > 0)) {
                println!("The OCR models are already installed.");
                return Ok(());
            }
            setup::ocr(paths, cfg, setup::OcrSources::default()).await
        }
        Step::OcrVl => crate::fetch_paddle_vl(paths).await,
        Step::Dictionary => {
            paths.ensure()?;
            let dict = maru_dict::Dictionary::open(&paths.dict_db())?;
            if dict.entry_count()? > 0 {
                println!("The dictionary is already installed ({} entries).", dict.entry_count()?);
                return Ok(());
            }
            crate::download_dictionary(paths, &dict, None).await
        }
        Step::Tts => crate::tts::setup(paths, cfg, crate::tts::SetupOptions { small: false, server_url: None, no_test: false }).await,
    }
}

/// Run the steps one after another; one failing does not stop the others. Returns the ones that failed.
pub async fn execute(paths: &Paths, cfg: &Config, decision: &Decision, force: bool) -> Vec<(Step, anyhow::Error)> {
    let steps = decision.steps();
    let mut failed = Vec::new();
    for (i, step) in steps.iter().enumerate() {
        println!("\n[{}/{}] {}", i + 1, steps.len(), step.label());
        if let Err(e) = run_step(paths, cfg, *step, decision, force).await {
            eprintln!("  ✗ {e:#}");
            failed.push((*step, e));
        }
    }
    failed
}

pub async fn run(paths: &Paths, args: WizardArgs) -> Result<i32> {
    use std::io::IsTerminal;
    if !args.yes && !args.dry_run && !std::io::stdin().is_terminal() {
        anyhow::bail!("`maru setup` asks questions and needs a terminal; add --yes to accept the recommendations without asking");
    }
    let cfg = Config::load(&paths.config_file).with_context(|| format!("reading {}", paths.config_file.display()))?;

    println!("Maru setup\n==========\nLooking at this PC …\n");
    let hw = hardware::detect(&paths.models_dir);
    let mut auto = Automatic;
    let mut terminal = Terminal::default();
    let ui: &mut dyn Ui = if args.yes || args.dry_run { &mut auto } else { &mut terminal };
    let decision = decide(&hw, &cfg, args.backend, ui);

    if args.dry_run {
        println!("\n(dry run: nothing was changed. `maru setup wizard --yes` takes the answers shown above.)");
        return Ok(0);
    }
    if !decision.proceed {
        println!("\nNothing was installed. Run `maru setup` again whenever you like.");
        return Ok(0);
    }

    let cfg = apply_config(paths, &decision.plan)?;
    println!("\nSettings saved in {} (lite = {}).", paths.config_file.display(), if cfg.lite { "on" } else { "off" });
    let failed = execute(paths, &cfg, &decision, args.force).await;

    if !failed.is_empty() {
        eprintln!("\nThese steps did not finish:");
        for (step, e) in &failed {
            eprintln!("  - {}: {e:#}", step.label());
        }
        eprintln!("\nFix the problem (a network error is the usual one) and run `maru setup` again: it skips what is already installed.");
        return Ok(1);
    }

    println!("\nEverything is installed. Running the full check (it starts the model, which can take a minute) …\n");
    let cfg = Config::load(&paths.config_file)?;
    let mut services = crate::services(paths, cfg, true).await;
    crate::tools::doctor(&mut services, true).await?;
    println!("\nDone. Start the overlay with:  cd app\\src-tauri; cargo run --release");
    println!("Settings (tray icon → Settings…) is where you change shortcuts, use a model on another PC, and more.");
    Ok(0)
}

// ─────────────────────────────── tests ───────────────────────────────

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use maru_engine::hardware::{Gpu, NvidiaDriver};

    use super::*;

    /// Answers questions from a script and remembers what was asked.
    struct Scripted {
        yes: VecDeque<bool>,
        choices: VecDeque<usize>,
        asked: Vec<String>,
        said: String,
    }

    impl Scripted {
        fn new(yes: &[bool], choices: &[usize]) -> Self {
            Self { yes: yes.iter().copied().collect(), choices: choices.iter().copied().collect(), asked: Vec::new(), said: String::new() }
        }
    }

    impl Ui for Scripted {
        fn say(&mut self, text: &str) {
            self.said.push_str(text);
            self.said.push('\n');
        }
        fn yes_no(&mut self, question: &str, default: bool) -> bool {
            self.asked.push(question.trim().to_string());
            self.yes.pop_front().unwrap_or(default)
        }
        fn choose(&mut self, question: &str, _options: &[String], default: usize) -> usize {
            self.asked.push(question.trim().to_string());
            self.choices.pop_front().unwrap_or(default)
        }
    }

    fn rtx4070() -> Hardware {
        Hardware {
            gpus: vec![Gpu { name: "NVIDIA GeForce RTX 4070".into(), vendor: Vendor::Nvidia, class: GpuClass::Discrete, vram_mb: Some(12_282) }],
            nvidia: Some(NvidiaDriver { driver: "552.44".into(), cuda: Some((12, 4)) }),
            ram_mb: Some(32_768),
            free_disk_mb: Some(500_000),
        }
    }

    fn integrated() -> Hardware {
        Hardware {
            gpus: vec![Gpu { name: "Intel(R) UHD Graphics 770".into(), vendor: Vendor::Intel, class: GpuClass::Integrated, vram_mb: None }],
            nvidia: None,
            ram_mb: Some(16_384),
            free_disk_mb: Some(500_000),
        }
    }

    fn scratch(name: &str) -> Paths {
        let root = std::env::temp_dir().join(format!("maru-wizard-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        Paths::under(&root)
    }

    #[test]
    fn download_sizes_are_written_the_way_download_pages_do() {
        assert_eq!(size(150), "150 MB");
        assert_eq!(size(2_500), "2.5 GB");
        assert_eq!(size(7_500 + 900), "8.4 GB");
        assert_eq!(gb(12_282), "12.0 GB");
    }

    #[test]
    fn accepting_every_default_on_a_strong_nvidia_pc() {
        let mut ui = Scripted::new(&[], &[]);
        let d = decide(&rtx4070(), &Config::default(), None, &mut ui);
        assert_eq!((d.plan.backend, d.plan.lite, d.ocr_vl, d.tts, d.proceed), (Backend::Cuda, false, true, false, true));
        assert_eq!(d.steps(), [Step::Llm, Step::Kiwi, Step::Ocr, Step::OcrVl, Step::Dictionary]);
        assert!(ui.said.contains("NVIDIA GeForce RTX 4070, 12.0 GB"), "{}", ui.said);
        assert!(ui.said.contains("CUDA up to 12.4"), "{}", ui.said);
        assert!(ui.said.contains("Total: about"), "{}", ui.said);
    }

    #[test]
    fn a_weak_pc_is_not_asked_about_the_reading_model() {
        let mut ui = Scripted::new(&[], &[]);
        let d = decide(&integrated(), &Config::default(), None, &mut ui);
        assert_eq!((d.plan.backend, d.plan.lite, d.ocr_vl), (Backend::Cpu, true, false));
        assert!(!ui.asked.iter().any(|q| q.contains("reading model")), "{:?}", ui.asked);
        assert!(!d.steps().contains(&Step::OcrVl));
    }

    #[test]
    fn refusing_the_recommendation_lets_the_person_pick_the_build_and_model() {
        // "Use this setup?" no → Vulkan (index 1), small model (index 1); then the voice: yes; start: yes
        let mut ui = Scripted::new(&[false, true, true], &[1, 1]);
        let d = decide(&rtx4070(), &Config::default(), None, &mut ui);
        assert_eq!((d.plan.backend, d.plan.lite, d.ocr_vl, d.tts, d.proceed), (Backend::Vulkan, true, false, true, true));
        assert_eq!(d.steps(), [Step::Llm, Step::Kiwi, Step::Ocr, Step::Dictionary, Step::Tts]);
    }

    #[test]
    fn the_command_line_backend_replaces_the_recommendation() {
        let mut ui = Scripted::new(&[], &[]);
        let d = decide(&rtx4070(), &Config::default(), Some(LlamaBackend::Vulkan), &mut ui);
        assert_eq!(d.plan.backend, Backend::Vulkan);
        assert!(ui.said.contains("command line"));
    }

    #[test]
    fn input_that_ends_early_installs_nothing() {
        struct Closed;
        impl Ui for Closed {
            fn say(&mut self, _: &str) {}
            fn yes_no(&mut self, _: &str, default: bool) -> bool {
                default // what a careless implementation would do at the end of input
            }
            fn choose(&mut self, _: &str, _: &[String], default: usize) -> usize {
                default
            }
            fn aborted(&self) -> bool {
                true
            }
        }
        assert!(!decide(&rtx4070(), &Config::default(), None, &mut Closed).proceed);
    }

    #[test]
    fn declining_the_final_question_installs_nothing() {
        let mut ui = Scripted::new(&[true, true, false, false], &[]);
        let d = decide(&rtx4070(), &Config::default(), None, &mut ui);
        assert!(!d.proceed);
    }

    #[test]
    fn a_model_on_another_pc_means_no_model_is_installed_here() {
        let cfg = Config { llm: maru_core::config::LlmConfig { endpoint: Some("http://192.168.1.20:8080/v1".into()), ..Default::default() }, ..Config::default() };
        let mut ui = Scripted::new(&[], &[]);
        let d = decide(&rtx4070(), &cfg, None, &mut ui);
        assert!(d.external_model);
        assert!(!d.steps().contains(&Step::Llm));
        assert!(ui.said.contains("endpoint"));
    }

    #[test]
    fn a_small_disk_is_reported() {
        let mut hw = rtx4070();
        hw.free_disk_mb = Some(6_000);
        let mut ui = Scripted::new(&[], &[]);
        decide(&hw, &Config::default(), None, &mut ui);
        assert!(ui.said.contains("is free on the drive"), "{}", ui.said);
    }

    #[test]
    fn a_new_config_file_gets_the_model_and_the_lite_setting() {
        let paths = scratch("new");
        let lite = plan::recommend(&integrated());
        let cfg = apply_config(&paths, &lite).unwrap();
        assert!(cfg.lite);
        assert_eq!(cfg.llm.hf_repo.as_deref(), Some(setup::RECOMMENDED_MODEL));
        assert_eq!(Config::load(&paths.config_file).unwrap(), cfg);
    }

    #[test]
    fn an_existing_config_keeps_its_comments_and_other_settings() {
        let paths = scratch("existing");
        paths.ensure().unwrap();
        std::fs::write(&paths.config_file, "# my notes\nlite = false   # beside the game\n\n[hotkeys]\ntranslate = \"F6\"\n").unwrap();
        let lite = plan::recommend(&integrated());
        let cfg = apply_config(&paths, &lite).unwrap();
        assert!(cfg.lite);
        assert_eq!(cfg.hotkeys.translate, "F6");
        let text = std::fs::read_to_string(&paths.config_file).unwrap();
        assert!(text.contains("# my notes") && text.contains("# beside the game"), "{text}");
        assert!(text.contains("lite = true"), "{text}");
        assert!(text.contains(setup::RECOMMENDED_MODEL), "a config with no model gets the recommended one: {text}");
    }

    #[test]
    fn a_config_that_names_its_own_model_is_not_given_another() {
        let paths = scratch("own-model");
        paths.ensure().unwrap();
        std::fs::write(&paths.config_file, "[llm]\nmodel_path = \"D:/models/x.gguf\"\n").unwrap();
        let cfg = apply_config(&paths, &plan::recommend(&rtx4070())).unwrap();
        assert!(cfg.llm.hf_repo.is_none());
        assert!(!cfg.lite);
    }
}
