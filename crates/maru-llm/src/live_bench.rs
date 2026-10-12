//! Isolated, opt-in model comparison. Nothing here changes the live pipeline or its cache.

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use maru_core::Paths;
use maru_core::config::LlmConfig;
use serde::{Deserialize, Serialize};

use crate::{ChatOpts, LlamaServer, LiveRequest, LlmChunk, LlmClient, LlmError, Message, PromptContext, Result};

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    Maru,
    HyMt,
    HyMt2,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Sampling {
    #[default]
    Matched,
    Tencent,
}

impl Sampling {
    fn temperature(self) -> f32 {
        match self { Self::Matched => 0.1, Self::Tencent => 0.7 }
    }

    fn server_args(self) -> Vec<String> {
        match self {
            Self::Matched => Vec::new(),
            Self::Tencent => ["--top-p", "0.6", "--top-k", "20", "--repeat-penalty", "1.05"].into_iter().map(str::to_owned).collect(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Case {
    pub id: String,
    pub scene: String,
    pub source: String,
    /// A review aid, not an independently certified gold translation.
    pub reference: String,
    pub origin: String,
    pub tags: Vec<String>,
}

pub fn parse_cases(text: &str) -> Result<Vec<Case>> {
    let mut ids = HashSet::new();
    let mut scenes = HashSet::new();
    let mut cases: Vec<Case> = Vec::new();
    for (i, line) in text.trim_start_matches('\u{feff}').lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let case: Case = serde_json::from_str(line).map_err(|e| LlmError::Api(format!("corpus line {}: {e}", i + 1)))?;
        if [&case.id, &case.scene, &case.source, &case.reference, &case.origin].iter().any(|s| s.trim().is_empty()) {
            return Err(LlmError::Api(format!("corpus line {} has an empty required field", i + 1)));
        }
        if !ids.insert(case.id.clone()) {
            return Err(LlmError::Api(format!("duplicate corpus id: {}", case.id)));
        }
        if cases.last().is_none_or(|c| c.scene != case.scene) && !scenes.insert(case.scene.clone()) {
            return Err(LlmError::Api(format!("scene {} must occupy consecutive corpus lines", case.scene)));
        }
        cases.push(case);
    }
    if cases.is_empty() {
        return Err(LlmError::Api("the corpus has no dialogue".into()));
    }
    Ok(cases)
}

#[derive(Clone, Debug, Serialize)]
pub struct Measurement {
    pub started_unix_ms: u128,
    pub first_text_ms: Option<f64>,
    pub done_ms: f64,
    pub translation: String,
    pub streamed_text: String,
    pub error: Option<String>,
}

fn hy_messages(source: &str, turns: &[(String, String)]) -> Vec<Message> {
    // Tencent's published user-only templates; no generic assistant system instruction.
    let prompt = if turns.is_empty() {
        format!("Translate the following segment into English, without additional explanation.\n\n{source}")
    } else {
        let context = turns.iter().map(|(ko, en)| format!("Korean: {ko}\nEnglish: {en}\n")).collect::<String>();
        format!("{context}\n参考上面的信息，把下面的文本翻译成英语，注意不需要翻译上文，也不要额外解释：\n{source}")
    };
    vec![Message::user(prompt)]
}

fn hy2_messages(source: &str, turns: &[(String, String)]) -> Vec<Message> {
    // MT2 has a published English context template distinct from MT1.5's.
    let prompt = if turns.is_empty() {
        format!("Translate the following text into English. Note that you should only output the translated result without any additional explanation:\n{source}")
    } else {
        let context = turns.iter().map(|(ko, en)| format!("Korean: {ko}\nEnglish: {en}\n")).collect::<String>();
        format!("[Background Information]\n{context}\nPlease translate the following text into English, taking the provided background information into consideration.\n[Source Text]\n{source}")
    };
    vec![Message::user(prompt)]
}

pub async fn measure(client: &LlmClient, profile: Profile, source: &str, turns: &[(String, String)], timeout: Duration) -> Measurement {
    measure_with_sampling(client, profile, Sampling::Matched, source, turns, timeout).await
}

async fn measure_with_sampling(client: &LlmClient, profile: Profile, sampling: Sampling, source: &str, turns: &[(String, String)], timeout: Duration) -> Measurement {
    let started_unix_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis();
    let started = Instant::now();
    let mut first_text_ms = None;
    let mut streamed_text = String::new();
    let mut delta = |text: &str| {
        if first_text_ms.is_none() && !text.trim().is_empty() {
            first_text_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
        }
        streamed_text.push_str(text);
    };
    let request = async {
        match profile {
            Profile::Maru => {
                let ctx = PromptContext { from_ocr: true, ..Default::default() };
                client.translate_live(&LiveRequest { text: source, ctx: &ctx, turns }, &mut |chunk| {
                    if let LlmChunk::Text { delta: text, .. } = chunk {
                        delta(&text);
                    }
                }).await
            }
            Profile::HyMt | Profile::HyMt2 => {
                let opts = ChatOpts {
                    max_tokens: Some((source.chars().count() as u32 * 2 + 40).min(256)),
                    temperature: Some(sampling.temperature()),
                    log_timings: true,
                    ..Default::default()
                };
                let messages = match profile { Profile::HyMt2 => hy2_messages(source, turns), _ => hy_messages(source, turns) };
                client.stream_chat(&messages, &opts, &mut delta).await.map(|s| crate::tasks::clean_text(&s))
            }
        }
    };
    let result = tokio::time::timeout(timeout, request).await;
    let (translation, error) = match result {
        Ok(Ok(text)) if !text.is_empty() => (text, None),
        Ok(Ok(_)) => (String::new(), Some("model returned no translation".into())),
        Ok(Err(e)) => (String::new(), Some(e.to_string())),
        Err(_) => (String::new(), Some(format!("request exceeded {} seconds", timeout.as_secs_f64()))),
    };
    Measurement { started_unix_ms, first_text_ms, done_ms: started.elapsed().as_secs_f64() * 1000.0, translation, streamed_text, error }
}

#[derive(Debug, Serialize)]
pub struct Record {
    pub pass: String,
    pub run: usize,
    pub case: Case,
    pub context: Vec<(String, String)>,
    pub profile: Profile,
    pub sampling: Sampling,
    pub measurement: Measurement,
}

#[derive(Debug, Serialize)]
pub struct Distribution {
    pub n: usize,
    pub min_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
}

fn distribution(mut values: Vec<f64>) -> Option<Distribution> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let percentile = |p: f64| values[((values.len() as f64 * p).ceil() as usize).saturating_sub(1)];
    Some(Distribution { n: values.len(), min_ms: values[0], p50_ms: percentile(0.5), p95_ms: percentile(0.95), max_ms: values[values.len() - 1] })
}

#[derive(Debug, Serialize)]
pub struct PassSummary {
    pub pass: String,
    pub attempted: usize,
    pub errors: usize,
    pub first_text: Option<Distribution>,
    pub done: Option<Distribution>,
}

fn summarize(pass: &str, records: &[Record]) -> PassSummary {
    let rows: Vec<_> = records.iter().filter(|r| r.pass == pass).collect();
    let good: Vec<_> = rows.iter().filter(|r| r.measurement.error.is_none()).collect();
    PassSummary {
        pass: pass.into(), attempted: rows.len(), errors: rows.len() - good.len(),
        first_text: distribution(good.iter().filter_map(|r| r.measurement.first_text_ms).collect()),
        done: distribution(good.iter().map(|r| r.measurement.done_ms).collect()),
    }
}

pub struct Options {
    pub baseline: PathBuf,
    pub candidate: PathBuf,
    pub out: PathBuf,
    pub runs: usize,
    pub context_lines: usize,
    pub rolling_context: bool,
    pub candidate_profile: Profile,
    pub candidate_sampling: Sampling,
    pub port: u16,
    pub game_state: String,
}

#[derive(Serialize)]
struct Report<'a> {
    status: &'a str,
    limitation: &'a str,
    game_state: &'a str,
    context_mode: &'a str,
    context_lines: usize,
    runs: usize,
    temperature: f32,
    candidate_sampling: Sampling,
    token_cap: &'a str,
    config: &'a LlmConfig,
    corpus: &'a [Case],
    passes: &'a [serde_json::Value],
    summaries: &'a [PassSummary],
    failure: &'a Option<String>,
}

fn io_error(e: impl std::fmt::Display) -> LlmError {
    LlmError::Api(format!("model comparison: {e}"))
}

fn new_output(out: &Path) -> Result<std::fs::File> {
    let parent = out.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(io_error)?;
    std::fs::create_dir(out).map_err(|e| io_error(format!("use a new report folder {}: {e}", out.display())))?;
    std::fs::File::create(out.join("records.jsonl")).map_err(io_error)
}

async fn occupied(port: u16) -> bool {
    matches!(tokio::time::timeout(Duration::from_millis(500), tokio::net::TcpStream::connect(("127.0.0.1", port))).await, Ok(Ok(_)))
}

async fn wait_released(port: u16) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while occupied(port).await {
        if Instant::now() >= deadline {
            return Err(LlmError::Api(format!("test port {port} did not close; no other model will be started")));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // The socket may close just before CUDA allocations are released.
    tokio::time::sleep(Duration::from_secs(2)).await;
    Ok(())
}

fn advance(turns: &mut Vec<(String, String)>, case: &Case, measurement: &Measurement, options: &Options) {
    let answer = if options.rolling_context { &measurement.translation } else { &case.reference };
    if !answer.is_empty() {
        turns.push((case.source.clone(), answer.clone()));
    }
    let excess = turns.len().saturating_sub(options.context_lines);
    turns.drain(..excess);
}

fn cell(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('|', "&#124;").replace(['\n', '\r'], " ")
}

fn review(cases: &[Case], records: &[Record]) -> String {
    let mut text = String::from("# Live model comparison\n\nReferences are review aids, not certified answers. Judge meaning, negation, numbers, names, pronouns and extra explanations; different wording can be correct. All distinct outputs across repetitions are shown. Exact context and errors are in records.jsonl.\n\n| ID / Korean | Review aid | Gemma before | Candidate | Gemma after |\n|---|---|---|---|---|\n");
    for case in cases {
        text.push_str(&format!("| {}: {} | {}", cell(&case.id), cell(&case.source), cell(&case.reference)));
        for pass in ["baseline_before", "candidate", "baseline_after"] {
            let mut outputs = Vec::new();
            for r in records.iter().filter(|r| r.pass == pass && r.case.id == case.id) {
                let output = r.measurement.error.as_ref().map(|e| format!("ERROR: {e}")).unwrap_or_else(|| r.measurement.translation.clone());
                if !outputs.contains(&output) {
                    outputs.push(output);
                }
            }
            text.push_str(&format!(" | {}", outputs.iter().map(|s| cell(s)).collect::<Vec<_>>().join("<br>")));
        }
        text.push_str(" |\n");
    }
    text
}

/// A/B/A, sequential requests, separate warm-up and no application services or answer cache.
/// The output directory must be new. Completed records survive a later server failure.
pub async fn compare(cfg: &LlmConfig, paths: &Paths, cases: &[Case], options: &Options, progress: &mut (dyn FnMut(&str) + Send)) -> Result<Vec<PassSummary>> {
    if options.candidate_sampling == Sampling::Tencent && matches!(options.candidate_profile, Profile::Maru) {
        return Err(LlmError::Api("Tencent sampling requires an HY-MT candidate prompt".into()));
    }
    if options.runs == 0 || options.port == 0 || cases.is_empty() {
        return Err(LlmError::Api("runs, port and corpus must be nonzero".into()));
    }
    for file in [&options.baseline, &options.candidate] {
        if !file.is_file() {
            return Err(LlmError::Api(format!("local model file not found: {}", file.display())));
        }
    }
    if options.baseline.canonicalize().map_err(io_error)? == options.candidate.canonicalize().map_err(io_error)? {
        return Err(LlmError::Api("baseline and candidate must be different model files".into()));
    }
    for port in [options.port, cfg.port, 18090, 18095] {
        if occupied(port).await {
            return Err(LlmError::Api(format!("port {port} is occupied; quit Maru and its model services before this isolated test")));
        }
    }
    let mut raw = new_output(&options.out)?;
    let mut records = Vec::new();
    let mut passes = Vec::new();
    let mut failure = None;
    let mut config = cfg.clone();
    config.endpoint = None;
    config.hf_repo = None;
    config.draft_model_path = None;
    config.extra_args.clear();
    config.vision = false;
    config.port = options.port;
    let timeout = Duration::from_secs(config.request_timeout_secs.max(1));
    for (pass, model, profile) in [
        ("baseline_before", &options.baseline, Profile::Maru),
        ("candidate", &options.candidate, options.candidate_profile),
        ("baseline_after", &options.baseline, Profile::Maru),
    ] {
        let sampling = if pass == "candidate" { options.candidate_sampling } else { Sampling::Matched };
        config.extra_args = sampling.server_args();
        config.model_path = Some(model.clone());
        progress(&format!("{pass}: loading {}", model.display()));
        let start = Instant::now();
        let server = match LlamaServer::start(&config, paths, Duration::from_secs(config.ready_timeout_secs)).await {
            Ok(server) => server,
            Err(e) => { failure = Some(e.to_string()); break; }
        };
        let load_ms = start.elapsed().as_secs_f64() * 1000.0;
        std::fs::write(options.out.join(format!("{pass}-startup.log")), server.log_tail()).map_err(io_error)?;
        let client = server.client();
        let warmup = measure_with_sampling(&client, profile, sampling, "준비가 되면 시작하겠습니다.", &[], timeout).await;
        passes.push(serde_json::json!({ "pass": pass, "model": model, "model_bytes": std::fs::metadata(model).map_err(io_error)?.len(), "profile": profile, "sampling": sampling, "temperature": sampling.temperature(), "load_ms": load_ms, "warmup": warmup, "args": crate::build_args(&config, &crate::ModelSource::File(model.clone())) }));
        if let Some(error) = warmup.error {
            failure = Some(format!("{pass} warm-up failed: {error}"));
            drop(server);
            wait_released(options.port).await?;
            break;
        }
        for run in 1..=options.runs {
            let mut turns = Vec::new();
            let mut scene = "";
            for (index, case) in cases.iter().enumerate() {
                if scene != case.scene {
                    turns.clear();
                    scene = &case.scene;
                }
                let measurement = measure_with_sampling(&client, profile, sampling, &case.source, &turns, timeout).await;
                let record = Record { pass: pass.into(), run, case: case.clone(), context: turns.clone(), profile, sampling, measurement };
                advance(&mut turns, case, &record.measurement, options);
                serde_json::to_writer(&mut raw, &record).map_err(io_error)?;
                writeln!(raw).map_err(io_error)?;
                raw.flush().map_err(io_error)?;
                records.push(record);
                if (index + 1) % 8 == 0 || index + 1 == cases.len() {
                    progress(&format!("{pass}: run {run}/{}, line {}/{}", options.runs, index + 1, cases.len()));
                }
            }
        }
        std::fs::write(options.out.join(format!("{pass}-final.log")), server.log_tail()).map_err(io_error)?;
        drop(client);
        drop(server);
        if let Err(e) = wait_released(options.port).await {
            failure = Some(e.to_string());
            break;
        }
    }
    let summaries: Vec<_> = ["baseline_before", "candidate", "baseline_after"].iter().map(|p| summarize(p, &records)).collect();
    let status = if failure.is_some() { "incomplete" } else if summaries.iter().any(|s| s.errors > 0) { "completed_with_errors" } else { "completed" };
    config.model_path = None;
    let report = Report {
        status, limitation: "Model and prompt comparison only: no screen capture, OCR, overlay, vision reader or TTS. Warm-up excluded. Success-only latency summaries. No automatic quality score. The top-level temperature is the baseline/matched setting; per-pass sampling and temperature identify candidate overrides. Tencent sets temperature 0.7, top_p 0.6, top_k 20 and repeat penalty 1.05; unspecified samplers remain llama-server defaults. All profiles retain the live output cap. Draft model and application extra server arguments disabled for all passes.",
        game_state: &options.game_state, context_mode: if options.rolling_context { "rolling_model_outputs" } else { "fixed_reference_translations" },
        context_lines: options.context_lines, runs: options.runs, temperature: 0.1, candidate_sampling: options.candidate_sampling, token_cap: "min(source characters * 2 + 40, 256)",
        config: &config, corpus: cases, passes: &passes, summaries: &summaries, failure: &failure,
    };
    std::fs::write(options.out.join("report.json"), serde_json::to_vec_pretty(&report).map_err(io_error)?).map_err(io_error)?;
    std::fs::write(options.out.join("review.md"), review(cases, &records)).map_err(io_error)?;
    if let Some(e) = failure {
        return Err(io_error(format!("{e}; partial results saved in {}", options.out.display())));
    }
    Ok(summaries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{MockResponse, spawn_mock, sse_answer};

    fn case(id: &str, scene: &str) -> Case {
        Case { id: id.into(), scene: scene.into(), source: "가지 마세요.".into(), reference: "Please don't go.".into(), origin: "authored".into(), tags: vec!["negation".into()] }
    }

    fn options() -> Options {
        Options { baseline: "a.gguf".into(), candidate: "b.gguf".into(), out: "unused".into(), runs: 1, context_lines: 1, rolling_context: false, candidate_profile: Profile::HyMt, candidate_sampling: Sampling::Matched, port: 19180, game_state: "unspecified".into() }
    }

    fn measurement(error: bool) -> Measurement {
        Measurement { started_unix_ms: 0, first_text_ms: Some(10.0), done_ms: 20.0, translation: if error { String::new() } else { "Don't leave.".into() }, streamed_text: "Don't leave.".into(), error: error.then(|| "HTTP 500".into()) }
    }

    #[test]
    fn duplicate_ids_and_nonconsecutive_scenes_are_refused_before_inference() {
        let json = |c: &Case| serde_json::to_string(c).unwrap();
        assert!(parse_cases(&format!("{}\n{}", json(&case("1", "a")), json(&case("1", "a")))).is_err());
        assert!(parse_cases(&format!("{}\n{}\n{}", json(&case("1", "a")), json(&case("2", "b")), json(&case("3", "a")))).is_err());
        assert!(parse_cases("").is_err());
    }

    #[test]
    fn an_existing_folder_is_refused_without_creating_or_overwriting_records() {
        let out = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert!(new_output(out).is_err());
        assert!(!out.join("records.jsonl").exists());
    }

    #[test]
    fn fixed_context_is_identical_even_when_model_answers_or_errors_differ() {
        let mut a = Vec::new();
        let mut b = Vec::new();
        for id in ["1", "2"] {
            advance(&mut a, &case(id, "a"), &measurement(false), &options());
            advance(&mut b, &case(id, "a"), &measurement(true), &options());
        }
        assert_eq!(a, b);
        assert_eq!(a, vec![("가지 마세요.".into(), "Please don't go.".into())]);
    }

    #[test]
    fn rolling_context_uses_model_answers_and_does_not_insert_failed_answers() {
        let mut turns = Vec::new();
        let options = Options { rolling_context: true, ..options() };
        advance(&mut turns, &case("1", "a"), &measurement(false), &options);
        advance(&mut turns, &case("2", "a"), &measurement(true), &options);
        assert_eq!(turns[0].1, "Don't leave.");
        assert_eq!(turns.len(), 1);
    }

    #[test]
    fn failed_requests_never_make_the_success_latency_distribution_look_better() {
        let records = vec![false, true].into_iter().map(|error| Record { pass: "candidate".into(), run: 1, case: case("1", "a"), context: vec![], profile: Profile::HyMt, sampling: Sampling::Matched, measurement: measurement(error) }).collect::<Vec<_>>();
        let summary = summarize("candidate", &records);
        assert_eq!((summary.attempted, summary.errors), (2, 1));
        assert_eq!(summary.done.unwrap().n, 1);
        assert!(summarize("absent", &records).done.is_none());
    }

    #[tokio::test]
    async fn both_profiles_stream_first_text_and_use_the_same_live_cap_and_temperature() {
        let mock = spawn_mock(|_, _| MockResponse { status: 200, chunks: sse_answer(&[" ", "Do not", " go."]), delay_ms: 15 }).await;
        let client = LlmClient::new(&mock.url, &LlmConfig::default());
        let turns = vec![("가도 되나요?".into(), "May I go?".into())];
        for profile in [Profile::Maru, Profile::HyMt, Profile::HyMt2] {
            let result = measure(&client, profile, "가지 마세요.", &turns, Duration::from_secs(2)).await;
            assert!(result.error.is_none());
            assert_eq!(result.translation, "Do not go.");
            assert!(result.first_text_ms.unwrap() < result.done_ms);
        }
        let requests = mock.requests.lock().unwrap();
        let a: serde_json::Value = serde_json::from_str(&requests[0].1).unwrap();
        let b: serde_json::Value = serde_json::from_str(&requests[1].1).unwrap();
        assert_eq!(a["max_tokens"], b["max_tokens"]);
        assert_eq!(a["temperature"], b["temperature"]);
        assert_eq!(b["messages"].as_array().unwrap().len(), 1);
        assert_eq!(b["messages"][0]["role"], "user");
        assert!(b["messages"][0]["content"].as_str().unwrap().contains("May I go?"));
        assert!(!b["messages"][0]["content"].as_str().unwrap().contains("without additional explanation"));
    }

    #[tokio::test]
    async fn mt2_uses_its_native_prompt_and_tencent_sampling_only_when_requested() {
        let mock = spawn_mock(|_, _| MockResponse { status: 200, chunks: sse_answer(&["Don't go."]), delay_ms: 0 }).await;
        let client = LlmClient::new(&mock.url, &LlmConfig::default());
        let turns = vec![("가도 되나요?".into(), "May I go?".into())];
        for (sampling, context) in [(Sampling::Matched, &[][..]), (Sampling::Tencent, turns.as_slice())] {
            let result = measure_with_sampling(&client, Profile::HyMt2, sampling, "가지 마세요.", context, Duration::from_secs(2)).await;
            assert!(result.error.is_none());
        }
        let requests = mock.requests.lock().unwrap();
        let bodies: Vec<serde_json::Value> = requests.iter().map(|r| serde_json::from_str(&r.1).unwrap()).collect();
        assert_eq!(bodies[0]["temperature"], serde_json::json!(0.1_f32));
        assert_eq!(bodies[1]["temperature"], serde_json::json!(0.7_f32));
        assert_eq!(bodies[0]["max_tokens"], bodies[1]["max_tokens"]);
        assert_eq!(bodies[1]["messages"].as_array().unwrap().len(), 1);
        assert!(bodies[0]["messages"][0]["content"].as_str().unwrap().starts_with("Translate the following text into English."));
        let prompt = bodies[1]["messages"][0]["content"].as_str().unwrap();
        assert!(prompt.starts_with("[Background Information]\n"));
        assert!(prompt.contains("English: May I go?"));
        assert!(prompt.ends_with("[Source Text]\n가지 마세요."));
        assert!(Sampling::Matched.server_args().is_empty());
        assert_eq!(Sampling::Tencent.server_args(), ["--top-p", "0.6", "--top-k", "20", "--repeat-penalty", "1.05"]);
    }

    #[tokio::test]
    async fn tencent_sampling_cannot_silently_change_the_existing_maru_live_task() {
        let options = Options { candidate_profile: Profile::Maru, candidate_sampling: Sampling::Tencent, ..options() };
        let result = compare(&LlmConfig::default(), &Paths::under(Path::new("unused")), &[case("1", "a")], &options, &mut |_| {}).await;
        assert!(result.unwrap_err().to_string().contains("requires an HY-MT"));
    }

    #[tokio::test]
    async fn empty_or_failed_or_stalled_output_is_preserved_as_an_error() {
        for status in [200, 500] {
            let mock = spawn_mock(move |_, _| MockResponse { status, chunks: sse_answer(&[" "]), delay_ms: 0 }).await;
            let result = measure(&LlmClient::new(&mock.url, &LlmConfig::default()), Profile::HyMt, "네.", &[], Duration::from_secs(1)).await;
            assert!(result.error.is_some());
            assert!(result.first_text_ms.is_none());
        }
        let mock = spawn_mock(|_, _| MockResponse { status: 200, chunks: sse_answer(&["late"]), delay_ms: 100 }).await;
        let result = measure(&LlmClient::new(&mock.url, &LlmConfig::default()), Profile::HyMt, "네.", &[], Duration::from_millis(10)).await;
        assert!(result.error.unwrap().contains("exceeded"));
    }

    #[test]
    fn review_keeps_distinct_answers_and_escapes_table_and_html_characters() {
        assert_eq!(cell("<a>|b\nc"), "&lt;a&gt;&#124;b c");
        let records = [false, true].into_iter().map(|error| Record { pass: "candidate".into(), run: 1, case: case("1", "a"), context: vec![], profile: Profile::HyMt, sampling: Sampling::Matched, measurement: measurement(error) }).collect::<Vec<_>>();
        let text = review(&[case("1", "a")], &records);
        assert!(text.contains("Don't leave.<br>ERROR: HTTP 500"));
    }
}
