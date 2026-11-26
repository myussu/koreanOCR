//! Scoring OCR on a labelled test set (`maru eval-ocr`).
//!
//! What it measures, and why it is built this way:
//! * **Letter error rate**: edit distance on letters and digits only. Spaces and punctuation are not what a translation depends on, and
//!   a missing comma used to count as a reading error. The old whitespace-only rate is still reported as "with punctuation".
//! * **Distinct subtitles**: several frames of one subtitle share a label and are not independent tests, so the headline numbers average
//!   over distinct subtitles (a subtitle's score is the mean over its frames) and the comparison counts subtitles, not pictures.
//! * **Groups**: by game (the folder), by the tags a person put on a picture (`name.tags`), and by a dev / held-out split that keeps a
//!   whole scene on one side, so a change tuned on one half is judged on the other.
//! * **Comparison**: two runs (two engines, or the same engine before and after a change) are compared subtitle by subtitle: how many got
//!   better, how many worse, and a sign test on that, because a lower average alone can be a few lucky lines.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use crate::tools::{cer, overlap};

/// Which half of the test set a picture is in. Whole scenes stay together (see [`scene_key`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Split {
    Dev,
    Held,
}

impl Split {
    pub fn name(self) -> &'static str {
        match self {
            Split::Dev => "dev",
            Split::Held => "held-out",
        }
    }
}

/// A picture with its true text and what is known about it.
#[derive(Debug, Clone)]
pub struct CaseInfo {
    pub img: PathBuf,
    /// The path below the test set's folder, with `/` separators (the same on every PC).
    pub rel: String,
    /// The folder the picture is in, one per game ("" when pictures sit directly in the test folder).
    pub game: String,
    pub tags: Vec<String>,
    pub split: Split,
    pub truth: String,
}

/// What one engine made of one picture.
#[derive(Debug, Clone)]
pub struct CaseResult {
    pub rel: String,
    pub game: String,
    pub tags: Vec<String>,
    pub split: Split,
    /// The same subtitle seen in several pictures has the same unit.
    pub unit: String,
    pub truth: String,
    pub got: String,
    /// Letters and digits only.
    pub cer: f64,
    /// Spaces ignored, punctuation counted (the rate earlier versions reported).
    pub cer_strict: f64,
    pub exact: bool,
    /// The picture has text and nothing was read.
    pub missed: bool,
    pub found: f64,
    pub extra: f64,
    /// Digits of the true text that were not read.
    pub digit_errors: usize,
    /// A negation word is in one of the two texts and not the other: the meaning would flip.
    pub negation_flip: bool,
    pub ms: u32,
}

/// Letters and digits of `s`, in order: what a reading is judged on.
pub fn letters(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).collect()
}

/// Edit distance on letters and digits divided by the true text's length (1 if the true text is empty and something was read).
pub fn letter_error(truth: &str, got: &str) -> f64 {
    let (t, g) = (letters(truth), letters(got));
    if t.is_empty() {
        return if g.is_empty() { 0.0 } else { 1.0 };
    }
    maru_engine::change::edit_distance(&t, &g) as f64 / t.chars().count() as f64
}

/// Digits of `truth` that `got` does not have (counting repeats).
fn digit_errors(truth: &str, got: &str) -> usize {
    let mut have: HashMap<char, usize> = HashMap::new();
    for c in got.chars().filter(char::is_ascii_digit) {
        *have.entry(c).or_default() += 1;
    }
    let mut missing = 0;
    for c in truth.chars().filter(char::is_ascii_digit) {
        match have.get_mut(&c) {
            Some(n) if *n > 0 => *n -= 1,
            _ => missing += 1,
        }
    }
    missing
}

/// A rough test for "this line says no": 않 and 없 and 아니 anywhere, or 안 or 못 standing alone. A heuristic, for counting
/// readings that would turn "I did not" into "I did", not a parser.
pub fn has_negation(text: &str) -> bool {
    text.contains('않') || text.contains('없') || text.contains("아니") || text.split_whitespace().any(|w| w.trim_matches(|c: char| !c.is_alphanumeric()) == "안" || w.trim_matches(|c: char| !c.is_alphanumeric()) == "못")
}

