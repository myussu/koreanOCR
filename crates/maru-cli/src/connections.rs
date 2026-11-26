//! Saved model connections from the command line: list them, check that each one answers through Maru's own
//! client, ask a server which models it offers, and time live subtitles on several of them with the same lines.
//!
//! A connection is chosen by the name it was saved under in Settings (`[llm].servers`); its key is used and never
//! shown. `local` means the model this PC runs itself. Nothing here opens the vocabulary or history databases.

use std::fmt::Write as _;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use maru_core::Paths;
use maru_core::config::{Config, LlmConfig};
use maru_core::external_servers::ServerProvider;
use maru_engine::senses::{CardWord, Decided, decide, questions};
use maru_llm::senses::{SenseAnswer, SenseOption, SenseRequest, SenseWord};
use maru_llm::{LiveRequest, LlamaServer, LlmChunk, LlmClient, PromptContext};
use serde::{Deserialize, Serialize};

/// The connection name for the model this PC runs itself.
pub const LOCAL: &str = "local";
/// Not a connection: the dictionary's first sense for every word, with no model asked.
pub const DICTIONARY: &str = "dictionary";

/// A line and its words to learn with their dictionary senses (`maru dict senses`): what `bench --cards` asks about.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardTask {
    pub line: String,
    pub words: Vec<CardWord>,
}

/// What one candidate decided for one line.
#[derive(Debug, Clone, Serialize)]
struct CardRecord<'a> {
    candidate: &'a str,
    /// The line's number in the tasks file, from 1.
    line: usize,
    /// A model was asked (a line whose words all have one sense needs none).
    asked: bool,
    ms: u32,
    error: Option<String>,
    tokens_in: u64,
    tokens_out: u64,
    words: Vec<Decided>,
    /// Requests made for the line: 2 when a word with many senses was asked about again.
    requests: usize,
    /// Words that got a sense beyond the first eight, by index.
    later: Vec<usize>,
    /// Words whose dictionary form the model corrected: `(index, form)`.
    corrected: Vec<(usize, String)>,
}

/// `name`, or `name=model` to ask that connection for another model. `name+native` asks a connection saved for
/// Anthropic's OpenAI-compatible endpoint through Anthropic's own Messages API instead, for this run only.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    pub name: String,
    pub model: Option<String>,
    pub native: bool,
}

impl Target {
    pub fn parse(text: &str) -> Self {
        let (name, model) = match text.split_once('=') {
            Some((name, model)) if !model.trim().is_empty() => (name.trim(), Some(model.trim().to_string())),
            _ => (text.trim(), None),
        };
        match name.strip_suffix("+native") {
            Some(name) => Self { name: name.trim().into(), model, native: true },
            None => Self { name: name.into(), model, native: false },
        }
    }
}

/// The settings to talk to a saved connection: found by name whatever the capitals, or by the beginning of its
/// name when only one starts that way ("azure" finds "Azure Contoso").
pub fn resolve(cfg: &Config, target: &Target) -> Result<(String, LlmConfig)> {
    let mut llm = cfg.llm.clone();
    if target.name.eq_ignore_ascii_case(LOCAL) {
        llm.endpoint = None;
        llm.provider = ServerProvider::Openai;
        llm.api_key = None;
        llm.model_name = LlmConfig::default().model_name;
        if target.model.is_some() {
            bail!("the local model is the one in [llm]; it cannot be changed from here");
        }
        return Ok((LOCAL.to_string(), llm));
    }
    let wanted = target.name.to_lowercase();
    let exact: Vec<_> = cfg.llm.servers.iter().filter(|s| s.name.to_lowercase() == wanted).collect();
    let found = if exact.is_empty() { cfg.llm.servers.iter().filter(|s| s.name.to_lowercase().starts_with(&wanted)).collect() } else { exact };
    let saved = match found.as_slice() {
        [one] => *one,
        [] => bail!("no saved connection is called \"{}\" (saved: {})", target.name, names(cfg)),
        many => bail!("\"{}\" could be {}; give more of the name", target.name, many.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(" or ")),
    };
    llm.endpoint = Some(saved.endpoint.clone());
    llm.provider = saved.provider;
    llm.api_key = saved.api_key.clone();
    llm.model_name = target.model.clone().unwrap_or_else(|| saved.model_name.clone());
    if target.native {
        if !saved.endpoint.to_ascii_lowercase().contains("api.anthropic.com") {
            bail!("+native is for a connection to Anthropic (api.anthropic.com); \"{}\" is not one", saved.name);
        }
        llm.provider = ServerProvider::Anthropic;
        return Ok((format!("{} (Messages API)", saved.name), llm));
    }
    Ok((saved.name.clone(), llm))
}

