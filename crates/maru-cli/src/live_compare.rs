//! CLI wiring for the isolated live-model diagnostic.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args as ClapArgs, ValueEnum};
use maru_core::Paths;
use maru_core::config::LlmConfig;
use maru_llm::live_bench::{self, Options, Profile, Sampling};

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum CandidatePrompt {
    HyMt,
    Maru,
    HyMt2,
    Lfm,
    Index,
    Milmmt,
    MilmmtContext,
    TranslateGemmaSub,
    Lmt,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum CandidateSampling {
    Matched,
    Tencent,
    Lfm,
    Greedy,
}

#[derive(ClapArgs)]
pub struct Args {
    /// Existing local Gemma GGUF; this command never downloads models.
    #[arg(long)]
    baseline: PathBuf,
    /// Existing local candidate GGUF, e.g. HY-MT1.5-1.8B Q8.
    #[arg(long)]
    candidate: PathBuf,
    /// A NEW folder for report.json, records.jsonl and review.md.
    #[arg(long)]
    out: PathBuf,
    /// Optional llama-server executable; defaults to the installed one.
    #[arg(long)]
    server: Option<PathBuf>,
    /// JSONL dialogue corpus; defaults to the bundled 40-line test.
    #[arg(long)]
    file: Option<PathBuf>,
    #[arg(short = 'n', long, default_value_t = 3, value_parser = clap::value_parser!(u32).range(1..))]
    runs: u32,
    #[arg(long, default_value_t = 19180, value_parser = clap::value_parser!(u16).range(1..))]
    port: u16,
    #[arg(long, default_value_t = 4)]
    context_lines: usize,
    /// Use each model's previous answers instead of identical reference context.
    #[arg(long)]
    rolling_context: bool,
    #[arg(long, value_enum, default_value_t = CandidatePrompt::HyMt)]
    candidate_prompt: CandidatePrompt,
    /// Matched: temperature 0.1; Tencent: HY-MT core sampling; LFM: 0.3; greedy: temperature 0 for compatible translation profiles.
    #[arg(long, value_enum, default_value_t = CandidateSampling::Matched)]
    candidate_sampling: CandidateSampling,
    /// A note in the report; this command never starts or stops your game.
    #[arg(long, default_value = "unspecified")]
    game_state: String,
}

pub async fn run(args: Args, cfg: &LlmConfig, paths: &Paths) -> Result<i32> {
    let text = match args.file {
        Some(file) => std::fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?,
        None => include_str!("../testdata/live-dialogue.jsonl").into(),
    };
    let cases = live_bench::parse_cases(&text)?;
    let mut cfg = cfg.clone();
    if let Some(server) = args.server {
        cfg.server_path = Some(server);
    }
    let options = Options {
        baseline: args.baseline, candidate: args.candidate, out: args.out, runs: args.runs as usize,
        context_lines: args.context_lines, rolling_context: args.rolling_context,
        candidate_profile: match args.candidate_prompt { CandidatePrompt::HyMt => Profile::HyMt, CandidatePrompt::Maru => Profile::Maru, CandidatePrompt::HyMt2 => Profile::HyMt2, CandidatePrompt::Lfm => Profile::Lfm, CandidatePrompt::Index => Profile::Index, CandidatePrompt::Milmmt => Profile::Milmmt, CandidatePrompt::MilmmtContext => Profile::MilmmtContext, CandidatePrompt::TranslateGemmaSub => Profile::TranslateGemmaSub, CandidatePrompt::Lmt => Profile::Lmt },
        candidate_sampling: match args.candidate_sampling { CandidateSampling::Matched => Sampling::Matched, CandidateSampling::Tencent => Sampling::Tencent, CandidateSampling::Lfm => Sampling::Lfm, CandidateSampling::Greedy => Sampling::Greedy },
        port: args.port, game_state: args.game_state,
    };
    println!("Model-only A/B/A: {} lines x {} runs per pass; warm-up excluded. Quit Maru before running.", cases.len(), options.runs);
    let summaries = live_bench::compare(&cfg, paths, &cases, &options, &mut |s| println!("{s}")).await?;
    for summary in &summaries {
        println!("{}: {} attempts, {} errors", summary.pass, summary.attempted, summary.errors);
        for (name, distribution) in [("first text", &summary.first_text), ("done", &summary.done)] {
            if let Some(d) = distribution {
                println!("  {name}: p50 {:.0} ms, p95 {:.0} ms ({} successes)", d.p50_ms, d.p95_ms, d.n);
            } else {
                println!("  {name}: no successful data");
            }
        }
    }
    println!("Review translations in {}", options.out.join("review.md").display());
    Ok(i32::from(summaries.iter().any(|s| s.errors > 0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_dialogue_retains_numbers_negation_names_and_fragments_for_review() {
        let cases = live_bench::parse_cases(include_str!("../testdata/live-dialogue.jsonl")).unwrap();
        assert_eq!(cases.len(), 40);
        for tag in ["numbers", "negation", "names", "fragment", "pronoun"] {
            assert!(cases.iter().any(|c| c.tags.iter().any(|t| t == tag)), "missing {tag}");
        }
    }
}
