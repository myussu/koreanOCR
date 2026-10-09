//! Diagnostics and measurement: `doctor`, `bench`, `eval-ocr`.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use maru_core::event::Stage;
use maru_core::{Capture, Event};
use maru_engine::Services;
use maru_engine::change::edit_distance;
use maru_llm::LlmStatus;

pub const KO_SENTENCES: &[&str] = &[
    "저는 한국어를 공부하고 있어요.",
    "어제 친구랑 같이 영화를 봤는데 정말 재미있었어요.",
    "내일 비가 오면 집에서 쉴 거예요.",
    "죄송하지만 길을 좀 알려 주시겠어요?",
    "이 식당은 음식이 맛있고 가격도 싸서 사람이 많아요.",
    "열심히 노력하면 언젠가 꼭 성공할 수 있을 거라고 믿어요.",
    "한국어를 배우는 것은 어렵지만 정말 재미있습니다.",
    "오늘은 날씨가 너무 더워서 아무것도 하기 싫어요.",
];
pub const EN_SENTENCES: &[&str] = &["Where is the nearest subway station?", "Thank you for your help yesterday.", "I would like to order two coffees, please."];

fn percentile(sorted: &[u32], p: f64) -> u32 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// One summary line per stage.
pub fn summarize(label: &str, mut values: Vec<u32>) -> String {
    if values.is_empty() {
        return format!("  {label:<12} no data");
    }
    values.sort_unstable();
    format!(
        "  {label:<12} min {:>5} · p50 {:>5} · p95 {:>5} · max {:>5} ms   (n={})",
        values[0],
        percentile(&values, 0.5),
        percentile(&values, 0.95),
        values[values.len() - 1],
        values.len()
    )
}

pub async fn bench(services: &mut Services, sentences: Vec<String>, runs: usize) -> Result<()> {
    if !services.engine.has_llm() {
        bail!("no language model is configured, so there is nothing to time (set [llm] in config.toml)");
    }
    eprintln!("Waiting for the language model …");
    if let LlmStatus::Failed(why) = services.wait_for_llm().await {
        bail!("the language model did not start: {why}");
    }
    let (mut analysis, mut first, mut model, mut total) = (vec![], vec![], vec![], vec![]);
    let mut errors = 0;
    // One untimed request so cold-start costs (KV cache, GPU clocks) do not skew the numbers.
    drain(&services.engine, Capture::typed(&sentences[0])).await;
    for _ in 0..runs {
        for s in &sentences {
            services.engine.clear_cache();
            let timings = drain(&services.engine, Capture::typed(s)).await;
            errors += timings.errors;
            for (stage, ms) in timings.stages {
                match stage {
                    Stage::Morphology => analysis.push(ms),
                    Stage::FirstToken => first.push(ms),
                    Stage::Llm => model.push(ms),
                    Stage::Total => total.push(ms),
                    _ => {}
                }
            }
        }
    }
    println!("Latency over {} requests ({} sentences × {runs} run(s), cache cleared each time):", sentences.len() * runs, sentences.len());
    for (name, v) in [("breakdown", analysis), ("first token", first), ("model total", model), ("everything", total)] {
        println!("{}", summarize(name, v));
    }
    if errors > 0 {
        println!("  {errors} request(s) reported errors; the numbers above include them");
    }
    Ok(())
}

struct Drained {
    stages: Vec<(Stage, u32)>,
    errors: usize,
}

async fn drain(engine: &maru_engine::Engine, capture: Capture) -> Drained {
    let (_, mut rx) = engine.translate(capture);
    let mut out = Drained { stages: vec![], errors: 0 };
    while let Some(e) = rx.recv().await {
        match e {
            Event::Timing { stage, ms, .. } => out.stages.push((stage, ms)),
            Event::Error { .. } => out.errors += 1,
            Event::Done { .. } => break,
            _ => {}
        }
    }
    out
}