fn names(cfg: &Config) -> String {
    let saved: Vec<&str> = cfg.llm.servers.iter().map(|s| s.name.as_str()).collect();
    if saved.is_empty() { "none; save one in Settings → Language model".into() } else { saved.join(", ") }
}

/// A connection ready to be asked; a local model server lives as long as this does.
pub struct Opened {
    /// `Gemini · gemini-flash`.
    pub label: String,
    pub connection: String,
    pub model: String,
    pub client: LlmClient,
    _server: Option<LlamaServer>,
}

pub async fn open(cfg: &Config, paths: &Paths, target: &Target) -> Result<Opened> {
    let (connection, llm) = resolve(cfg, target)?;
    if connection != LOCAL {
        let client = LlmClient::from_config(&llm).counting_tokens();
        return Ok(Opened { label: format!("{connection} · {}", llm.model_name), connection, model: llm.model_name.clone(), client, _server: None });
    }
    eprintln!("Starting the local model …");
    let server = LlamaServer::start(&llm, paths, Duration::from_secs(llm.ready_timeout_secs.max(30))).await.context("starting the local model")?;
    let model = llm.hf_repo.clone().or_else(|| llm.model_path.as_ref().map(|p| p.display().to_string())).unwrap_or_else(|| "local model".into());
    let client = server.client().counting_tokens();
    Ok(Opened { label: format!("{LOCAL} · {model}"), connection, model, client, _server: Some(server) })
}

/// `maru connection list`: what is saved, without the keys.
pub fn list(cfg: &Config) {
    if cfg.llm.servers.is_empty() {
        println!("No saved connections ({}).", names(cfg));
    }
    for s in &cfg.llm.servers {
        let provider = match s.provider {
            ServerProvider::Openai => "OpenAI-compatible",
            ServerProvider::Azure => "Azure OpenAI",
            ServerProvider::Gemini => "Gemini",
            ServerProvider::Anthropic => "Anthropic",
        };
        println!("{:<16} {:<18} {:<46} {}  key: {}", s.name, provider, s.endpoint, s.model_name, if s.api_key.is_some() { "saved" } else { "none" });
    }
    println!("{:<16} the model this PC runs itself ([llm])", LOCAL);
}

/// `maru connection models <name>`.
pub async fn models(cfg: &Config, paths: &Paths, target: &Target) -> Result<()> {
    let opened = open(cfg, paths, target).await?;
    let models = opened.client.list_models().await.map_err(|e| anyhow::anyhow!("{}: {e}", opened.connection))?;
    println!("{} offers {} model(s):", opened.connection, models.len());
    for m in models {
        println!("  {m}");
    }
    Ok(())
}

const CHECK_LINE: &str = "배가 고파서 식당에 갔어요.";

