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

/// Time the live-subtitle lane: each Korean sentence goes through `translate_live`, one after the other, as
/// subtitles of a cutscene would, so the earlier lines are sent to the model as context. Only the model (and the
/// network to it) is timed; there is no screen and no OCR.
pub async fn bench_live(services: &mut Services, sentences: Vec<String>, runs: usize) -> Result<()> {
    if !services.engine.has_llm() {
        bail!("no language model is configured, so there is nothing to time (set [llm] in config.toml or pass --endpoint)");
    }
    eprintln!("Waiting for the language model …");
    if let LlmStatus::Failed(why) = services.wait_for_llm().await {
        bail!("the language model did not start: {why}");
    }
    let (mut first, mut total) = (vec![], vec![]);
    let mut errors = 0;
    // One untimed line so cold-start costs (KV cache, GPU clocks, the first connection) do not skew the numbers.
    live_once(&services.engine, &sentences[0]).await;
    for _ in 0..runs {
        for s in &sentences {
            services.engine.clear_cache(); // a repeated line would otherwise be answered from the cache
            let timed = live_once(&services.engine, s).await;
            errors += usize::from(timed.error);
            if let Some(ms) = timed.first_ms {
                first.push(ms);
            }
            total.push(timed.total_ms);
        }
    }
    let model = services.engine.config().llm.endpoint.clone().map_or_else(|| "the local model".to_string(), |url| format!("the model at {url}"));
    println!("Live-subtitle latency with {model}: {} lines ({} sentences × {runs} run(s)), each sent with the lines before it as context:", sentences.len() * runs, sentences.len());
    for (name, v) in [("first word", first), ("done", total)] {
        println!("{}", summarize(name, v));
    }
    if errors > 0 {
        println!("  {errors} line(s) reported errors; the numbers above include them");
    }
    Ok(())
}

struct LiveTiming {
    first_ms: Option<u32>,
    total_ms: u32,
    error: bool,
}

async fn live_once(engine: &maru_engine::Engine, text: &str) -> LiveTiming {
    let started = std::time::Instant::now();
    let ms = |t: std::time::Instant| t.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
    let line = maru_engine::LiveLine { capture: Capture::typed(text), appeared: started, ready_ms: 0, ocr_ms: 0 };
    let (_, mut rx) = engine.translate_live(line);
    let mut out = LiveTiming { first_ms: None, total_ms: 0, error: false };
    while let Some(e) = rx.recv().await {
        match e {
            Event::Translation { .. } if out.first_ms.is_none() => out.first_ms = Some(ms(started)),
            Event::Error { .. } => out.error = true,
            Event::Done { .. } => break,
            _ => {}
        }
    }
    out.total_ms = ms(started);
    out
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

/// What a scan found, with every label update applied.
#[derive(Debug, Default)]
pub struct ScanReport {
    pub items: Vec<maru_core::ScanItem>,
    pub errors: Vec<String>,
    pub timings: Vec<(Stage, u32)>,
}

/// Read a scan's events to the end and fold the label updates into the items.
pub async fn collect_scan(mut rx: maru_engine::EventRx) -> ScanReport {
    let mut report = ScanReport::default();
    while let Some(e) = rx.recv().await {
        match e {
            Event::ScanItems { items, .. } => report.items = items,
            Event::ScanLabel { item, label, source, .. } => {
                if let Some(it) = report.items.iter_mut().find(|i| i.item == item) {
                    it.label = label;
                    it.source = source;
                }
            }
            Event::Timing { stage, ms, .. } => report.timings.push((stage, ms)),
            Event::Error { message, .. } => report.errors.push(message),
            Event::Done { .. } => break,
            _ => {}
        }
    }
    report
}

fn source_word(s: maru_core::LabelSource) -> &'static str {
    use maru_core::LabelSource::*;
    match s {
        Pending => "-",
        Dictionary => "dictionary",
        Cache => "remembered",
        Glossary => "glossary",
        Model => "model",
    }
}