/// Score one reading of one picture.
pub fn score_case(info: &CaseInfo, got: &str, ms: u32) -> CaseResult {
    let (found, extra) = overlap(&info.truth, got);
    let truth_letters = letters(&info.truth);
    CaseResult {
        rel: info.rel.clone(),
        game: info.game.clone(),
        tags: info.tags.clone(),
        split: info.split,
        unit: format!("{}/{}", info.game, truth_letters),
        truth: info.truth.trim().to_string(),
        got: got.trim().to_string(),
        cer: letter_error(&info.truth, got),
        cer_strict: cer(&info.truth, got),
        exact: truth_letters == letters(got),
        missed: !truth_letters.is_empty() && letters(got).is_empty(),
        found,
        extra,
        digit_errors: digit_errors(&info.truth, got),
        negation_flip: has_negation(&info.truth) != has_negation(got),
        ms,
    }
}

/// A scene is a run of pictures that should not be split between dev and held-out: pictures saved within five minutes of each other
/// (`<unix seconds>-<n>.png`, as `save_reads` names them), or frames of one subtitle (`s012_3.png` is scene `s012`).
pub fn scene_key(game: &str, file_stem: &str) -> String {
    if let Some((secs, rest)) = file_stem.split_once('-')
        && secs.len() >= 9
        && secs.chars().all(|c| c.is_ascii_digit())
        && rest.chars().all(|c| c.is_ascii_digit())
        && let Ok(secs) = secs.parse::<u64>()
    {
        return format!("{game}/{}", secs / 300);
    }
    format!("{game}/{}", file_stem.split('_').next().unwrap_or(file_stem))
}

/// Dev or held-out, decided by the scene alone so it never changes between runs or PCs.
pub fn split_of(scene: &str) -> Split {
    // FNV-1a: tiny, fixed forever (unlike `DefaultHasher`, which may change between Rust versions).
    let mut h: u64 = 0xcbf29ce484222325;
    for b in scene.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    if (h >> 33) & 1 == 0 { Split::Dev } else { Split::Held }
}

/// Everything known about a labelled picture.
pub fn case_info(root: &Path, img: &Path, truth_path: &Path) -> std::io::Result<CaseInfo> {
    let rel_path = img.strip_prefix(root).unwrap_or(img);
    let rel = rel_path.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
    let game = rel_path.parent().map(|p| p.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/")).unwrap_or_default();
    let stem = img.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let tags = std::fs::read_to_string(img.with_extension("tags"))
        .map(|t| t.split(',').map(|x| x.trim().to_lowercase()).filter(|x| !x.is_empty()).collect())
        .unwrap_or_default();
    Ok(CaseInfo { img: img.to_path_buf(), rel, split: split_of(&scene_key(&game, &stem)), game, tags, truth: std::fs::read_to_string(truth_path)? })
}

/// Averages over a set of results. Rates are means over distinct subtitles.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Summary {
    pub pictures: usize,
    pub subtitles: usize,
    pub cer: f64,
    pub cer_strict: f64,
    pub exact: f64,
    pub missed: usize,
    pub found: f64,
    pub extra: f64,
    pub digit_errors: usize,
    pub negation_flips: usize,
    pub p50_ms: u32,
}

/// Mean per distinct subtitle (a subtitle's value is the mean over its pictures).
fn mean_by_unit(results: &[&CaseResult], value: impl Fn(&CaseResult) -> f64) -> f64 {
    let mut units: BTreeMap<&str, (f64, usize)> = BTreeMap::new();
    for r in results {
        let e = units.entry(r.unit.as_str()).or_default();
        e.0 += value(r);
        e.1 += 1;
    }
    if units.is_empty() { 0.0 } else { units.values().map(|(sum, n)| sum / *n as f64).sum::<f64>() / units.len() as f64 }
}