/// `maru connection test <name>`: the Settings check, then one live subtitle line and one card-meaning choice,
/// the two jobs a connection is used for.
pub async fn test(cfg: &Config, paths: &Paths, target: &Target) -> Result<bool> {
    let opened = open(cfg, paths, target).await?;
    println!("{}", opened.label);
    let mut ok = true;

    let live = live_line(&opened.client, CHECK_LINE, &[]).await;
    match &live.error {
        None => println!("  live subtitle   ok   first word {} ms, whole line {} ms{}   \"{}\"", live.first_ms.unwrap_or(live.total_ms), live.total_ms, if live.capped { ", CUT OFF at the length limit" } else { "" }, live.translation),
        Some(e) => {
            ok = false;
            println!("  live subtitle   FAILED after {} ms: {e}", live.total_ms);
        }
    }
    ok &= !live.capped;

    let words = [SenseWord {
        index: 0,
        surface: "배가".into(),
        lemma: "배".into(),
        senses: vec![
            SenseOption { en: "belly; stomach".into(), definition: "The part of the body between the chest and the legs.".into() },
            SenseOption { en: "ship; boat".into(), definition: "A vehicle that carries people or goods on water.".into() },
            SenseOption { en: "pear".into(), definition: "A sweet fruit of the pear tree.".into() },
        ],
    }];
    let started = Instant::now();
    let mut answers = Vec::new();
    let result = opened.client.choose_senses(&SenseRequest { sentence: CHECK_LINE, words: &words }, &mut |a| answers.push(a)).await;
    let took = started.elapsed().as_millis();
    match (result, answers.first()) {
        (Ok(()), Some(SenseAnswer::Sense { sense: 1, .. })) => println!("  card meaning    ok   {took} ms   chose sense 1 (belly) for 배 in \"{CHECK_LINE}\""),
        (Ok(()), Some(other)) => {
            ok = false;
            println!("  card meaning    WRONG in {took} ms: answered {other:?} where sense 1 (belly) fits");
        }
        (Ok(()), None) => {
            ok = false;
            println!("  card meaning    FAILED in {took} ms: the reply had no usable line");
        }
        (Err(e), _) => {
            ok = false;
            println!("  card meaning    FAILED after {took} ms: {e}");
        }
    }
    let used = opened.client.tokens_used();
    if used.replies > 0 {
        println!("  tokens reported by the server: {} in, {} out over {} replies", used.input, used.output, used.replies);
    }
    Ok(ok)
}

/// One live subtitle line, timed.
#[derive(Debug, Clone, Default, Serialize)]
pub struct LiveResult {
    pub first_ms: Option<u32>,
    pub total_ms: u32,
    pub translation: String,
    pub capped: bool,
    pub error: Option<String>,
}

async fn live_line(client: &LlmClient, text: &str, turns: &[(String, String)]) -> LiveResult {
    let started = Instant::now();
    let ms = |t: Instant| t.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
    let mut out = LiveResult::default();
    let ctx = PromptContext::default();
    let result = client
        .translate_live(&LiveRequest { text, ctx: &ctx, turns }, &mut |chunk| match chunk {
            LlmChunk::Text { .. } if out.first_ms.is_none() => out.first_ms = Some(ms(started)),
            LlmChunk::Capped => out.capped = true,
            _ => {}
        })
        .await;
    out.total_ms = ms(started);
    match result {
        Ok(text) if text.trim().is_empty() => out.error = Some("the reply was empty".into()),
        Ok(text) => out.translation = text,
        Err(e) => out.error = Some(e.to_string()),
    }
    out
}

/// What one connection did over every pass.
#[derive(Debug, Clone, Serialize)]
pub struct LiveSummary {
    pub connection: String,
    pub model: String,
    pub lines: usize,
    pub failures: usize,
    pub cut_off: usize,
    pub first_p50_ms: Option<u32>,
    pub first_p95_ms: Option<u32>,
    pub total_p50_ms: Option<u32>,
    pub total_p95_ms: Option<u32>,
    pub total_max_ms: Option<u32>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Of the input tokens, those the server said it read from its prompt cache (0 when it reports none).
    pub cached_input_tokens: u64,
    /// The different failure messages, each with how often it came.
    pub errors: Vec<(String, usize)>,
}

#[derive(Debug, Clone, Serialize)]
struct LiveRecord<'a> {
    connection: &'a str,
    model: &'a str,
    pass: usize,
    line: usize,
    source: &'a str,
    #[serde(flatten)]
    result: LiveResult,
}