/// The scan as a table, one row per piece of text, top to bottom.
pub fn format_scan(report: &ScanReport) -> String {
    let mut out = String::new();
    let clip = |s: &str, n: usize| -> String {
        let mut t: String = s.chars().take(n).collect();
        if s.chars().count() > n {
            t.push('…');
        }
        t
    };
    out.push_str(&format!("{} Korean text item(s) found\n\n", report.items.len()));
    if !report.items.is_empty() {
        out.push_str(&format!("  {:>3}  {:<19} {:<28} {:<26} {}\n", "#", "position", "text", "label", "source"));
    }
    for it in &report.items {
        let pos = format!("{:.0},{:.0} {:.0}×{:.0}", it.rect.x, it.rect.y, it.rect.w, it.rect.h);
        let known = if it.known { " (known)" } else { "" };
        out.push_str(&format!("  {:>3}  {:<19} {:<28} {:<26} {}{}\n", it.item, pos, clip(&it.text, 26), clip(&it.label, 24), source_word(it.source), known));
    }
    let ms = |stage: Stage| report.timings.iter().find(|(s, _)| *s == stage).map(|(_, ms)| *ms);
    let mut times = Vec::new();
    for (name, stage) in [("capture", Stage::Capture), ("OCR", Stage::Ocr), ("first label", Stage::FirstToken), ("total", Stage::Total)] {
        if let Some(v) = ms(stage) {
            times.push(format!("{name} {v} ms"));
        }
    }
    if !times.is_empty() {
        out.push_str(&format!("\n{}\n", times.join(" · ")));
    }
    for e in &report.errors {
        out.push_str(&format!("note: {e}\n"));
    }
    out
}