pub fn summarize(results: &[&CaseResult]) -> Summary {
    let mut times: Vec<u32> = results.iter().map(|r| r.ms).collect();
    times.sort_unstable();
    Summary {
        pictures: results.len(),
        subtitles: results.iter().map(|r| r.unit.as_str()).collect::<std::collections::BTreeSet<_>>().len(),
        cer: mean_by_unit(results, |r| r.cer),
        cer_strict: mean_by_unit(results, |r| r.cer_strict),
        exact: mean_by_unit(results, |r| f64::from(u8::from(r.exact))),
        missed: results.iter().filter(|r| r.missed).count(),
        found: mean_by_unit(results, |r| r.found),
        extra: mean_by_unit(results, |r| r.extra),
        digit_errors: results.iter().map(|r| r.digit_errors).sum(),
        negation_flips: results.iter().filter(|r| r.negation_flip).count(),
        p50_ms: times.get(times.len() / 2).copied().unwrap_or(0),
    }
}

/// `(group name, summary)` for every game, tag and split present, each under its heading.
pub fn grouped(results: &[CaseResult]) -> Vec<(&'static str, Vec<(String, Summary)>)> {
    type KeyFn = fn(&CaseResult) -> Vec<String>;
    let kinds: [(&'static str, KeyFn); 3] = [
        ("game", |r| vec![if r.game.is_empty() { "(top folder)".to_string() } else { r.game.clone() }]),
        ("tag", |r| r.tags.clone()),
        ("split", |r| vec![r.split.name().to_string()]),
    ];
    kinds
        .into_iter()
        .map(|(kind, key)| {
            let mut by: BTreeMap<String, Vec<&CaseResult>> = BTreeMap::new();
            for r in results {
                for k in key(r) {
                    by.entry(k).or_default().push(r);
                }
            }
            (kind, by.into_iter().map(|(k, rs)| (k, summarize(&rs))).collect())
        })
        .collect()
}

/// How a second run compares with a first, subtitle by subtitle.
#[derive(Debug, Clone, PartialEq)]
pub struct Comparison {
    pub better: usize,
    pub worse: usize,
    pub same: usize,
    /// Subtitles only one of the runs has (the labels changed in between, or pictures were added).
    pub not_in_both: usize,
    /// Two-sided sign test over the subtitles that changed: how likely a split this lopsided is if neither run were really better.
    pub p: f64,
    /// The subtitles that got worse, largest increase first, and the ones that got better: `(unit, error before, error after, text)`.
    pub worst: Vec<(String, f64, f64, String)>,
    pub best: Vec<(String, f64, f64, String)>,
}

fn per_unit(results: &[CaseResult]) -> BTreeMap<String, (f64, String)> {
    let mut sums: BTreeMap<String, (f64, usize, String)> = BTreeMap::new();
    for r in results {
        let e = sums.entry(r.unit.clone()).or_insert((0.0, 0, r.truth.clone()));
        e.0 += r.cer;
        e.1 += 1;
    }
    sums.into_iter().map(|(k, (s, n, t))| (k, (s / n as f64, t))).collect()
}

/// Two-sided sign test: `2 * P(X <= min(better, worse))` for `X ~ Binomial(better + worse, 1/2)`, at most 1.
pub fn sign_test(better: usize, worse: usize) -> f64 {
    let n = better + worse;
    if n == 0 {
        return 1.0;
    }
    let k = better.min(worse);
    // pmf(0) = 0.5^n; pmf(i + 1) = pmf(i) * (n - i) / (i + 1). Done in logs so a few hundred subtitles do not underflow.
    let mut log_pmf = -(n as f64) * std::f64::consts::LN_2;
    let mut tail = log_pmf.exp();
    for i in 0..k {
        log_pmf += ((n - i) as f64).ln() - ((i + 1) as f64).ln();
        tail += log_pmf.exp();
    }
    (2.0 * tail).min(1.0)
}

/// Compare `after` with `before`. A subtitle is better when its error went down by more than a hair.
pub fn compare(before: &[CaseResult], after: &[CaseResult]) -> Comparison {
    const HAIR: f64 = 1e-9;
    let (a, b) = (per_unit(before), per_unit(after));
    let (mut better, mut worse, mut same) = (0, 0, 0);
    let (mut worst, mut best) = (Vec::new(), Vec::new());
    for (unit, (was, text)) in &a {
        let Some((now, _)) = b.get(unit) else { continue };
        if *now < was - HAIR {
            better += 1;
            best.push((unit.clone(), *was, *now, text.clone()));
        } else if *now > was + HAIR {
            worse += 1;
            worst.push((unit.clone(), *was, *now, text.clone()));
        } else {
            same += 1;
        }
    }
    worst.sort_by(|x, y| (y.2 - y.1).total_cmp(&(x.2 - x.1)));
    best.sort_by(|x, y| (y.1 - y.2).total_cmp(&(x.1 - x.2)));
    let not_in_both = a.keys().filter(|k| !b.contains_key(*k)).count() + b.keys().filter(|k| !a.contains_key(*k)).count();
    Comparison { better, worse, same, not_in_both, p: sign_test(better, worse), worst, best }
}

// ───────────── saved reports ─────────────

/// One engine's per-picture results as JSON, for comparing a later run with this one.
pub fn to_json(name: &str, scale: f32, results: &[CaseResult]) -> serde_json::Value {
    serde_json::json!({
        "engine": name,
        "scale": scale,
        "pictures": results.iter().map(|r| serde_json::json!({
            "rel": r.rel, "game": r.game, "tags": r.tags, "split": r.split.name(), "unit": r.unit,
            "truth": r.truth, "got": r.got, "cer": r.cer, "cer_strict": r.cer_strict, "exact": r.exact, "ms": r.ms,
        })).collect::<Vec<_>>(),
    })
}

/// The results in a report written by [`to_json`] (only what [`compare`] needs).
pub fn from_json(value: &serde_json::Value) -> Option<(String, Vec<CaseResult>)> {
    let name = value.get("engine")?.as_str()?.to_string();
    let rows = value.get("pictures")?.as_array()?;
    let results = rows
        .iter()
        .filter_map(|r| {
            let text = |k: &str| r.get(k).and_then(|v| v.as_str()).map(str::to_string);
            Some(CaseResult {
                rel: text("rel")?,
                game: text("game")?,
                tags: r.get("tags").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|t| t.as_str().map(str::to_string)).collect()).unwrap_or_default(),
                split: if text("split").as_deref() == Some("held-out") { Split::Held } else { Split::Dev },
                unit: text("unit")?,
                truth: text("truth")?,
                got: text("got")?,
                cer: r.get("cer")?.as_f64()?,
                cer_strict: r.get("cer_strict").and_then(|v| v.as_f64()).unwrap_or(0.0),
                exact: r.get("exact").and_then(|v| v.as_bool()).unwrap_or(false),
                missed: false,
                found: 0.0,
                extra: 0.0,
                digit_errors: 0,
                negation_flip: false,
                ms: r.get("ms").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
            })
        })
        .collect();
    Some((name, results))
}