fn percentile(sorted: &[u32], p: f64) -> Option<u32> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (p * sorted.len() as f64).ceil() as usize;
    Some(sorted[rank.clamp(1, sorted.len()) - 1])
}

/// Summarise one connection's timed lines; only lines that succeeded count towards the times.
pub fn summarize(connection: &str, model: &str, results: &[LiveResult], tokens: (u64, u64)) -> LiveSummary {
    let ok: Vec<&LiveResult> = results.iter().filter(|r| r.error.is_none()).collect();
    let mut first: Vec<u32> = ok.iter().map(|r| r.first_ms.unwrap_or(r.total_ms)).collect();
    let mut total: Vec<u32> = ok.iter().map(|r| r.total_ms).collect();
    first.sort_unstable();
    total.sort_unstable();
    let mut errors: Vec<(String, usize)> = Vec::new();
    for e in results.iter().filter_map(|r| r.error.as_deref()) {
        let short: String = e.chars().take(160).collect();
        match errors.iter_mut().find(|(known, _)| *known == short) {
            Some((_, n)) => *n += 1,
            None => errors.push((short, 1)),
        }
    }
    LiveSummary {
        connection: connection.into(),
        model: model.into(),
        lines: results.len(),
        failures: results.len() - ok.len(),
        cut_off: results.iter().filter(|r| r.capped).count(),
        first_p50_ms: percentile(&first, 0.5),
        first_p95_ms: percentile(&first, 0.95),
        total_p50_ms: percentile(&total, 0.5),
        total_p95_ms: percentile(&total, 0.95),
        total_max_ms: total.last().copied(),
        input_tokens: tokens.0,
        output_tokens: tokens.1,
        cached_input_tokens: 0,
        errors,
    }
}

/// The comparison as a Markdown table, one row per connection.
/// `context` says whether each line was sent with the lines before it.
pub fn table(summaries: &[LiveSummary], sentences: usize, runs: usize, context: bool) -> String {
    let ms = |v: Option<u32>| v.map_or("–".to_string(), |v| v.to_string());
    let mut out = String::new();
    let how = if context { "each line sent with the lines before it as context" } else { "each line sent by itself" };
    let _ = writeln!(out, "Live subtitles: {sentences} lines × {runs} passes per connection, {how}, one at a time. Times in ms; failed lines are counted, not timed.\n");
    let _ = writeln!(out, "| Connection | Model | First word p50 | First word p95 | Whole line p50 | Whole line p95 | Slowest | Failed | Cut off | Tokens in / out | Input read from cache |");
    let _ = writeln!(out, "|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    for s in summaries {
        let tokens = if s.input_tokens + s.output_tokens == 0 { "not reported".to_string() } else { format!("{} / {}", s.input_tokens, s.output_tokens) };
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} | {} | {} | {} of {} | {} | {} | {} |",
            s.connection, s.model, ms(s.first_p50_ms), ms(s.first_p95_ms), ms(s.total_p50_ms), ms(s.total_p95_ms), ms(s.total_max_ms), s.failures, s.lines, s.cut_off, tokens, s.cached_input_tokens
        );
    }
    for s in summaries.iter().filter(|s| !s.errors.is_empty()) {
        for (error, n) in &s.errors {
            let _ = writeln!(out, "\n{} · {}: {n}× {error}", s.connection, s.model);
        }
    }
    out
}

