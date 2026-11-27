//! Analyse the lines of a Kiwi review (`tools/build_kiwi_review.py`'s `tokens.jsonl`) again with dialects or typo
//! correction allowed, and write the result in the same shape, for `build_kiwi_review.py changes`.
//!
//!     cargo run --release -p maru-cli --example kiwi_options -- tokens.jsonl out-dir
//!
//! Writes `out-dir/standard.jsonl` (Kiwi as Maru runs it), `gyeongsang.jsonl` (Gyeongsang dialect allowed) and
//! `typo.jsonl` (typo correction on). Uses the Kiwi `maru setup kiwi` installed; reads nothing else and sends nothing.
use std::io::{BufRead, Write};
use std::path::PathBuf;

use maru_lang::kiwi::{KiwiAnalyzer, KiwiSetup};
use maru_lang::{AnalysisOptions, Analyzer, dialect_mask};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let (input, out) = (PathBuf::from(args.get(1).ok_or("give tokens.jsonl")?), PathBuf::from(args.get(2).ok_or("give an output folder")?));
    let paths = maru_core::Paths::discover();
    let (lib, model) = maru_engine::bootstrap::find_kiwi(&paths).ok_or("no Kiwi that passed the self-test; run `maru setup kiwi`")?;
    let gyeongsang = dialect_mask(&["gyeongsang"]).0;
    let kiwi = KiwiAnalyzer::spawn_from_paths_with(lib.clone(), model.clone(), KiwiSetup { dialects: gyeongsang, typo: true })?;
    let analyzer = Analyzer::new(std::sync::Arc::new(kiwi));
    println!("Kiwi from {}", lib.display());

    let lines: Vec<(u64, String)> = std::io::BufReader::new(std::fs::File::open(&input)?)
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(&l).ok())
        .filter_map(|v| Some((v.get("line_no")?.as_u64()?, v.get("line")?.as_str()?.to_string())))
        .collect();
    std::fs::create_dir_all(&out)?;
    for (name, options) in [
        ("standard", AnalysisOptions::default()),
        ("gyeongsang", AnalysisOptions { dialects: gyeongsang, typo: false }),
        ("typo", AnalysisOptions { dialects: 0, typo: true }),
    ] {
        let mut file = std::fs::File::create(out.join(format!("{name}.jsonl")))?;
        for (no, line) in &lines {
            let analysis = analyzer.analyze_with(line, options);
            writeln!(file, "{}", serde_json::json!({ "line_no": no, "line": line, "analyzer": analysis.analyzer, "eojeols": analysis.eojeols }))?;
        }
        println!("{name}: {} lines", lines.len());
    }
    Ok(())
}