// ───────────── printing ─────────────

fn pct(x: f64) -> String {
    format!("{:.1}%", x * 100.0)
}

/// The headline lines for one engine.
pub fn print_summary(results: &[CaseResult]) {
    let all: Vec<&CaseResult> = results.iter().collect();
    let s = summarize(&all);
    println!(
        "  {} pictures, {} distinct subtitles\n  letter error {}   (with punctuation {})   exact lines {}   missed {}   digit errors {}   meaning flips (negation) {}   median {} ms",
        s.pictures,
        s.subtitles,
        pct(s.cer),
        pct(s.cer_strict),
        pct(s.exact),
        s.missed,
        s.digit_errors,
        s.negation_flips,
        s.p50_ms
    );
    if s.subtitles < 30 {
        println!("  (only {} distinct subtitles: a difference of a line or two between two runs means nothing)", s.subtitles);
    }
    for (kind, rows) in grouped(results) {
        if rows.is_empty() || (kind == "game" && rows.len() == 1 && rows[0].0 == "(top folder)") {
            continue;
        }
        let cells: Vec<String> = rows.iter().map(|(k, g)| format!("{k} {} ({})", pct(g.cer), g.subtitles)).collect();
        println!("  by {kind}: {}", cells.join(" · "));
    }
}

/// How `after` compares with `before`, with the lines that moved most.
pub fn print_comparison(before_name: &str, after_name: &str, c: &Comparison) {
    println!("{after_name} against {before_name}, subtitle by subtitle:");
    println!("  better on {}, worse on {}, the same on {}   (sign test p = {:.3}{})", c.better, c.worse, c.same, c.p, if c.p < 0.05 { ": unlikely to be chance" } else { ": could easily be chance" });
    if c.not_in_both > 0 {
        println!("  {} subtitle(s) are in only one of the two runs and were left out", c.not_in_both);
    }
    for (label, rows) in [("got worse", &c.worst), ("got better", &c.best)] {
        for (unit, was, now, text) in rows.iter().take(5) {
            println!("  {label}: {} → {}   {} ({})", pct(*was), pct(*now), text.chars().take(40).collect::<String>(), unit.split('/').next().unwrap_or(""));
        }
    }
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(game: &str, truth: &str) -> CaseInfo {
        CaseInfo { img: PathBuf::from("x.png"), rel: format!("{game}/x.png"), game: game.into(), tags: vec![], split: Split::Dev, truth: truth.into() }
    }

    fn result(game: &str, truth: &str, got: &str) -> CaseResult {
        score_case(&info(game, truth), got, 100)
    }

    #[test]
    fn punctuation_and_spaces_are_not_reading_errors_but_a_wrong_letter_is() {
        let r = result("g", "오, 하고 있구나", "오하고 있구나");
        assert_eq!(r.cer, 0.0);
        assert!(r.exact);
        assert!(r.cer_strict > 0.0, "the old rate still sees the missing comma");
        let r = result("g", "저게 뭐야?", "저게 뮈야");
        assert!((r.cer - 0.25).abs() < 1e-9, "one of four letters is wrong: {}", r.cer);
        assert!(!r.exact);
    }

    #[test]
    fn nothing_read_is_a_miss_and_a_picture_with_no_text_read_as_text_is_all_junk() {
        let r = result("g", "안녕하세요", "");
        assert!(r.missed && r.cer == 1.0);
        let r = result("g", "", "잡음");
        assert!(!r.missed && r.cer == 1.0);
        assert!(!result("g", "", "").missed);
    }

    #[test]
    fn lost_numbers_and_flipped_negations_are_counted_on_their_own() {
        assert_eq!(result("g", "3000골드 필요해", "300골드 필요해").digit_errors, 1);
        assert_eq!(result("g", "12 13", "13 12").digit_errors, 0);
        assert!(result("g", "가지 않았어", "가지 았어").negation_flip);
        assert!(result("g", "난 안 해", "난 해").negation_flip);
        assert!(!result("g", "안녕하세요", "안녕하세요").negation_flip, "안녕 is not a negation");
        assert!(result("g", "못 가", "못 가").exact && !result("g", "못 가", "못 가").negation_flip);
    }

    #[test]
    fn a_scene_stays_on_one_side_of_the_split() {
        assert_eq!(scene_key("yuzu", "1760000100-001"), scene_key("yuzu", "1760000350-099"), "within the same five minutes");
        assert_ne!(scene_key("yuzu", "1760000100-001"), scene_key("yuzu", "1760009000-001"));
        assert_eq!(scene_key("g", "s012_3"), "g/s012");
        assert_eq!(scene_key("g", "s012_0"), scene_key("g", "s012_3"));
        for stem in ["s012_0", "s012_1", "s012_2"] {
            assert_eq!(split_of(&scene_key("g", stem)), split_of("g/s012"));
        }
        // fixed forever, or an old baseline would be judged on different lines than a new run
        assert_eq!(split_of("g/s012"), split_of("g/s012"));
        let sides: Vec<Split> = (0..40).map(|i| split_of(&format!("game/{i}"))).collect();
        assert!(sides.contains(&Split::Dev) && sides.contains(&Split::Held), "both halves get scenes");
    }

    #[test]
    fn frames_of_one_subtitle_count_once_in_the_averages() {
        // one subtitle seen in four pictures (all wrong), another in one picture (right): 50%, not 80%
        let mut rs: Vec<CaseResult> = (0..4).map(|_| result("g", "가나다라", "가나다마")).collect();
        rs.push(result("g", "마바사아", "마바사아"));
        let all: Vec<&CaseResult> = rs.iter().collect();
        let s = summarize(&all);
        assert_eq!((s.pictures, s.subtitles), (5, 2));
        assert!((s.cer - 0.125).abs() < 1e-9, "{}", s.cer);
        assert!((s.exact - 0.5).abs() < 1e-9);
    }

    #[test]
    fn groups_are_listed_by_game_tag_and_split() {
        let mut a = result("yuzu", "가나다라", "가나다라");
        a.tags = vec!["outlined".into(), "busy".into()];
        let b = result("other", "마바사아", "마바사자");
        let g = grouped(&[a, b]);
        let games = &g[0].1;
        assert_eq!(games.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["other", "yuzu"]);
        assert!((games[0].1.cer - 0.25).abs() < 1e-9);
        let tags = &g[1].1;
        assert_eq!(tags.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["busy", "outlined"], "a picture with two tags is in both");
    }

    #[test]
    fn the_sign_test_needs_a_lopsided_split_to_call_it_real() {
        assert_eq!(sign_test(0, 0), 1.0);
        assert!((sign_test(5, 5) - 1.0).abs() < 1e-9);
        assert!((sign_test(8, 2) - 0.109375).abs() < 1e-6, "8 to 2 is not yet convincing: {}", sign_test(8, 2));
        assert!(sign_test(20, 3) < 0.001);
        assert!((sign_test(3, 20) - sign_test(20, 3)).abs() < 1e-12, "it does not matter which run is which");
        assert!(sign_test(300, 250) > 0.01, "large counts do not underflow or overflow");
    }

    #[test]
    fn two_runs_are_compared_subtitle_by_subtitle() {
        let before = vec![result("g", "가나다라", "가나다마"), result("g", "마바사아", "마바사아"), result("g", "자차카타", "자차"), result("g", "파하거너", "파하거너")];
        let after = vec![result("g", "가나다라", "가나다라"), result("g", "마바사아", "마바사자"), result("g", "자차카타", "자차"), result("g", "더러머버", "더러머버")];
        let c = compare(&before, &after);
        assert_eq!((c.better, c.worse, c.same), (1, 1, 1));
        assert_eq!(c.not_in_both, 2, "one subtitle only in each run");
        assert_eq!(c.worst[0].3, "마바사아");
        assert_eq!(c.best[0].3, "가나다라");
        assert!(c.p > 0.9);
    }

    #[test]
    fn a_saved_report_compares_the_same_as_the_run_it_came_from() {
        let rs = vec![result("g", "가나다라", "가나다마"), result("h", "마바사아", "마바사아")];
        let (name, back) = from_json(&to_json("PP-OCR", 1.0, &rs)).unwrap();
        assert_eq!(name, "PP-OCR");
        assert_eq!(back.len(), 2);
        let c = compare(&back, &rs);
        assert_eq!((c.better, c.worse, c.same, c.not_in_both), (0, 0, 2, 0));
        assert!(from_json(&serde_json::json!({"nothing": true})).is_none());
    }

    #[test]
    fn case_info_reads_the_game_the_tags_and_the_text_from_the_files() {
        let root = std::env::temp_dir().join(format!("maru-info-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("yuzu")).unwrap();
        let img = root.join("yuzu/1760000000-001.png");
        std::fs::write(&img, b"").unwrap();
        std::fs::write(root.join("yuzu/1760000000-001.txt"), "안녕\n").unwrap();
        std::fs::write(root.join("yuzu/1760000000-001.tags"), "Outlined, busy\n").unwrap();
        let i = case_info(&root, &img, &img.with_extension("txt")).unwrap();
        assert_eq!((i.rel.as_str(), i.game.as_str()), ("yuzu/1760000000-001.png", "yuzu"));
        assert_eq!(i.tags, ["outlined", "busy"]);
        assert_eq!(i.truth.trim(), "안녕");
        let top = root.join("loose.png");
        std::fs::write(&top, b"").unwrap();
        std::fs::write(root.join("loose.txt"), "가").unwrap();
        assert_eq!(case_info(&root, &top, &top.with_extension("txt")).unwrap().game, "");
        let _ = std::fs::remove_dir_all(root);
    }
}