/// `maru bench --live --connection A --connection B=model …`: the same lines through each connection, `runs`
/// passes. With `out`, the table and every line's result are written there (the lines are yours: keep that folder
/// out of the repository).
pub async fn bench_live(cfg: &Config, paths: &Paths, targets: &[Target], sentences: &[String], runs: usize, out: Option<&Path>) -> Result<()> {
    let context_lines = cfg.continuous.context_lines;
    let mut summaries = Vec::new();
    let mut records = String::new();
    for target in targets {
        let opened = match open(cfg, paths, target).await {
            Ok(opened) => opened,
            Err(e) => {
                eprintln!("{}: {e:#}", target.name);
                summaries.push(summarize(&target.name, target.model.as_deref().unwrap_or("–"), &[LiveResult { error: Some(format!("{e:#}")), ..Default::default() }], (0, 0)));
                continue;
            }
        };
        eprintln!("{} …", opened.label);
        // One untimed line: the first connection, a cold cache or a model still loading must not skew the numbers.
        let warm = live_line(&opened.client, &sentences[0], &[]).await;
        if let Some(e) = &warm.error {
            eprintln!("  warm-up failed: {e}");
        }
        let before = opened.client.tokens_used();
        let mut results = Vec::new();
        for pass in 0..runs {
            let mut turns: Vec<(String, String)> = Vec::new();
            for (n, source) in sentences.iter().enumerate() {
                let result = live_line(&opened.client, source, &turns).await;
                if result.error.is_none() {
                    turns.push((source.clone(), result.translation.clone()));
                    if turns.len() > context_lines {
                        turns.remove(0);
                    }
                }
                let record = LiveRecord { connection: &opened.connection, model: &opened.model, pass: pass + 1, line: n + 1, source, result: result.clone() };
                let _ = writeln!(records, "{}", serde_json::to_string(&record).unwrap_or_default());
                results.push(result);
            }
            eprintln!("  pass {} of {runs} done", pass + 1);
        }
        let used = opened.client.tokens_used();
        let mut summary = summarize(&opened.connection, &opened.model, &results, (used.input - before.input, used.output - before.output));
        summary.cached_input_tokens = used.cached_input - before.cached_input;
        summaries.push(summary);
    }
    let table = table(&summaries, sentences.len(), runs, context_lines > 0);
    println!("{table}");
    if let Some(dir) = out {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        std::fs::write(dir.join("live-bench.md"), &table)?;
        std::fs::write(dir.join("live-bench.jsonl"), records)?;
        std::fs::write(dir.join("live-bench.json"), serde_json::to_string_pretty(&summaries)?)?;
        eprintln!("Written to {}", dir.display());
    }
    Ok(())
}

/// `maru dict senses`: the tasks file for `bench --cards`.
pub fn write_tasks(engine: &maru_engine::Engine, lines: &[String], out: &Path) -> Result<()> {
    let mut text = String::new();
    let (mut words, mut open) = (0, 0);
    for line in lines.iter().map(|l| l.trim()).filter(|l| !l.is_empty()) {
        let task = CardTask { line: line.to_string(), words: engine.card_words(line)? };
        words += task.words.len();
        open += questions(&task.words).len();
        let _ = writeln!(text, "{}", serde_json::to_string(&task)?);
    }
    std::fs::write(out, &text).with_context(|| format!("writing {}", out.display()))?;
    println!("{} lines, {words} words to learn, {open} of them need a model (several senses, or not in the dictionary) -> {}", text.lines().count(), out.display());
    Ok(())
}