pub async fn doctor(services: &mut Services, deep: bool) -> Result<()> {
    println!("Maru — environment check\n");
    println!("config:  {}", services.paths.config_file.display());
    println!("data:    {}", services.paths.data_dir.display());
    println!("models:  {}\n", services.paths.models_dir.display());
    for note in &services.notes {
        println!("  • {note}");
    }
    let engine = &services.engine;
    if let Ok(stats) = engine.vocab_stats() {
        println!("  • vocabulary: {} words ({} saved, {} known, {} due)", stats.total, stats.saved, stats.known, stats.due_now);
    }
    if engine.config().learning.anki.enabled {
        let client = maru_srs::AnkiClient::new(engine.config().learning.anki.clone());
        match client.ping().await {
            Ok(v) => println!("  • Anki: AnkiConnect v{v} reachable"),
            Err(e) => println!("  • Anki: {e}"),
        }
    }

    if engine.has_llm() {
        println!("  • language model: waiting for it to load …");
        let status = tokio::time::timeout(Duration::from_secs(engine.config().llm.ready_timeout_secs + 5), services.wait_for_llm()).await;
        match status {
            Ok(LlmStatus::Ready) => println!("  • language model: ready"),
            Ok(LlmStatus::Failed(why)) => println!("  • language model: FAILED — {why}"),
            Ok(other) => println!("  • language model: {other:?}"),
            Err(_) => println!("  • language model: still loading after the configured timeout"),
        }
        if deep && services.llm_status() == LlmStatus::Ready {
            let started = Instant::now();
            let t = drain(&services.engine, Capture::typed("안녕하세요")).await;
            println!("  • probe translation: {} ms, {} error(s)", started.elapsed().as_millis(), t.errors);
        }
    } else {
        println!("  • language model: not configured");
    }
    Ok(())
}

/// Character error rate of `got` against `truth`, ignoring whitespace.
pub fn cer(truth: &str, got: &str) -> f64 {
    let strip = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    let (t, g) = (strip(truth), strip(got));
    if t.is_empty() {
        return if g.is_empty() { 0.0 } else { 1.0 };
    }
    edit_distance(&t, &g) as f64 / t.chars().count() as f64
}

/// Score OCR engines on your own screenshots: `name.png` + `name.txt` (the true text) per case.
pub fn eval_ocr(engines: &[(String, std::sync::Arc<dyn maru_ocr::OcrEngine>)], dir: &Path) -> Result<()> {
    let mut cases = Vec::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        let is_image = path.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg" | "bmp"));
        let truth = path.with_extension("txt");
        if is_image && truth.exists() {
            cases.push((path, truth));
        }
    }
    cases.sort();
    if cases.is_empty() {
        bail!("no `name.png` + `name.txt` pairs found in {}", dir.display());
    }
    println!("{} test image(s)\n", cases.len());
    for (name, engine) in engines {
        let (mut total_cer, mut times) = (0.0, Vec::new());
        println!("{name}:");
        for (img_path, truth_path) in &cases {
            let img = image::open(img_path)?.to_rgb8();
            let truth = std::fs::read_to_string(truth_path)?;
            let t = Instant::now();
            let out = engine.recognize(&img).map_err(|e| anyhow::anyhow!("{e}"))?;
            let ms = t.elapsed().as_millis() as u32;
            let got = out.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join(" ");
            let c = cer(&truth, &got);
            total_cer += c;
            times.push(ms);
            let file = img_path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
            println!("  {file:<28} CER {:>5.1}%  {ms:>4} ms  conf {:.2}", c * 100.0, out.confidence());
            if c > 0.0 {
                println!("      expected: {}\n      got:      {got}", truth.trim());
            }
        }
        println!("  mean CER {:.1}%\n{}\n", 100.0 * total_cer / cases.len() as f64, summarize("latency", times));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cer_ignores_whitespace_and_counts_character_edits() {
        assert_eq!(cer("안녕 하세요", "안녕하세요"), 0.0);
        assert!((cer("안녕하세요", "안녕하세오") - 0.2).abs() < 1e-9);
        assert_eq!(cer("", ""), 0.0);
        assert_eq!(cer("", "x"), 1.0);
        assert_eq!(cer("abcd", ""), 1.0);
    }

    #[test]
    fn summary_reports_percentiles() {
        let line = summarize("first token", (1..=100).collect());
        assert!(line.contains("min     1") && line.contains("p50    51") && line.contains("p95    95") && line.contains("max   100"), "{line}");
        assert!(summarize("x", vec![]).contains("no data"));
    }
}