/// A copy of `image` with each found text outlined: green = labelled by the model (or remembered / glossary),
/// blue = dictionary meaning only, orange = no label yet.
pub fn draw_scan(image: &image::RgbImage, items: &[maru_core::ScanItem]) -> image::RgbImage {
    use maru_core::LabelSource::*;
    let mut out = image.clone();
    let (w, h) = (out.width() as i32, out.height() as i32);
    for it in items {
        let color = match it.source {
            Model | Cache | Glossary => image::Rgb([40, 200, 90]),
            Dictionary => image::Rgb([60, 130, 255]),
            Pending => image::Rgb([255, 150, 30]),
        };
        let (x0, y0) = (it.rect.x.floor() as i32, it.rect.y.floor() as i32);
        let (x1, y1) = ((it.rect.x + it.rect.w).ceil() as i32, (it.rect.y + it.rect.h).ceil() as i32);
        for t in 0..2 {
            for x in x0..=x1 {
                for y in [y0 + t, y1 - t] {
                    if (0..w).contains(&x) && (0..h).contains(&y) {
                        out.put_pixel(x as u32, y as u32, color);
                    }
                }
            }
            for y in y0..=y1 {
                for x in [x0 + t, x1 - t] {
                    if (0..w).contains(&x) && (0..h).contains(&y) {
                        out.put_pixel(x as u32, y as u32, color);
                    }
                }
            }
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

    if services.vision_status() != LlmStatus::Disabled {
        println!("  • OCR reader (PaddleOCR-VL): waiting for it to load …");
        match tokio::time::timeout(maru_engine::vision::READY_TIMEOUT, services.wait_for_vision()).await {
            Ok(LlmStatus::Ready) => println!("  • OCR reader (PaddleOCR-VL): ready (OCR now uses PP-OCR + PaddleOCR-VL)"),
            Ok(LlmStatus::Failed(why)) => println!("  • OCR reader (PaddleOCR-VL): FAILED — {why}; OCR is PP-OCR only"),
            Ok(other) => println!("  • OCR reader (PaddleOCR-VL): {other:?}"),
            Err(_) => println!("  • OCR reader (PaddleOCR-VL): still loading"),
        }
        debug_assert!(services.vision_server_running() || services.vision_status() != LlmStatus::Ready);
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

/// Order-independent character overlap: `(found, extra)` as fractions.
///
/// `found` is the share of the expected characters present anywhere in the output;
/// `extra` is the share of the output's characters that are not in the expected text
/// (real text the ground truth left out, noise, or misreads). Reading order is ignored,
/// so a scene with several text blocks is not punished for the order they come out in.
pub fn overlap(truth: &str, got: &str) -> (f64, f64) {
    use std::collections::HashMap;
    let count = |s: &str| {
        let mut m: HashMap<char, usize> = HashMap::new();
        for c in s.chars().filter(|c| !c.is_whitespace()) {
            *m.entry(c).or_default() += 1;
        }
        m
    };
    let (t, g) = (count(truth), count(got));
    let shared: usize = t.iter().map(|(c, n)| (*n).min(g.get(c).copied().unwrap_or(0))).sum();
    let (tn, gn): (usize, usize) = (t.values().sum(), g.values().sum());
    let found = if tn == 0 { 1.0 } else { shared as f64 / tn as f64 };
    let extra = if gn == 0 { 0.0 } else { (gn - shared) as f64 / gn as f64 };
    (found, extra)
}

/// One engine's scores over all test images.
#[derive(Debug, Clone)]
pub struct EngineScore {
    pub name: String,
    pub cer: f64,
    pub found: f64,
    pub extra: f64,
    pub p50_ms: u32,
}

/// `name.png` + `name.txt` (the true text) pairs found in `dir`, sorted by name.
pub fn load_cases(dir: &Path) -> Result<Vec<(std::path::PathBuf, std::path::PathBuf)>> {
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
    Ok(cases)
}

/// Score one engine on every case and print the per-image lines.
///
/// One untimed warm-up run comes first: the first call of any engine pays for model and
/// runtime start-up, which says nothing about how it feels in use.
pub fn eval_engine(name: &str, engine: &dyn maru_ocr::OcrEngine, cases: &[(std::path::PathBuf, std::path::PathBuf)]) -> Result<EngineScore> {
    let (mut total_cer, mut total_found, mut total_extra, mut times) = (0.0, 0.0, 0.0, Vec::new());
    println!("{name}:");
    let first = image::open(&cases[0].0)?.to_rgb8();
    engine.recognize(&first).map_err(|e| anyhow::anyhow!("{e}"))?;
    for (img_path, truth_path) in cases {
        let img = image::open(img_path)?.to_rgb8();
        let truth = std::fs::read_to_string(truth_path)?;
        let t = Instant::now();
        let out = engine.recognize(&img).map_err(|e| anyhow::anyhow!("{e}"))?;
        let ms = t.elapsed().as_millis() as u32;
        let got = out.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join(" ");
        let c = cer(&truth, &got);
        let (found, extra) = overlap(&truth, &got);
        total_cer += c;
        total_found += found;
        total_extra += extra;
        times.push(ms);
        let file = img_path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        println!("  {file:<28} CER {:>5.1}%  found {:>5.1}%  extra {:>5.1}%  {ms:>5} ms", c * 100.0, found * 100.0, extra * 100.0);
        if c > 0.0 {
            println!("      expected: {}\n      got:      {got}", truth.trim());
        }
    }
    let n = cases.len() as f64;
    println!("  mean CER {:.1}%   found {:.1}%   extra {:.1}%\n{}\n", 100.0 * total_cer / n, 100.0 * total_found / n, 100.0 * total_extra / n, summarize("latency", times.clone()));
    times.sort_unstable();
    Ok(EngineScore {
        name: name.to_string(),
        cer: total_cer / n,
        found: total_found / n,
        extra: total_extra / n,
        p50_ms: times.get(times.len() / 2).copied().unwrap_or(0),
    })
}

/// Side-by-side comparison, best (lowest CER) first.
pub fn print_scores(scores: &[EngineScore]) {
    let mut sorted = scores.to_vec();
    sorted.sort_by(|a, b| a.cer.total_cmp(&b.cer));
    println!("Summary (lower CER and extra, higher found, is better):");
    println!("  {:<34} {:>8} {:>8} {:>8} {:>9}", "engine", "CER", "found", "extra", "median");
    for s in sorted {
        println!("  {:<34} {:>7.1}% {:>7.1}% {:>7.1}% {:>6} ms", s.name, s.cer * 100.0, s.found * 100.0, s.extra * 100.0, s.p50_ms);
    }
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
    fn overlap_ignores_order_and_separates_missing_from_extra() {
        assert_eq!(overlap("가나 다라", "다라가나"), (1.0, 0.0)); // same text, different order
        let (found, extra) = overlap("가나다라", "가나 마바"); // 2 of 4 found, 2 of 4 extra
        assert!((found - 0.5).abs() < 1e-9 && (extra - 0.5).abs() < 1e-9);
        assert_eq!(overlap("", ""), (1.0, 0.0));
        assert_eq!(overlap("가", ""), (0.0, 0.0));
    }

    fn scan_item(n: u32, text: &str, label: &str, source: maru_core::LabelSource) -> maru_core::ScanItem {
        maru_core::ScanItem { item: n, rect: maru_core::Rect::new(10.0 * n as f32, 20.0, 60.0, 18.0), text: text.into(), label: label.into(), source, known: false, join: None }
    }

    #[tokio::test]
    async fn scan_events_are_folded_into_one_report() {
        use maru_core::LabelSource::*;
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        for e in [
            Event::ScanStarted { id: 1, screen: maru_core::Rect::new(0.0, 0.0, 100.0, 100.0) },
            Event::ScanItems { id: 1, items: vec![scan_item(0, "인벤토리", "", Pending), scan_item(1, "학교", "school", Dictionary)] },
            Event::ScanLabel { id: 1, item: 0, label: "Inventory".into(), source: Model },
            Event::Timing { id: 1, stage: Stage::Ocr, ms: 900 },
            Event::Error { id: 1, stage: Stage::Llm, message: "slow".into() },
            Event::Done { id: 1 },
        ] {
            tx.send(e).unwrap();
        }
        let report = collect_scan(rx).await;
        assert_eq!(report.items[0].label, "Inventory");
        assert_eq!(report.items[1].source, Dictionary);
        let text = format_scan(&report);
        assert!(text.contains("2 Korean text item(s) found") && text.contains("Inventory") && text.contains("OCR 900 ms") && text.contains("note: slow"), "{text}");
    }

    #[test]
    fn the_outline_colours_say_where_each_label_came_from() {
        use maru_core::LabelSource::*;
        let img = image::RgbImage::from_pixel(200, 60, image::Rgb([0, 0, 0]));
        let marked = draw_scan(&img, &[scan_item(1, "a", "x", Model), scan_item(5, "b", "y", Dictionary), scan_item(9, "c", "", Pending)]);
        assert_eq!(marked.get_pixel(10, 20), &image::Rgb([40, 200, 90]));
        assert_eq!(marked.get_pixel(50, 20), &image::Rgb([60, 130, 255]));
        assert_eq!(marked.get_pixel(90, 20), &image::Rgb([255, 150, 30]));
        assert_eq!(marked.get_pixel(40, 40), &image::Rgb([0, 0, 0]), "the inside of a box is left alone");
        // boxes hanging off the picture are clipped, not a panic
        let _ = draw_scan(&img, &[maru_core::ScanItem { rect: maru_core::Rect::new(190.0, 50.0, 80.0, 40.0), ..scan_item(0, "z", "", Pending) }]);
    }

    #[test]
    fn summary_reports_percentiles() {
        let line = summarize("first token", (1..=100).collect());
        assert!(line.contains("min     1") && line.contains("p50    51") && line.contains("p95    95") && line.contains("max   100"), "{line}");
        assert!(summarize("x", vec![]).contains("no data"));
    }
}