/// `maru bench --cards tasks.jsonl --connection A --connection dictionary …`: each candidate decides every
/// line's word meanings the way the cards sheet does (one request per line for the words that need it), and
/// every decision is written down for a blind review. Nothing is scored here.
pub async fn bench_cards(cfg: &Config, paths: &Paths, targets: &[Target], tasks: &Path, out: &Path, forms: bool) -> Result<()> {
    let text = std::fs::read_to_string(tasks).with_context(|| format!("opening {}", tasks.display()))?;
    let tasks: Vec<CardTask> = text.lines().filter(|l| !l.trim().is_empty()).map(serde_json::from_str).collect::<std::result::Result<_, _>>().context("the tasks file is made by `maru dict senses`")?;
    if tasks.is_empty() {
        bail!("no lines in the tasks file");
    }
    std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    let mut table = String::from("| Candidate | Lines | Requests | Failed | Request p50 ms | Request p95 ms | Tokens in / out |\n|---|---:|---:|---:|---:|---:|---:|\n");
    for target in targets {
        let mut records = String::new();
        let (label, client, _server) = if target.name.eq_ignore_ascii_case(DICTIONARY) {
            (DICTIONARY.to_string(), None, None)
        } else {
            match open(cfg, paths, target).await {
                Ok(opened) => (opened.label, Some(opened.client), opened._server),
                Err(e) => {
                    eprintln!("{}: {e:#}", target.name);
                    let _ = writeln!(table, "| {} | 0 | 0 | could not be opened: {e:#} | – | – | – |", target.name);
                    continue;
                }
            }
        };
        eprintln!("{label} …");
        let (mut times, mut requests, mut failed, mut tokens) = (Vec::new(), 0usize, 0usize, (0u64, 0u64));
        for (n, task) in tasks.iter().enumerate() {
            let asked = questions(&task.words);
            let (mut ms, mut error, mut used) = (0, None, (0, 0));
            let mut chosen = None;
            if let (Some(client), false) = (&client, asked.is_empty()) {
                let before = client.tokens_used();
                let started = Instant::now();
                // The same two steps as the cards sheet: one request, and a second look at a word with many senses.
                let result = maru_engine::senses::choose(client, &task.line, &task.words, forms).await;
                ms = started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
                let after = client.tokens_used();
                used = (after.input - before.input, after.output - before.output);
                match result {
                    Ok(found) => {
                        times.push(ms);
                        requests += found.as_ref().map_or(1, |c| c.requests);
                        chosen = found;
                    }
                    Err(e) => {
                        requests += 1;
                        failed += 1;
                        error = Some(e.to_string().chars().take(300).collect());
                    }
                }
            }
            tokens = (tokens.0 + used.0, tokens.1 + used.1);
            let (line_requests, later, corrected) = chosen.as_ref().map(|c| (c.requests, c.later.clone(), c.corrected.clone())).unwrap_or_default();
            let words = chosen.map(|c| c.decided).unwrap_or_else(|| decide(&task.words, &[]));
            let record = CardRecord { candidate: &label, line: n + 1, asked: client.is_some() && !asked.is_empty(), ms, error, tokens_in: used.0, tokens_out: used.1, words, requests: line_requests, later, corrected };
            let _ = writeln!(records, "{}", serde_json::to_string(&record)?);
        }
        times.sort_unstable();
        let ms = |v: Option<u32>| v.map_or("–".to_string(), |v| v.to_string());
        let spent = if tokens.0 + tokens.1 == 0 { "–".to_string() } else { format!("{} / {}", tokens.0, tokens.1) };
        let _ = writeln!(table, "| {label} | {} | {requests} | {failed} | {} | {} | {spent} |", tasks.len(), ms(percentile(&times, 0.5)), ms(percentile(&times, 0.95)));
        let file: String = label.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '.' { c } else { '_' }).collect();
        std::fs::write(out.join(format!("cards-{file}.jsonl")), records)?;
    }
    println!("{table}");
    let existing = std::fs::read_to_string(out.join("cards-bench.md")).unwrap_or_default();
    // Runs for different candidates add to the same table.
    let rows: String = table.lines().skip(2).map(|l| format!("{l}\n")).collect();
    let merged = if existing.is_empty() { table } else { format!("{existing}{rows}") };
    std::fs::write(out.join("cards-bench.md"), merged)?;
    eprintln!("Written to {}", out.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use maru_core::external_servers::SavedServer;

    fn config() -> Config {
        let mut cfg = Config::default();
        cfg.llm.endpoint = Some("https://generativelanguage.googleapis.com".into());
        cfg.llm.provider = ServerProvider::Gemini;
        cfg.llm.api_key = Some("active-key".to_string().into());
        cfg.llm.model_name = "gemini-flash".into();
        let server = |name: &str, endpoint: &str, model: &str, provider, key: Option<&str>| SavedServer { name: name.into(), endpoint: endpoint.into(), model_name: model.into(), provider, api_key: key.map(|k| k.to_string().into()) };
        cfg.llm.servers = vec![
            server("NAS", "http://nas:11434", "gemma", ServerProvider::Openai, None),
            server("Azure Contoso", "https://example.openai.azure.com", "gpt-mini", ServerProvider::Azure, Some("azure-key")),
            server("Azure Two", "https://two.openai.azure.com", "gpt-mini", ServerProvider::Azure, Some("other-key")),
        ];
        cfg
    }

    #[test]
    fn a_connection_is_found_by_its_name_or_an_unambiguous_beginning_and_brings_its_own_key() {
        let cfg = config();
        let (name, llm) = resolve(&cfg, &Target::parse("nas")).unwrap();
        assert_eq!((name.as_str(), llm.endpoint.as_deref(), llm.model_name.as_str(), llm.api_key.is_none()), ("NAS", Some("http://nas:11434"), "gemma", true));
        assert_eq!(llm.provider, ServerProvider::Openai, "not the active connection's provider or key");
        let (name, llm) = resolve(&cfg, &Target::parse("azure c=gpt-big")).unwrap();
        assert_eq!((name.as_str(), llm.model_name.as_str(), llm.provider), ("Azure Contoso", "gpt-big", ServerProvider::Azure));
        assert_eq!(llm.api_key.as_ref().map(|k| k.as_str()), Some("azure-key"));
        let unclear = resolve(&cfg, &Target::parse("azure")).unwrap_err().to_string();
        assert!(unclear.contains("Azure Contoso or Azure Two"), "{unclear}");
        let missing = resolve(&cfg, &Target::parse("Claude")).unwrap_err().to_string();
        assert!(missing.contains("NAS, Azure Contoso, Azure Two") && !missing.contains("key"), "{missing}");
    }

    #[test]
    fn local_is_this_pcs_own_model_without_the_active_servers_address_or_key() {
        let (name, llm) = resolve(&config(), &Target::parse("LOCAL")).unwrap();
        assert_eq!(name, LOCAL);
        assert!(llm.endpoint.is_none() && llm.api_key.is_none() && llm.provider == ServerProvider::Openai);
        assert!(resolve(&config(), &Target::parse("local=other")).is_err());
    }

    #[test]
    fn times_are_taken_from_the_lines_that_worked_and_failures_are_counted_with_their_reasons() {
        let line = |first, total| LiveResult { first_ms: Some(first), total_ms: total, translation: "ok".into(), ..Default::default() };
        let failed = |why: &str| LiveResult { total_ms: 9000, error: Some(why.into()), ..Default::default() };
        let mut results: Vec<LiveResult> = (1..=20).map(|n| line(n * 10, n * 100)).collect();
        results.push(failed("HTTP 429: slow down"));
        results.push(failed("HTTP 429: slow down"));
        results.push(LiveResult { capped: true, ..line(50, 500) });
        let s = summarize("Gemini", "flash", &results, (1200, 340));
        assert_eq!((s.lines, s.failures, s.cut_off), (23, 2, 1));
        assert_eq!((s.first_p50_ms, s.total_p50_ms, s.total_p95_ms, s.total_max_ms), (Some(100), Some(1000), Some(1900), Some(2000)));
        assert_eq!(s.errors, [("HTTP 429: slow down".to_string(), 2)]);
        let text = table(&[s, summarize("NAS", "gemma", &[failed("unreachable")], (0, 0))], 23, 1, true);
        assert!(text.contains("| Gemini | flash | 100 | 190 | 1000 | 1900 | 2000 | 2 of 23 | 1 | 1200 / 340 | 0 |"), "{text}");
        assert!(text.contains("| NAS | gemma | – | – | – | – | – | 1 of 1 | 0 | not reported |"), "{text}");
        assert!(text.contains("NAS · gemma: 1× unreachable"));
    }
}
