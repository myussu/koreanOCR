//! The whole pipeline, against a scripted stand-in for llama-server.
#[path = "support/reading_areas.rs"]
mod reading_areas;

use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

use maru_core::config::{AppProfile, Config};
use maru_core::event::{DeepSection, Side, Stage, Variant};
use maru_core::{AppInfo, Capture, CaptureSource, Direction, Event, Register, WordState};
use maru_dict::Dictionary;
use maru_engine::{DeepDiveRequest, Engine, EngineParts, EventRx};
use maru_lang::Analyzer;
use maru_llm::LlmClient;
use maru_llm::testing::{MockResponse, MockServer, sse_answer, spawn_mock};
use maru_platform::Platform;
use maru_platform::testing::{FakeScreen, FakeSelection};
use maru_srs::{SrsConfig, Vocab};

const KAIKKI: &str = r#"
{"word":"학교","lang_code":"ko","pos":"noun","senses":[{"glosses":["school"]}]}
{"word":"공부하다","lang_code":"ko","pos":"verb","senses":[{"glosses":["to study; to learn"]}]}
{"word":"저","lang_code":"ko","pos":"pronoun","senses":[{"glosses":["I (humble)"]}]}
"#;

/// Answers like a model would, keyed on what the request asks for.
fn model(_path: &str, body: &str) -> MockResponse {
    let answer: Vec<&str> = if body.contains("Words:") {
        vec!["0|I|\n1|school|\n2|to study|past tense\n", "NOTE|Topic marker 는 sets the subject.\n"]
    } else if body.contains("Translate into English") {
        vec!["I studied ", "at school."]
    } else if body.contains("Translate into Korean") {
        vec!["POLITE: 감사", "합니다\nFORMAL: 감사드립니다\nCASUAL: 고마워\nNOTE: Use CASUAL only with friends.\n"]
    } else if body.contains("Word:") {
        vec!["USAGE: Means to study.\n", "EXAMPLES:\n- 공부해요 — I study.\n", "NUANCE: Unlike 배우다, it is self-study.\n"]
    } else {
        vec!["?"]
    };
    MockResponse { status: 200, chunks: sse_answer(&answer), delay_ms: 1 }
}

struct Harness {
    engine: Engine,
    vocab: Arc<Vocab>,
    server: Option<MockServer>,
}

impl Harness {
    fn requests(&self) -> Vec<String> {
        self.server.as_ref().unwrap().requests.lock().unwrap().iter().map(|(_, b)| b.clone()).collect()
    }
}

async fn harness_with(handler: impl Fn(&str, &str) -> MockResponse + Send + Sync + 'static, tweak: impl FnOnce(&mut Config)) -> Harness {
    let server = spawn_mock(handler).await;
    let mut cfg = Config::default();
    tweak(&mut cfg);
    let llm = LlmClient::new(&server.url, &cfg.llm);
    build(cfg, Some(llm), Some(server))
}

async fn harness() -> Harness {
    harness_with(model, |_| {}).await
}

#[tokio::test]
async fn failed_or_missing_requested_korean_registers_end_with_done_and_are_not_cached() {
    for failed in [false, true] {
        let h = harness_with(move |_, _| {
            if failed { MockResponse { status: 503, chunks: vec![b"unavailable".to_vec()], delay_ms: 0 } }
            else { MockResponse { status: 200, chunks: sse_answer(&["FORMAL: 감사합니다\n"]), delay_ms: 0 } }
        }, |_| {}).await;
        for _ in 0..2 {
            let (_, rx) = h.engine.translate(Capture::typed("Thank you."));
            let events = tokio::time::timeout(Duration::from_secs(1), collect(rx)).await.expect("a finished translation releases its follow-up");
            assert!(matches!(events.last(), Some(Event::Done { .. })));
            assert!(events.iter().any(|e| matches!(e, Event::Error { .. })));
        }
        assert_eq!(h.requests().len(), 2);
        assert!(h.engine.history().is_empty());
    }
}

#[tokio::test]
async fn an_unfinished_translation_is_not_added_to_cache_or_history() {
    for tail in ["", "data: broken\n\ndata: [DONE]\n\n", "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"length\"}]}\n\ndata: [DONE]\n\n", "data: {\"error\":\"failed at EOF\"}"] {
        let h = harness_with(move |path, body| {
            if body.contains("Words:") { model(path, body) }
            else { MockResponse { status: 200, chunks: vec![maru_llm::testing::sse_event("Partial."), tail.as_bytes().to_vec()], delay_ms: 0 } }
        }, |_| {}).await;
        for _ in 0..2 {
            let events = collect(h.engine.translate(Capture::typed(SENTENCE)).1).await;
            assert!(events.iter().any(|e| matches!(e, Event::Error { .. })));
            assert!(!events.iter().any(|e| matches!(e, Event::TranslationDone { .. })));
            assert!(matches!(events.last(), Some(Event::Done { .. })));
        }
        assert_eq!(h.requests().iter().filter(|b| !b.contains("Words:")).count(), 2);
        assert!(h.engine.history().is_empty());
    }
}

#[tokio::test]
async fn a_korean_register_closed_before_a_later_stream_error_is_not_cached() {
    let h = harness_with(|path, body| {
        if body.contains("Words:") { model(path, body) }
        else { MockResponse { status: 200, chunks: vec![maru_llm::testing::sse_event("POLITE: 감사합니다\nFORMAL: 감사드립니다\n"), b"data: {\"error\":\"failed at EOF\"}".to_vec()], delay_ms: 0 } }
    }, |_| {}).await;
    for _ in 0..2 {
        let events = collect(h.engine.translate(Capture::typed("Thank you.")).1).await;
        assert!(events.iter().any(|e| matches!(e, Event::Error { .. })));
        assert!(matches!(events.last(), Some(Event::Done { .. })));
    }
    assert_eq!(h.requests().iter().filter(|b| b.contains("Translate into Korean")).count(), 2);
    assert!(h.engine.history().is_empty());
}

#[tokio::test]
async fn repeating_a_cached_translation_retries_failed_meanings_without_retranslating() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    for source in [SENTENCE, "Thank you."] {
        let gloss_calls = AtomicUsize::new(0);
        let h = harness_with(move |path, body| {
            if body.contains("Words:") && gloss_calls.fetch_add(1, Ordering::SeqCst) == 0 {
                MockResponse { status: 200, chunks: vec![maru_llm::testing::sse_event("0|untrusted partial meaning|\n"), b"data: {\"error\":\"temporary failure\"}\n\n".to_vec()], delay_ms: 0 }
            } else { model(path, body) }
        }, |_| {}).await;
        let first = collect(h.engine.translate(Capture::typed(source)).1).await;
        assert!(first.iter().any(|e| matches!(e, Event::TranslationDone { .. })));
        assert!(first.iter().any(|e| matches!(e, Event::Error { .. })));
        let second = collect(h.engine.translate(Capture::typed(source)).1).await;
        assert!(second.iter().any(|e| matches!(e, Event::Started { cached: true, .. })));
        assert!(second.iter().any(|e| matches!(e, Event::Gloss { gloss, .. } if gloss.meaning == "I")));
        assert!(!second.iter().any(|e| matches!(e, Event::Gloss { gloss, .. } if gloss.meaning == "untrusted partial meaning")));
        assert_eq!(h.requests().iter().filter(|b| b.contains("Words:")).count(), 2);
        assert_eq!(h.requests().iter().filter(|b| !b.contains("Words:")).count(), 1);
        assert_eq!(h.engine.history().len(), 1);
        collect(h.engine.translate(Capture::typed(source)).1).await;
        assert_eq!(h.requests().len(), 3, "completed meanings are reusable too");
    }
}

#[tokio::test]
async fn only_missing_meanings_are_retried_when_a_finished_gloss_answer_omits_a_word() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let gloss_calls = AtomicUsize::new(0);
    let h = harness_with(move |path, body| {
        if body.contains("Words:") && gloss_calls.fetch_add(1, Ordering::SeqCst) == 0 {
            MockResponse { status: 200, chunks: sse_answer(&["0|I|\n"]), delay_ms: 0 }
        } else if body.contains("Words:") {
            MockResponse { status: 200, chunks: sse_answer(&["0|unrequested meaning|\n1|school|\n2|to study|\n"]), delay_ms: 0 }
        } else { model(path, body) }
    }, |_| {}).await;
    collect(h.engine.translate(Capture::typed(SENTENCE)).1).await;
    let events = collect(h.engine.translate(Capture::typed(SENTENCE)).1).await;
    assert!(events.iter().any(|e| matches!(e, Event::Gloss { gloss, .. } if gloss.index == 1 && gloss.meaning == "school")));
    assert!(!events.iter().any(|e| matches!(e, Event::Gloss { gloss, .. } if gloss.meaning == "unrequested meaning")));
    let requests = h.requests();
    let retry = requests.iter().rfind(|b| b.contains("Words:")).unwrap();
    assert!(!retry.contains("0. 저는"), "a validated meaning is already cached: {retry}");
    assert!(retry.contains("1. 학교에서") && retry.contains("2. 공부했어요"));
    collect(h.engine.translate(Capture::typed(SENTENCE)).1).await;
    assert_eq!(h.requests().len(), 3);
}

#[tokio::test]
async fn a_cached_answer_reannotates_words_and_requests_meanings_that_are_no_longer_known() {
    let h = harness().await;
    h.engine.set_known("학교", true).unwrap();
    collect(h.engine.translate(Capture::typed(SENTENCE)).1).await;
    h.engine.set_known("학교", false).unwrap();
    let events = collect(h.engine.translate(Capture::typed(SENTENCE)).1).await;
    assert!(events.iter().any(|e| matches!(e, Event::Gloss { gloss, .. } if gloss.index == 1 && gloss.meaning == "school")));
    let requests = h.requests();
    let retry = requests.iter().rfind(|b| b.contains("Words:")).unwrap();
    assert!(retry.contains("1. 학교에서"));
    assert!(!retry.contains("0. 저는") && !retry.contains("2. 공부했어요"));
    assert_eq!(requests.len(), 3);
}

#[tokio::test]
async fn a_live_request_stalled_before_headers_releases_its_slot_for_the_next_line() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut stalled, _) = listener.accept().await.unwrap();
        let mut request = [0; 8192];
        assert!(stalled.read(&mut request).await.unwrap() > 0);
        let (mut next, _) = listener.accept().await.unwrap();
        assert!(next.read(&mut request).await.unwrap() > 0);
        next.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n").await.unwrap();
        for event in sse_answer(&["Next answer."]) { next.write_all(&event).await.unwrap(); }
        next.shutdown().await.unwrap();
        drop(stalled);
    });
    let mut cfg = Config::default();
    cfg.llm.request_timeout_secs = 1;
    cfg.continuous.max_in_flight = 1;
    let client = LlmClient::new(&root, &cfg.llm);
    let h = build(cfg, Some(client), None);
    let (_, first) = h.engine.translate_live(live_line(SENTENCE));
    let (_, next) = h.engine.translate_live(live_line("저는 학생이에요"));
    let (first, next) = tokio::join!(collect(first), collect(next));
    server.await.unwrap();
    assert!(first.iter().any(|e| matches!(e, Event::Error { .. })));
    assert!(matches!(first.last(), Some(Event::Done { .. })));
    assert_eq!(text_of(&next, Variant::Primary), "Next answer.");
    assert_eq!(h.engine.history().len(), 1);
}

#[tokio::test]
async fn unfinished_live_answers_are_not_saved_as_context_cache_or_history() {
    let h = harness_with(|_, _| MockResponse { status: 200, chunks: vec![maru_llm::testing::sse_event("Partial.")], delay_ms: 0 }, |_| {}).await;
    for _ in 0..2 {
        let events = collect(h.engine.translate_live(live_line(SENTENCE)).1).await;
        assert!(events.iter().any(|e| matches!(e, Event::Error { .. })));
        assert!(!events.iter().any(|e| matches!(e, Event::TranslationDone { .. })));
    }
    assert_eq!(h.requests().len(), 2);
    assert!(h.engine.history().is_empty());
    assert!(h.requests().iter().all(|b| !b.contains("\"assistant\"")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn overlapping_live_answers_are_context_in_capture_order_instead_of_completion_order() {
    let h = harness_with(|_, body| {
        let value: serde_json::Value = serde_json::from_str(body).unwrap();
        let source = value["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap();
        MockResponse { status: 200, chunks: sse_answer(&["Answer."]), delay_ms: if source.contains(SENTENCE) { 150 } else { 0 } }
    }, |c| c.continuous.max_in_flight = 2).await;
    let (first, a) = h.engine.translate_live(live_line(SENTENCE));
    let (second, b) = h.engine.translate_live(live_line("저는 학생이에요"));
    collect(b).await;
    collect(a).await;
    assert_eq!(h.engine.history().iter().map(|i| i.id).collect::<Vec<_>>(), [second, first]);
    collect(h.engine.translate_live(live_line("안녕하세요")).1).await;
    let request: serde_json::Value = serde_json::from_str(h.requests().last().unwrap()).unwrap();
    let previous: Vec<_> = request["messages"].as_array().unwrap().iter().filter(|m| m["role"] == "user").map(|m| m["content"].as_str().unwrap()).collect();
    assert!(previous[0].contains(SENTENCE), "{previous:?}");
    assert!(previous[1].contains("저는 학생이에요"), "{previous:?}");
}

#[tokio::test]
async fn snapshots_reuse_the_live_translator_until_live_is_turned_off() {
    use maru_core::live_model::LiveModel;
    let main = spawn_mock(model).await;
    let native = spawn_mock(|_, _| MockResponse { status: 200, chunks: vec![b"data: {\"content\":\"Local translation.\",\"stop\":true}\n\n".to_vec()], delay_ms: 0 }).await;
    let mut cfg = Config::default();
    cfg.continuous.translation.model = LiveModel::Milmmt;
    let h = build_with_live(cfg.clone(), Some(LlmClient::new(&main.url, &cfg.llm)), None, Some(LlmClient::new(&native.url, &cfg.llm)));
    h.engine.set_live_active(true);
    for _ in 0..2 {
        let (_, rx) = h.engine.translate(Capture::typed("저는 학교에서 공부했어요"));
        let events = collect(rx).await;
        assert!(events.iter().any(|e| matches!(e, Event::Breakdown { .. })));
        assert!(events.iter().any(|e| matches!(e, Event::TranslationDone { text, .. } if text == "Local translation.")));
        assert!(!events.iter().any(|e| matches!(e, Event::Gloss { .. })));
    }
    assert!(main.requests.lock().unwrap().is_empty());
    assert_eq!(native.requests.lock().unwrap().len(), 2);
    h.engine.set_live_active(false);
    let (_, rx) = h.engine.translate(Capture::typed("저는 학교에서 공부했어요"));
    let events = collect(rx).await;
    assert!(events.iter().any(|e| matches!(e, Event::TranslationDone { text, .. } if text == "I studied at school.")));
    assert!(events.iter().any(|e| matches!(e, Event::Gloss { .. })));
    assert_eq!(native.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn snapshots_prefer_the_configured_quality_server_and_failed_requests_fall_back_without_calling_local_gemma() {
    use maru_core::live_model::LiveModel;
    for failed in [false, true] {
        let main = spawn_mock(model).await;
        let remote = spawn_mock(move |path, body| {
            if failed && path != "/health" { MockResponse { status: 500, chunks: vec![b"model unavailable".to_vec()], delay_ms: 0 } } else { model(path, body) }
        }).await;
        let native = spawn_mock(|_, _| MockResponse { status: 200, chunks: vec![b"data: {\"content\":\"Fallback.\",\"stop\":true}\n\n".to_vec()], delay_ms: 0 }).await;
        let mut cfg = Config::default();
        cfg.continuous.translation.model = LiveModel::Milmmt;
        cfg.continuous.translation.snapshot_endpoint = Some(remote.url.clone());
        cfg.continuous.translation.snapshot_model_name = "nas-gemma".into();
        let h = build_with_live(cfg.clone(), Some(LlmClient::new(&main.url,&cfg.llm)), None, Some(LlmClient::new(&native.url,&cfg.llm)));
        h.engine.set_live_active(true);
        let (_,rx) = h.engine.translate(Capture::typed("저는 학교에서 공부했어요"));
        let events = collect(rx).await;
        assert!(main.requests.lock().unwrap().is_empty());
        assert_eq!(native.requests.lock().unwrap().len(), usize::from(failed));
        let expected = if failed { "Fallback." } else { "I studied at school." };
        assert!(events.iter().any(|e| matches!(e, Event::TranslationDone {text,..} if text == expected)));
        assert!(remote.requests.lock().unwrap().iter().any(|(_,b)| b.contains("nas-gemma")));
    }
}

#[tokio::test]
async fn live_native_context_is_opt_in_and_a_missing_live_model_does_not_use_gemma() {
    use maru_core::live_model::LiveModel;
    for context in [false, true] {
        let main = spawn_mock(model).await;
        let native = spawn_mock(|_, _| MockResponse { status:200,chunks:vec![b"data: {\"content\":\"Yes.\",\"stop\":true}\n\n".to_vec()],delay_ms:0 }).await;
        let mut cfg=Config::default();cfg.continuous.translation.model=LiveModel::Milmmt;cfg.continuous.translation.dialogue_context=context;
        let h=build_with_live(cfg.clone(),Some(LlmClient::new(&main.url,&cfg.llm)),None,Some(LlmClient::new(&native.url,&cfg.llm)));
        h.engine.set_live_active(true);
        for source in ["네.","왜요?"] {
            let line=maru_engine::LiveLine {capture:Capture::typed(source),appeared:std::time::Instant::now(),ready_ms:0,ocr_ms:0};
            let (_,rx)=h.engine.translate_live(line);collect(rx).await;
        }
        let requests=native.requests.lock().unwrap();
        let body:serde_json::Value=serde_json::from_str(&requests[1].1).unwrap();
        assert_eq!(body["prompt"].as_str().unwrap().contains("English: Yes."),context);
        assert!(main.requests.lock().unwrap().is_empty());
    }
    let h=harness_with(model,|c|c.continuous.translation.model=LiveModel::HyMt2).await;
    h.engine.set_live_active(true);
    let (_,rx)=h.engine.translate(Capture::typed("네."));
    assert!(collect(rx).await.iter().any(|e|matches!(e,Event::Error {..})));
    assert!(h.requests().is_empty());
}

fn build(cfg: Config, llm: Option<LlmClient>, server: Option<MockServer>) -> Harness {
    build_with_live(cfg, llm, server, None)
}

fn build_with_live(cfg: Config, llm: Option<LlmClient>, server: Option<MockServer>, live_llm: Option<LlmClient>) -> Harness {
    let dict = Dictionary::open_in_memory().unwrap();
    dict.import_kaikki(Cursor::new(KAIKKI), |_| {}).unwrap();
    dict.import_levels(Cursor::new("학교\t1\n")).unwrap();
    let vocab = Arc::new(Vocab::open_in_memory(SrsConfig::from_learning(&cfg.learning)).unwrap());
    let engine = Engine::new(EngineParts {
        config: cfg,
        analyzer: Analyzer::heuristic(),
        dict: Some(Arc::new(dict)),
        vocab: vocab.clone(),
        llm,
        llm_status: None,
        platform: Platform { selection: Arc::new(FakeSelection::default()), screen: Arc::new(FakeScreen::new(vec![])), os_ocr: None },
        ocr: None,
        scan_ocr: None,
        live_ocr: None,
        live_llm,
        labels: None,
    });
    Harness { engine, vocab, server }
}

async fn collect(mut rx: EventRx) -> Vec<Event> {
    let mut out = Vec::new();
    loop {
        match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
            Ok(Some(e)) => {
                let done = matches!(e, Event::Done { .. });
                out.push(e);
                if done {
                    return out;
                }
            }
            Ok(None) => return out,
            Err(_) => panic!("timed out waiting for events; got {out:#?}"),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_corrected_snapshot_keeps_its_app_drops_stale_word_boxes_and_refuses_an_old_editor() {
    let h=harness().await;
    let mut capture=Capture::typed("ㅅ컵");capture.source=CaptureSource::Ocr;
    capture.app=Some(AppInfo {exe:"game.exe".into(),title:"Game".into()});
    capture.words=vec![maru_core::WordBox {text:"ㅅ컵".into(),rect:maru_core::Rect::new(1.0,2.0,30.0,12.0)}];
    let (id,rx)=h.engine.translate(capture);collect(rx).await;
    let (fixed,next,rx)=h.engine.correct_snapshot(id,"스킵",false).unwrap();
    assert_eq!(fixed.text,"스킵");assert!(fixed.words.is_empty());assert_eq!(fixed.app.unwrap().exe,"game.exe");
    let events=collect(rx).await;
    assert!(events.iter().any(|e|matches!(e,Event::Started {capture,..} if capture.text=="스킵")));
    assert!(h.engine.correct_snapshot(id,"다른것",false).is_err());
    assert!(h.engine.correct_snapshot(next,"",false).is_err());
    let (_,rx)=h.engine.translate(Capture::typed("학교"));collect(rx).await;
    assert!(h.engine.correct_snapshot(next,"스킵",false).is_err());
}

fn names(events: &[Event]) -> Vec<&'static str> {
    events
        .iter()
        .map(|e| match e {
            Event::Started { .. } => "started",
            Event::Breakdown { .. } => "breakdown",
            Event::Translation { .. } => "translation",
            Event::TranslationDone { .. } => "translation_done",
            Event::Gloss { .. } => "gloss",
            Event::Note { .. } => "note",
            Event::DeepEntries { .. } => "deep_entries",
            Event::DeepDelta { .. } => "deep_delta",
            Event::WordState { .. } => "word_state",
            Event::ScanStarted { .. } => "scan_started",
            Event::ScanItems { .. } => "scan_items",
            Event::ScanLabel { .. } => "scan_label",
            Event::ScanCorrection { .. } => "scan_correction",
            Event::ScanLabelsFinished { .. } => "scan_labels_finished",
            Event::ScanSeparated { .. } => "scan_separated",
            Event::Timing { .. } => "timing",
            Event::Error { .. } => "error",
            Event::Done { .. } => "done",
        })
        .collect()
}

fn first_index(events: &[Event], name: &str) -> usize {
    names(events).iter().position(|n| *n == name).unwrap_or_else(|| panic!("no {name} event in {:?}", names(events)))
}

fn text_of(events: &[Event], variant: Variant) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Translation { variant: v, delta, .. } if *v == variant => Some(delta.as_str()),
            _ => None,
        })
        .collect()
}

const SENTENCE: &str = "저는 학교에서 공부했어요";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn korean_to_english_streams_breakdown_first_then_translation_and_meanings() {
    let h = harness().await;
    let (_, rx) = h.engine.translate(Capture::typed(SENTENCE));
    let events = collect(rx).await;

    // the offline breakdown arrives before any model output
    assert!(first_index(&events, "breakdown") < first_index(&events, "translation"));
    assert!(first_index(&events, "started") < first_index(&events, "breakdown"));
    assert_eq!(names(&events).last(), Some(&"done"));

    let Event::Started { direction, cached, .. } = &events[0] else { panic!() };
    assert_eq!((*direction, *cached), (Direction::KoToEn, false));

    let Event::Breakdown { side, eojeols, romanization, pronunciation, .. } = &events[first_index(&events, "breakdown")] else { panic!() };
    assert_eq!(*side, Side::Source);
    assert_eq!(eojeols.len(), 3);
    assert_eq!(eojeols[1].lemma.as_deref(), Some("학교"));
    assert_eq!(eojeols[1].dict_glosses, ["school"], "dictionary meanings are available before the model answers");
    assert_eq!(eojeols[1].level.as_deref(), Some("TOPIK 1"));
    assert_eq!(eojeols[2].lemma.as_deref(), Some("공부하다"));
    assert_eq!(romanization, "jeoneun hakgyoeseo gongbuhaesseoyo");
    assert_eq!(pronunciation, "저는 학꾜에서 공부해써요");

    assert_eq!(text_of(&events, Variant::Primary), "I studied at school.");
    assert!(events.iter().any(|e| matches!(e, Event::TranslationDone { text, .. } if text == "I studied at school.")));
    let glosses: Vec<_> = events.iter().filter_map(|e| match e { Event::Gloss { gloss, .. } => Some((gloss.index, gloss.meaning.as_str())), _ => None }).collect();
    assert_eq!(glosses, [(0, "I"), (1, "school"), (2, "to study")]);
    assert!(events.iter().any(|e| matches!(e, Event::Note { text, .. } if text.contains("Topic marker"))));

    let stages: Vec<Stage> = events.iter().filter_map(|e| match e { Event::Timing { stage, .. } => Some(*stage), _ => None }).collect();
    for s in [Stage::Morphology, Stage::FirstToken, Stage::Llm, Stage::Total] {
        assert!(stages.contains(&s), "missing timing for {s:?}: {stages:?}");
    }
    assert!(!events.iter().any(|e| matches!(e, Event::Error { .. })));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_same_line_twice_is_served_from_the_cache_without_calling_the_model() {
    let h = harness().await;
    collect(h.engine.translate(Capture::typed(SENTENCE)).1).await;
    let calls_after_first = h.requests().len();
    assert_eq!(calls_after_first, 2, "one translation + one gloss request");

    let events = collect(h.engine.translate(Capture::typed(SENTENCE)).1).await;
    let Event::Started { cached, .. } = &events[0] else { panic!() };
    assert!(*cached);
    assert_eq!(h.requests().len(), calls_after_first, "no new model calls");
    assert_eq!(text_of(&events, Variant::Primary), "I studied at school.");
    assert_eq!(events.iter().filter(|e| matches!(e, Event::Gloss { .. })).count(), 3);
    assert!(events.iter().any(|e| matches!(e, Event::Breakdown { .. })));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn known_words_are_not_sent_for_glossing() {
    let h = harness().await;
    h.engine.set_known("학교", true).unwrap();
    let events = collect(h.engine.translate(Capture::typed(SENTENCE)).1).await;

    let Event::Breakdown { eojeols, .. } = &events[first_index(&events, "breakdown")] else { panic!() };
    assert_eq!(eojeols[1].state, WordState::Known);
    let gloss_request = h.requests().into_iter().find(|b| b.contains("Words:")).expect("a gloss request");
    assert!(!gloss_request.contains("1. 학교에서"), "known word must be left out to save tokens: {gloss_request}");
    assert!(gloss_request.contains("0. 저는") && gloss_request.contains("2. 공부했어요"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn english_to_korean_gives_registers_and_breaks_down_the_korean_while_streaming() {
    let h = harness().await;
    let (_, rx) = h.engine.translate(Capture::typed("Thank you"));
    let events = collect(rx).await;

    let Event::Started { direction, .. } = &events[0] else { panic!() };
    assert_eq!(*direction, Direction::EnToKo);
    assert_eq!(text_of(&events, Variant::Polite), "감사합니다");
    assert_eq!(text_of(&events, Variant::Formal), "감사드립니다");
    assert_eq!(text_of(&events, Variant::Casual), "고마워");

    // the Korean output is analysed (Target side) and glossed
    let Event::Breakdown { side, eojeols, romanization, .. } = &events[first_index(&events, "breakdown")] else { panic!() };
    assert_eq!(*side, Side::Target);
    assert_eq!(eojeols[0].surface, "감사합니다");
    assert_eq!(romanization, "gamsahamnida");
    assert!(events.iter().any(|e| matches!(e, Event::Gloss { side: Side::Target, .. })));
    assert!(events.iter().any(|e| matches!(e, Event::Note { text, .. } if text.contains("CASUAL"))));
    assert_eq!(names(&events).last(), Some(&"done"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn requested_register_comes_from_the_app_profile_and_reaches_the_prompt() {
    let h = harness_with(model, |cfg| {
        let mut p = AppProfile { name: "game".into(), exe: vec!["Game.exe".into()], register: Some(Register::Casual), hint: Some("dialogue from a fantasy RPG".into()), ..Default::default() };
        p.glossary.insert("이도현".into(), "Lee Do-hyun".into());
        cfg.profiles.push(p);
    })
    .await;
    let mut cap = Capture::typed("Where is Lee Do-hyun?");
    cap.app = Some(AppInfo { exe: "game.exe".into(), title: "Game".into() });
    collect(h.engine.translate(cap).1).await;
    let body = h.requests().into_iter().find(|b| b.contains("Translate into Korean")).unwrap();
    assert!(body.contains("Write the CASUAL line first"), "{body}");
    assert!(body.contains("fantasy RPG") && body.contains("이도현 = Lee Do-hyun"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn previous_lines_become_context_for_the_next_one() {
    let h = harness().await;
    collect(h.engine.translate(Capture::typed(SENTENCE)).1).await;
    collect(h.engine.translate(Capture::typed("저는 학생이에요")).1).await;
    let second = h.requests().into_iter().filter(|b| b.contains("Translate into English")).nth(1).unwrap();
    assert!(second.contains("Previous lines") && second.contains(SENTENCE) && second.contains("I studied at school."), "{second}");
    assert_eq!(h.engine.history().len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ocr_captures_tell_the_model_the_text_may_be_noisy() {
    let h = harness().await;
    let mut cap = Capture::typed(SENTENCE);
    cap.source = CaptureSource::Ocr;
    collect(h.engine.translate(cap).1).await;
    assert!(h.requests().iter().any(|b| b.contains("read from the screen by OCR")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_a_model_the_user_still_gets_the_breakdown_and_a_clear_message() {
    let h = build(Config::default(), None, None);
    let events = collect(h.engine.translate(Capture::typed(SENTENCE)).1).await;
    assert!(events.iter().any(|e| matches!(e, Event::Breakdown { .. })));
    let err = events.iter().find_map(|e| match e { Event::Error { stage: Stage::Llm, message, .. } => Some(message.clone()), _ => None }).unwrap();
    assert!(err.contains("maru setup llm"), "{err}");
    assert_eq!(names(&events).last(), Some(&"done"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreachable_model_is_reported_but_does_not_break_the_stream() {
    let cfg = Config::default();
    let h = build(cfg.clone(), Some(LlmClient::new("http://127.0.0.1:1", &cfg.llm)), None);
    let events = collect(h.engine.translate(Capture::typed(SENTENCE)).1).await;
    assert!(events.iter().any(|e| matches!(e, Event::Breakdown { .. })));
    assert!(events.iter().any(|e| matches!(e, Event::Error { stage: Stage::Llm, message, .. } if message.contains("cannot reach"))));
    assert_eq!(names(&events).last(), Some(&"done"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_newer_request_cancels_the_older_one() {
    let slow = |p: &str, b: &str| MockResponse { delay_ms: 150, ..model(p, b) };
    let h = harness_with(slow, |_| {}).await;
    let (id1, mut rx1) = h.engine.translate(Capture::typed(SENTENCE));
    // wait until the first request is clearly in flight
    loop {
        if matches!(rx1.recv().await, Some(Event::Breakdown { .. })) {
            break;
        }
    }
    let (id2, rx2) = h.engine.translate(Capture::typed("저는 학생이에요"));
    assert!(id2 > id1);

    let mut rest = Vec::new();
    while let Ok(Some(e)) = tokio::time::timeout(Duration::from_secs(3), rx1.recv()).await {
        rest.push(e);
    }
    assert!(!rest.iter().any(|e| matches!(e, Event::Done { .. })), "the cancelled request must not finish: {rest:?}");
    let events2 = collect(rx2).await;
    assert_eq!(names(&events2).last(), Some(&"done"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn long_selections_are_cut_at_a_sentence_and_flagged() {
    let h = harness_with(model, |cfg| cfg.capture.max_chars = 30).await;
    let text = "첫 번째 문장입니다. 두 번째 문장입니다. 세 번째 문장이 아주 길게 계속 이어집니다";
    let events = collect(h.engine.translate(Capture::typed(text)).1).await;
    let Event::Started { capture, .. } = &events[0] else { panic!() };
    assert_eq!(capture.text, "첫 번째 문장입니다. 두 번째 문장입니다.");
    assert!(events.iter().any(|e| matches!(e, Event::Note { text, .. } if text.contains("first part"))));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn text_without_letters_is_rejected_politely() {
    let h = harness().await;
    let events = collect(h.engine.translate(Capture::typed("12345 !!!")).1).await;
    assert!(matches!(events.first(), Some(Event::Error { stage: Stage::Capture, .. })));
    assert!(h.requests().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hover_lookup_is_offline_and_never_calls_the_model() {
    let h = harness().await;
    let events = collect(h.engine.lookup_word(Capture::typed("학교에서")).1).await;
    let Event::Breakdown { eojeols, .. } = &events[first_index(&events, "breakdown")] else { panic!() };
    assert_eq!(eojeols[0].dict_glosses, ["school"]);
    assert!(h.requests().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deep_dive_explains_records_the_lookup_and_auto_saves_on_the_second_click() {
    let h = harness().await;
    let analysis = Analyzer::heuristic().analyze(SENTENCE);
    let request = DeepDiveRequest {
        eojeol: analysis.eojeols[2].clone(),
        sentence: SENTENCE.into(),
        translation: Some("I studied at school.".into()),
        meaning: Some("to study".into()),
        app: None,
    };

    let events = collect(h.engine.deep_dive(request.clone()).1).await;
    let Event::WordState { status, .. } = &events[0] else { panic!("first event should be the recorded lookup: {:?}", names(&events)) };
    assert_eq!((status.key.as_str(), status.lookups, status.saved, status.state), ("공부하다", 1, false, WordState::Learning));

    let Event::DeepEntries { entries, conjugation, .. } = &events[first_index(&events, "deep_entries")] else { panic!() };
    assert_eq!(entries[0].lemma, "공부하다");
    assert_eq!(conjugation.iter().map(|c| c.form.as_str()).collect::<Vec<_>>(), ["공부", "하다", "았", "어요"]);

    let section = |s: DeepSection| -> String {
        events.iter().filter_map(|e| match e { Event::DeepDelta { section, delta, .. } if *section == s => Some(delta.as_str()), _ => None }).collect()
    };
    assert_eq!(section(DeepSection::Usage), "Means to study.\n");
    assert!(section(DeepSection::Examples).contains("공부해요"));
    assert!(section(DeepSection::Nuance).contains("self-study"));
    assert_eq!(names(&events).last(), Some(&"done"));

    // the model was grounded with the dictionary entry
    assert!(h.requests().iter().any(|b| b.contains("Dictionary: verb: to study")));

    // second click → queued for review automatically, with both sentences stored
    let again = DeepDiveRequest { sentence: "어제 공부했어요".into(), ..request };
    let events = collect(h.engine.deep_dive(again).1).await;
    let Event::WordState { status, .. } = &events[0] else { panic!() };
    assert_eq!((status.lookups, status.saved), (2, true));
    assert_eq!(h.vocab.contexts("공부하다", 10).unwrap().len(), 2);
    assert_eq!(h.engine.due_words(10).unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reviews_and_bulk_known_levels_flow_back_into_word_states() {
    let h = harness().await;
    h.engine.save_word("학교").unwrap();
    let r = h.engine.review_word("학교", 3).unwrap();
    assert!(r.interval_days >= 2.0 && r.status.saved);
    assert!(h.engine.review_word("학교", 9).is_err(), "ratings outside 1-4 are rejected");

    assert_eq!(h.engine.mark_known_up_to_level(1).unwrap(), 1);
    assert_eq!(h.engine.word_state("학교"), WordState::Known);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn saved_words_export_as_an_anki_import_file() {
    let h = harness().await;
    let saved = h.engine.save_word("학교").unwrap();
    assert_eq!((saved.meaning.as_deref(), saved.level.as_deref()), (Some("school"), Some("TOPIK 1")), "meaning comes from the dictionary");
    let path = std::env::temp_dir().join(format!("maru-export-{}.tsv", std::process::id()));
    assert_eq!(h.engine.export_tsv(&path).unwrap(), 1);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.starts_with("학교\t") && text.trim_end().ends_with("\tmaru"), "{text}");
    assert!(text.contains("school"), "the card's answer must not be empty: {text}");
    std::fs::remove_file(path).ok();
    let err = h.engine.anki_overview("학교", None).await.unwrap_err().to_string();
    assert!(err.contains("Settings → Anki"), "disabled by default with an actionable message: {err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn requests_wait_for_a_model_that_is_still_loading() {
    use maru_llm::LlmStatus;
    let server = spawn_mock(model).await;
    let cfg = Config::default();
    let (status_tx, status_rx) = tokio::sync::watch::channel(LlmStatus::Starting);
    let engine = Engine::new(EngineParts {
        config: cfg.clone(),
        analyzer: Analyzer::heuristic(),
        dict: None,
        vocab: Arc::new(Vocab::open_in_memory(SrsConfig::default()).unwrap()),
        llm: Some(LlmClient::new(&server.url, &cfg.llm)),
        llm_status: Some(status_rx),
        platform: Platform { selection: Arc::new(FakeSelection::default()), screen: Arc::new(FakeScreen::new(vec![])), os_ocr: None },
        ocr: None,
        scan_ocr: None,
        live_ocr: None,
        live_llm: None,
        labels: None,
    });
    assert_eq!(engine.llm_status(), LlmStatus::Starting);

    let (_, mut rx) = engine.translate(Capture::typed(SENTENCE));
    // the offline breakdown must appear while the model is still loading
    let mut early = Vec::new();
    while let Some(e) = rx.recv().await {
        let note = matches!(&e, Event::Note { text, .. } if text.contains("still loading"));
        early.push(e);
        if note {
            break;
        }
    }
    assert!(early.iter().any(|e| matches!(e, Event::Breakdown { .. })));
    assert!(server.requests.lock().unwrap().is_empty(), "no model call before it is ready");

    status_tx.send(LlmStatus::Ready).unwrap();
    let rest = collect(rx).await;
    assert_eq!(text_of(&rest, Variant::Primary), "I studied at school.");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_model_that_failed_to_start_explains_why() {
    use maru_llm::LlmStatus;
    let cfg = Config::default();
    let (_tx, status_rx) = tokio::sync::watch::channel(LlmStatus::Failed("out of VRAM".into()));
    let engine = Engine::new(EngineParts {
        config: cfg.clone(),
        analyzer: Analyzer::heuristic(),
        dict: None,
        vocab: Arc::new(Vocab::open_in_memory(SrsConfig::default()).unwrap()),
        llm: Some(LlmClient::new("http://127.0.0.1:1", &cfg.llm)),
        llm_status: Some(status_rx),
        platform: Platform { selection: Arc::new(FakeSelection::default()), screen: Arc::new(FakeScreen::new(vec![])), os_ocr: None },
        ocr: None,
        scan_ocr: None,
        live_ocr: None,
        live_llm: None,
        labels: None,
    });
    let events = collect(engine.translate(Capture::typed(SENTENCE)).1).await;
    assert!(events.iter().any(|e| matches!(e, Event::Error { message, .. } if message.contains("out of VRAM"))));
    assert!(events.iter().any(|e| matches!(e, Event::Breakdown { .. })));
}

// ───────────────────────────── screen scan ─────────────────────────────

mod scan {
    use maru_core::config::GlossMode;
    use maru_core::{LabelSource, Rect, ScanItem};
    use maru_engine::labels::LabelCache;
    use maru_ocr::{OcrEngine, OcrLine, OcrOutput};

    use super::*;

    /// Returns the same recognised lines for every screen.
    struct FakeOcr(Vec<OcrLine>);

    impl OcrEngine for FakeOcr {
        fn name(&self) -> &'static str {
            "fake"
        }
        fn recognize(&self, _: &image::RgbImage) -> maru_ocr::Result<OcrOutput> {
            Ok(OcrOutput { lines: self.0.clone() })
        }
    }

    fn line(text: &str, x: f32, y: f32) -> OcrLine {
        OcrLine { text: text.into(), rect: Rect::new(x, y, 120.0, 22.0), words: vec![], confidence: 0.95 }
    }

    /// A model that labels the screen strings it recognises, line by line, and counts as `?` the rest.
    fn labeller(_path: &str, body: &str) -> MockResponse {
        let table = [("인벤토리", "Inventory"), ("확인", "OK"), ("상점", "Shop"), ("학교", "School"), ("새 게임", "New game")];
        let mut lines = Vec::new();
        // the request body is JSON: its newlines arrive as the two characters `\n`
        for part in body.split("\\n") {
            let Some((n, text)) = part.split_once('|') else { continue };
            if let Ok(n) = n.trim().parse::<usize>() {
                let label = table.iter().find(|(k, _)| text.trim() == *k).map(|(_, v)| *v).unwrap_or("?");
                lines.push(format!("{n}|{label}\n"));
            }
        }
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        MockResponse { status: 200, chunks: sse_answer(&refs), delay_ms: 1 }
    }

    struct Setup {
        engine: Engine,
        vocab: Arc<Vocab>,
        server: Option<MockServer>,
    }

    impl Setup {
        fn model_requests(&self) -> usize {
            self.server.as_ref().map_or(0, |s| s.requests.lock().unwrap().iter().filter(|(_, b)| b.contains("Strings:")).count())
        }
    }

    async fn setup(lines: Vec<OcrLine>, with_model: bool, tweak: impl FnOnce(&mut Config), labels: Option<Arc<LabelCache>>) -> Setup {
        let server = if with_model { Some(spawn_mock(labeller).await) } else { None };
        setup_with_server(lines, server, tweak, labels)
    }

    fn setup_with_server(lines: Vec<OcrLine>, server: Option<MockServer>, tweak: impl FnOnce(&mut Config), labels: Option<Arc<LabelCache>>) -> Setup {
        let mut cfg = Config::default();
        tweak(&mut cfg);
        let dict = Dictionary::open_in_memory().unwrap();
        dict.import_kaikki(Cursor::new(KAIKKI), |_| {}).unwrap();
        let vocab = Arc::new(Vocab::open_in_memory(SrsConfig::from_learning(&cfg.learning)).unwrap());
        let screen = FakeScreen { app: Some(AppInfo { exe: "game.exe".into(), title: "Game".into() }), ..FakeScreen::new(vec![FakeScreen::blank(1920, 1080)]) };
        let llm = server.as_ref().map(|s| LlmClient::new(&s.url, &cfg.llm));
        let engine = Engine::new(EngineParts {
            config: cfg,
            analyzer: Analyzer::heuristic(),
            dict: Some(Arc::new(dict)),
            vocab: vocab.clone(),
            llm,
            llm_status: None,
            platform: Platform { selection: Arc::new(FakeSelection::default()), screen: Arc::new(screen), os_ocr: None },
            ocr: None,
            scan_ocr: Some(Arc::new(FakeOcr(lines))),
            live_ocr: None,
            live_llm: None,
            labels,
        });
        Setup { engine, vocab, server }
    }

    const SCREEN: Rect = Rect::new(0.0, 0.0, 1920.0, 1080.0);

    fn items_of(events: &[Event]) -> Vec<ScanItem> {
        events
            .iter()
            .find_map(|e| if let Event::ScanItems { items, .. } = e { Some(items.clone()) } else { None })
            .expect("a ScanItems event")
    }

    fn model_labels(events: &[Event]) -> Vec<(u32, String)> {
        events.iter().filter_map(|e| if let Event::ScanLabel { item, label, .. } = e { Some((*item, label.clone())) } else { None }).collect()
    }

    fn completions(events: &[Event]) -> Vec<(Vec<u32>, Option<String>)> {
        events.iter().filter_map(|e| if let Event::ScanLabelsFinished { items, error, .. } = e {
            Some((items.clone(), error.clone()))
        } else { None }).collect()
    }

    fn tight_words(words: &[&str]) -> Vec<OcrLine> {
        let mut x=100.0;
        words.iter().map(|text| {
            let width=text.chars().count() as f32*14.0;
            let out=OcrLine {text:(*text).into(),rect:Rect::new(x,100.0,width,22.0),words:vec![],confidence:0.99};
            x+=width+2.0;out
        }).collect()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_sentence_join_keeps_word_boundaries_and_explanations_for_every_word() {
        let server=spawn_mock(model).await;
        let s=setup_with_server(tight_words(&["카메라의","각종","설정을","조정합니다"]),Some(server),|_|{},None);
        let (id,rx)=s.engine.scan(SCREEN);collect(rx).await;
        let events=collect(s.engine.join_scan_items(id,4,vec![3,1,0,2],false).await.unwrap()).await;
        let joined=items_of(&events).remove(0);
        assert_eq!(joined.text,"카메라의 각종 설정을 조정합니다");
        assert_eq!(joined.join.as_ref().unwrap().parts.len(),4);
        let (capture,single)=s.engine.capture_scan_item(id,joined.item).unwrap();
        assert!(!single);assert_eq!(capture.app.as_ref().unwrap().exe,"game.exe");
        assert!(s.engine.capture_scan_item(id,0).is_err(),"absorbed labels are no longer current");
        let translated=collect(s.engine.translate(capture.clone()).1).await;
        let words=translated.iter().find_map(|e|if let Event::Breakdown {eojeols,..}=e {Some(eojeols.clone())}else{None}).unwrap();
        assert_eq!(words.len(),4);assert!(words.iter().all(|e|e.content));
        assert!(translated.iter().any(|e|matches!(e,Event::TranslationDone {..})));
        for word in words {
            let deep=collect(s.engine.deep_dive(DeepDiveRequest {eojeol:word,sentence:capture.text.clone(),translation:None,meaning:None,app:capture.app.clone()}).1).await;
            assert!(deep.iter().any(|e|matches!(e,Event::DeepDelta {..})));
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_legacy_collapsed_join_is_spaced_and_can_be_separated_after_a_rescan() {
        let cache=Arc::new(LabelCache::open_in_memory().unwrap());
        cache.put_join("game.exe",&["카메라".into(),"설정".into()],"카메라설정",1);
        cache.put_join("other.exe",&["카메라".into(),"설정".into()],"카메라설정",1);
        let s=setup(tight_words(&["카메라","설정"]),true,|_|{},Some(cache.clone())).await;
        let (id,rx)=s.engine.scan(SCREEN);let joined=items_of(&collect(rx).await).remove(0);
        assert_eq!(joined.text,"카메라 설정");
        assert_eq!(joined.join.as_ref().unwrap().parts.len(),2);
        assert!(!s.engine.capture_scan_item(id,joined.item).unwrap().1);
        let separated=collect(s.engine.separate_scan_item(id,joined.item).await.unwrap()).await;
        assert_eq!(items_of(&separated).iter().map(|i|i.text.as_str()).collect::<Vec<_>>(),["카메라","설정"]);
        assert!(items_of(&separated).iter().all(|i|i.item>joined.item));
        assert!(s.engine.capture_scan_item(id,joined.item).is_err());
        assert!(cache.joins_for("game.exe").is_empty());assert_eq!(cache.joins_for("other.exe").len(),1);
        let again=items_of(&collect(s.engine.scan(SCREEN).1).await);
        assert_eq!(again.len(),2);assert!(again.iter().all(|i|i.join.is_none()));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_explicit_single_word_join_keeps_its_spacing_choice_after_restart() {
        let path=std::env::temp_dir().join(format!("maru-word-join-{}.db",std::process::id()));
        let _=std::fs::remove_file(&path);
        {
            let cache=Arc::new(LabelCache::open(&path).unwrap());
            let s=setup(tight_words(&["게","임"]),true,|_|{},Some(cache)).await;
            let (id,rx)=s.engine.scan(SCREEN);collect(rx).await;
            let joined=items_of(&collect(s.engine.join_scan_items(id,2,vec![0,1],true).await.unwrap()).await).remove(0);
            assert_eq!(joined.text,"게임");assert!(joined.join.unwrap().as_word);
            assert!(s.engine.capture_scan_item(id,2).unwrap().1);
        }
        {
            let cache=Arc::new(LabelCache::open(&path).unwrap());
            let s=setup(tight_words(&["게","임"]),true,|_|{},Some(cache)).await;
            let joined=items_of(&collect(s.engine.scan(SCREEN).1).await).remove(0);
            assert_eq!(joined.text,"게임");assert!(joined.join.unwrap().as_word);
        }
        let _=std::fs::remove_file(path);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn nested_legacy_rules_match_their_old_text_and_separate_to_all_original_pieces() {
        let cache=Arc::new(LabelCache::open_in_memory().unwrap());
        cache.put_join("game.exe",&["카메라".into(),"설정".into()],"카메라설정",1);
        cache.put_join("game.exe",&["카메라설정".into(),"변경".into()],"카메라설정변경",2);
        let s=setup(tight_words(&["카메라","설정","변경"]),true,|_|{},Some(cache.clone())).await;
        let (id,rx)=s.engine.scan(SCREEN);let joined=items_of(&collect(rx).await).remove(0);
        assert_eq!(joined.text,"카메라 설정 변경");assert_eq!(joined.join.as_ref().unwrap().rules.len(),2);
        let restored=items_of(&collect(s.engine.separate_scan_item(id,joined.item).await.unwrap()).await);
        assert_eq!(restored.len(),3);assert!(cache.joins_for("game.exe").is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_sentence_join_preserves_a_nested_word_and_separation_forgets_both_rules() {
        let cache=Arc::new(LabelCache::open_in_memory().unwrap());
        let s=setup(tight_words(&["게","임","시작"]),true,|_|{},Some(cache.clone())).await;
        let (id,rx)=s.engine.scan(SCREEN);collect(rx).await;
        collect(s.engine.join_scan_items(id,3,vec![0,1],true).await.unwrap()).await;
        let joined=items_of(&collect(s.engine.join_scan_items(id,4,vec![3,2],false).await.unwrap()).await).remove(0);
        assert_eq!(joined.text,"게임 시작");assert_eq!(joined.join.unwrap().parts.len(),3);
        let (next,rx)=s.engine.scan(SCREEN);let recalled=items_of(&collect(rx).await).remove(0);
        assert_eq!(recalled.text,"게임 시작");
        let restored=items_of(&collect(s.engine.separate_scan_item(next,recalled.item).await.unwrap()).await);
        assert_eq!(restored.iter().map(|i|i.text.as_str()).collect::<Vec<_>>(),["게","임","시작"]);
        assert!(cache.joins_for("game.exe").is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn correcting_a_join_does_not_remove_its_separate_option_or_change_its_rule_key() {
        let cache=Arc::new(LabelCache::open_in_memory().unwrap());
        let s=setup(tight_words(&["가메라","설정"]),true,|_|{},Some(cache.clone())).await;
        let (id,rx)=s.engine.scan(SCREEN);collect(rx).await;
        collect(s.engine.join_scan_items(id,2,vec![0,1],false).await.unwrap()).await;
        let corrected=items_of(&collect(s.engine.correct_scan(id,2,"카메라의 설정",false).unwrap()).await).remove(0);
        assert!(corrected.join.is_some());assert!(!s.engine.capture_scan_item(id,corrected.item).unwrap().1);
        let restored=items_of(&collect(s.engine.separate_scan_item(id,corrected.item).await.unwrap()).await);
        assert_eq!(restored.iter().map(|i|i.text.as_str()).collect::<Vec<_>>(),["가메라","설정"]);
        assert!(cache.joins_for("game.exe").is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn separation_deletes_only_its_exact_rule_when_another_rule_has_the_same_output() {
        let cache=Arc::new(LabelCache::open_in_memory().unwrap());
        cache.put_join("game.exe",&["새".into(),"게임".into()],"새 게임",1);
        cache.put_join("game.exe",&["새게".into(),"임".into()],"새 게임",2);
        let s=setup(tight_words(&["새","게임"]),true,|_|{},Some(cache.clone())).await;
        let (id,rx)=s.engine.scan(SCREEN);let joined=items_of(&collect(rx).await).remove(0);
        collect(s.engine.separate_scan_item(id,joined.item).await.unwrap()).await;
        let left=cache.joins_for("game.exe");assert_eq!(left.len(),1);assert_eq!(left[0].parts,["새게","임"]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn duplicate_reused_or_stale_selections_are_refused_before_changing_labels() {
        let cache=Arc::new(LabelCache::open_in_memory().unwrap());
        let s=setup(tight_words(&["카메라","설정"]),false,|_|{},Some(cache.clone())).await;
        let (id,rx)=s.engine.scan(SCREEN);collect(rx).await;
        assert!(s.engine.join_scan_items(id,2,vec![0,0],false).await.is_err());
        assert!(s.engine.join_scan_items(id,0,vec![0,1],false).await.is_err());
        assert!(s.engine.capture_scan_item(id,0).is_ok());assert!(cache.joins_for("game.exe").is_empty());
        let (_,rx)=s.engine.scan(SCREEN);collect(rx).await;
        assert!(s.engine.join_scan_items(id,2,vec![0,1],false).await.is_err());
        assert!(s.engine.capture_scan_item(id,0).is_err());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failed_translation_does_not_trap_the_joined_label() {
        let s=setup(tight_words(&["카메라","설정"]),false,|_|{},Some(Arc::new(LabelCache::open_in_memory().unwrap()))).await;
        let (id,rx)=s.engine.scan(SCREEN);collect(rx).await;
        let joined=items_of(&collect(s.engine.join_scan_items(id,2,vec![0,1],false).await.unwrap()).await).remove(0);
        assert!(joined.join.is_some());
        assert_eq!(items_of(&collect(s.engine.separate_scan_item(id,joined.item).await.unwrap()).await).len(),2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn separating_a_join_keeps_each_corrected_parts_original_ocr_for_remember_and_forget() {
        let s=setup(tight_words(&["가메라","설정"]),false,|_|{},Some(Arc::new(LabelCache::open_in_memory().unwrap()))).await;
        let (id,rx)=s.engine.scan(SCREEN);collect(rx).await;
        let fixed=items_of(&collect(s.engine.correct_scan(id,0,"카메라",false).unwrap()).await).remove(0);
        collect(s.engine.join_scan_items(id,3,vec![fixed.item,1],false).await.unwrap()).await;
        let restored=items_of(&collect(s.engine.separate_scan_item(id,3).await.unwrap()).await);
        let view=s.engine.ocr_correction_view(maru_engine::ocr_corrections::Target::Scan {id,item:restored[0].item}).unwrap();
        assert_eq!(view.text,"카메라");assert_eq!(view.observed,"가메라");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stale_separation_cannot_delete_a_new_scan_or_its_saved_rules() {
        let cache=Arc::new(LabelCache::open_in_memory().unwrap());
        cache.put_join("game.exe",&["카메라".into(),"설정".into()],"카메라설정",1);
        let s=setup(tight_words(&["카메라","설정"]),true,|_|{},Some(cache.clone())).await;
        let (old,rx)=s.engine.scan(SCREEN);let old_item=items_of(&collect(rx).await).remove(0);
        let (current,rx)=s.engine.scan(SCREEN);let current_item=items_of(&collect(rx).await).remove(0);
        assert!(s.engine.separate_scan_item(old,old_item.item).await.is_err());
        assert_eq!(cache.joins_for("game.exe").len(),1);
        assert!(s.engine.capture_scan_item(current,current_item.item).is_ok());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_scan_label_keeps_its_original_game_when_the_overlay_becomes_foreground() {
        use maru_platform::ScreenGrabber;
        use std::sync::atomic::{AtomicUsize,Ordering};
        struct ChangingForeground {calls:AtomicUsize}
        impl ScreenGrabber for ChangingForeground {
            fn grab(&self,_:Rect)->maru_platform::Result<image::RgbImage>{Ok(FakeScreen::blank(8,8))}
            fn cursor(&self)->Option<maru_core::Point>{None}
            fn screen_bounds(&self)->Rect{SCREEN}
            fn foreground_app(&self)->Option<AppInfo>{
                Some(AppInfo {exe:if self.calls.fetch_add(1,Ordering::SeqCst)==0 {"game.exe"}else{"maru-app.exe"}.into(),title:"Foreground".into()})
            }
        }
        let screen=Arc::new(ChangingForeground {calls:AtomicUsize::new(0)});
        let engine=Engine::new(EngineParts {config:Config::default(),analyzer:Analyzer::heuristic(),dict:None,
            vocab:Arc::new(Vocab::open_in_memory(SrsConfig::default()).unwrap()),llm:None,llm_status:None,
            platform:Platform {selection:Arc::new(FakeSelection::default()),screen:screen.clone(),os_ocr:None},
            ocr:None,scan_ocr:Some(Arc::new(FakeOcr(tight_words(&["카메라","설정"])))),live_ocr:None,live_llm:None,labels:None});
        let (id,rx)=engine.scan(SCREEN);collect(rx).await;
        assert_eq!(screen.foreground_app().unwrap().exe,"maru-app.exe");
        let (capture,_) = engine.capture_scan_item(id,0).unwrap();
        assert_eq!(capture.app.unwrap().exe,"game.exe");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pending_labels_finish_only_after_the_delayed_model_answer() {
        let server = spawn_mock(|_, _| MockResponse { status: 200, chunks: sse_answer(&["0|Inventory\n"]), delay_ms: 30 }).await;
        let s = setup_with_server(vec![line("인벤토리", 100.0, 100.0)], Some(server), |_| {}, None);
        let events = collect(s.engine.scan(SCREEN).1).await;
        assert_eq!(items_of(&events)[0].source, LabelSource::Pending);
        let label = events.iter().position(|e| matches!(e, Event::ScanLabel { .. })).unwrap();
        let finished = events.iter().position(|e| matches!(e, Event::ScanLabelsFinished { .. })).unwrap();
        assert!(label < finished);
        assert_eq!(completions(&events), vec![(vec![0], None)]);
        assert!(finished < events.len() - 1, "loading ends before Done");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn declined_and_omitted_labels_finish_without_a_fake_model_failure() {
        let server = spawn_mock(|_, _| MockResponse { status: 200, chunks: sse_answer(&["0|?\n"]), delay_ms: 1 }).await;
        let s = setup_with_server(vec![line("인벤토리", 100.0, 100.0), line("상점", 100.0, 200.0)], Some(server), |_| {}, None);
        let events = collect(s.engine.scan(SCREEN).1).await;
        assert!(model_labels(&events).is_empty());
        assert_eq!(completions(&events), vec![(vec![0, 1], None)]);
        assert!(!events.iter().any(|e| matches!(e, Event::Error { .. })));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failed_batch_reports_only_its_own_items_and_keeps_the_server_error() {
        let server = spawn_mock(|_, body| {
            if body.contains("항목 24") {
                MockResponse { status: 503, chunks: vec![b"Model is unavailable".to_vec()], delay_ms: 1 }
            } else {
                let answer = (0..24).map(|n| format!("{n}|Label {n}\n")).collect::<String>();
                MockResponse { status: 200, chunks: sse_answer(&[&answer]), delay_ms: 1 }
            }
        }).await;
        let input = (0..26).map(|n| line(&format!("한국어 항목 {n}"), 100.0, 30.0 + n as f32 * 30.0)).collect();
        let s = setup_with_server(input, Some(server), |_| {}, None);
        let events = collect(s.engine.scan(SCREEN).1).await;
        let completed = completions(&events);
        let failed = completed.iter().find(|(_, error)| error.is_some()).unwrap();
        assert_eq!(failed.0, vec![24, 25]);
        assert!(failed.1.as_ref().unwrap().contains("503"));
        assert!(failed.1.as_ref().unwrap().contains("Model is unavailable"));
        let ready = completed.iter().find(|(_, error)| error.is_none()).unwrap();
        assert_eq!(ready.0, (0..24).collect::<Vec<_>>());
        assert_eq!(model_labels(&events).len(), 24);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_incomplete_stream_finishes_the_unanswered_items_with_an_error() {
        let server = spawn_mock(|_, _| MockResponse { status: 200,
            chunks: vec![maru_llm::testing::sse_event("0|Inventory\n")], delay_ms: 1 }).await;
        let s = setup_with_server(vec![line("인벤토리", 100.0, 100.0), line("상점", 100.0, 200.0)], Some(server), |_| {}, None);
        let events = collect(s.engine.scan(SCREEN).1).await;
        assert_eq!(model_labels(&events), vec![(0, "Inventory".into())]);
        let completed = completions(&events);
        assert_eq!(completed[0].0, vec![0, 1]);
        assert!(completed[0].1.is_some());
        assert!(matches!(events.last(), Some(Event::Done { .. })));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn every_copy_of_a_repeated_text_stops_waiting_together() {
        let s = setup(vec![line("인벤토리", 100.0, 100.0), line("인벤토리", 100.0, 200.0)], true, |_| {}, None).await;
        let events = collect(s.engine.scan(SCREEN).1).await;
        assert_eq!(s.model_requests(), 1);
        assert_eq!(completions(&events), vec![(vec![0, 1], None)]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_model_timeout_settles_the_pending_label_instead_of_leaving_it_loading() {
        let server = spawn_mock(|_, _| {
            let mut chunks = vec![b": waiting\n\n".to_vec()];
            chunks.extend(sse_answer(&["0|Inventory\n"]));
            MockResponse { status: 200, chunks, delay_ms: 1500 }
        }).await;
        let s = setup_with_server(vec![line("인벤토리", 100.0, 100.0)], Some(server),
            |cfg| cfg.llm.request_timeout_secs = 1, None);
        let events = collect(s.engine.scan(SCREEN).1).await;
        assert!(model_labels(&events).is_empty());
        let completed = completions(&events);
        assert_eq!(completed[0].0, vec![0]);
        let message = events.iter().find_map(|e| if let Event::Error { stage: Stage::Llm, message, .. } = e {
            Some(message.clone())
        } else { None }).unwrap();
        assert_eq!(completed[0].1, Some(message));
        assert!(matches!(events.last(), Some(Event::Done { .. })));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_join_finishes_only_its_new_item_under_the_original_scan_id() {
        let s = setup(vec![line("새", 100.0, 100.0), line("게임", 230.0, 100.0)], true, |_| {}, None).await;
        let (id, rx) = s.engine.scan(SCREEN);
        let items = items_of(&collect(rx).await);
        let pieces = items.iter().map(|i| maru_engine::joins::JoinPiece { text: i.text.clone(), rect: i.rect }).collect();
        let events = collect(s.engine.scan_join(id, 4, pieces)).await;
        assert_eq!(completions(&events), vec![(vec![4], None)]);
        assert!(events.iter().all(|e| e.id() == id));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn correcting_a_scan_uses_new_item_ids_and_remembers_only_the_exact_program_text() {
        use maru_engine::ocr_corrections::Target;
        let labels=Arc::new(LabelCache::open_in_memory().unwrap());
        let h=setup(vec![line("ㅅ컵",40.0,50.0)],true,|_|{},Some(labels.clone())).await;
        let (id,rx)=h.engine.scan(SCREEN);let original=items_of(&collect(rx).await).remove(0);
        let rx=h.engine.correct_scan(id,original.item,"확인",true).unwrap();
        let events=collect(rx).await;
        let fixed=items_of(&events).remove(0);
        assert_ne!(fixed.item,original.item);
        assert_eq!(fixed.rect,original.rect);
        assert_eq!(fixed.text,"확인");
        assert_eq!(completions(&events), vec![(vec![fixed.item], None)]);
        assert!(events.iter().any(|e|matches!(e,Event::ScanCorrection {item,..} if *item==original.item)));
        assert_eq!(labels.ocr_correction("game.exe","ㅅ컵").unwrap(),Some("확인".into()));
        assert_eq!(labels.ocr_correction("other.exe","ㅅ컵").unwrap(),None);
        let (next,rx)=h.engine.scan(SCREEN);let next_items=items_of(&collect(rx).await);
        assert_eq!(next_items[0].text,"확인");
        assert!(h.engine.correct_scan(id,fixed.item,"틀림",true).is_err());
        assert_eq!(labels.ocr_correction("game.exe","ㅅ컵").unwrap(),Some("확인".into()));
        h.engine.forget_ocr_fix(Target::Scan {id:next,item:next_items[0].item}).unwrap();
        assert_eq!(labels.ocr_correction("game.exe","ㅅ컵").unwrap(),None);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_one_time_scan_correction_does_not_create_a_saved_rule() {
        let labels=Arc::new(LabelCache::open_in_memory().unwrap());
        let h=setup(vec![line("ㅅ컵",40.0,50.0)],false,|_|{},Some(labels.clone())).await;
        let (id,rx)=h.engine.scan(SCREEN);let item=items_of(&collect(rx).await)[0].item;
        let events=collect(h.engine.correct_scan(id,item,"스킵",false).unwrap()).await;
        assert_eq!(items_of(&events)[0].text,"스킵");
        assert_eq!(labels.ocr_correction("game.exe","ㅅ컵").unwrap(),None);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_screen_fills_in_dictionary_first_then_model_labels_in_one_request() {
        let s = setup(vec![line("인벤토리", 100.0, 100.0), line("Menu", 100.0, 150.0), line("학교", 100.0, 200.0), line("확인", 900.0, 900.0)], true, |_| {}, None).await;
        let (_, rx) = s.engine.scan(SCREEN);
        let events = collect(rx).await;

        // order: started → items → labels → done, and nothing that is not Korean
        let order = names(&events);
        assert_eq!(order.first(), Some(&"scan_started"));
        assert!(first_index(&events, "scan_items") < first_index(&events, "scan_label"));
        assert_eq!(order.last(), Some(&"done"));
        let items = items_of(&events);
        assert_eq!(items.iter().map(|i| i.text.as_str()).collect::<Vec<_>>(), ["인벤토리", "학교", "확인"]);

        // the dictionary's meaning is on screen before the model has answered
        let school = items.iter().find(|i| i.text == "학교").unwrap();
        assert_eq!((school.label.as_str(), school.source), ("school", LabelSource::Dictionary));
        assert_eq!(items[0].source, LabelSource::Pending);

        // every string was asked in a single batched request, and answered
        assert_eq!(s.model_requests(), 1);
        let labels = model_labels(&events);
        assert!(labels.contains(&(0, "Inventory".into())) && labels.contains(&(2, "OK".into())) && labels.contains(&(1, "School".into())), "{labels:?}");
        assert!(events.iter().any(|e| matches!(e, Event::Timing { stage: Stage::Ocr, .. })));
    }

    fn rect_of(l: &OcrLine) -> Rect {
        l.rect
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn labels_joined_by_hand_become_one_item_that_is_translated_and_remembered() {
        use maru_engine::joins::JoinPiece;
        let (a, b) = (line("새", 100.0, 100.0), line("게임", 230.0, 100.0));
        let cache = Arc::new(LabelCache::open_in_memory().unwrap());
        let s = setup(vec![a.clone(), b.clone(), line("확인", 900.0, 900.0)], true, |_| {}, Some(cache.clone())).await;

        // the detector cut "새 게임" in two: three items on screen
        let (scan_id, rx) = s.engine.scan(SCREEN);
        let first = collect(rx).await;
        assert_eq!(items_of(&first).iter().map(|i| i.text.as_str()).collect::<Vec<_>>(), ["새", "게임", "확인"]);

        // the person joins the two labels (picked in the "wrong" order): one new item, number 3
        let pieces = vec![JoinPiece { text: "게임".into(), rect: rect_of(&b) }, JoinPiece { text: "새".into(), rect: rect_of(&a) }];
        let events = collect(s.engine.scan_join(scan_id, 3, pieces)).await;
        let joined = items_of(&events);
        assert_eq!(joined.len(), 1);
        assert_eq!((joined[0].item, joined[0].text.as_str()), (3, "새 게임"));
        assert_eq!(joined[0].rect, Rect::new(100.0, 100.0, 250.0, 22.0), "one box around both");
        assert!(events.iter().all(|e| !matches!(e, Event::ScanItems { id, .. } if *id != scan_id)), "it belongs to the scan that is on screen");
        assert_eq!(model_labels(&events), vec![(3, "New game".to_string())]);
        assert_eq!(names(&events).last(), Some(&"done"));
        assert_eq!(s.model_requests(), 2, "the scan, then the joined text");

        // the next scan of this screen makes the same join by itself, with the label already known
        let (second_id, rx) = s.engine.scan(SCREEN);
        let again = collect(rx).await;
        let items = items_of(&again);
        assert_eq!(items.iter().map(|i| i.text.as_str()).collect::<Vec<_>>(), ["새 게임", "확인"]);
        assert_eq!((items[0].label.as_str(), items[0].source), ("New game", LabelSource::Cache));
        assert_eq!(s.model_requests(), 2, "nothing new to ask");

        // splitting it again forgets the join
        assert_eq!(s.engine.forget_join(second_id, "새 게임"), 1);
        let (_, rx) = s.engine.scan(SCREEN);
        assert_eq!(items_of(&collect(rx).await).len(), 3);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_join_on_a_scan_that_is_no_longer_current_is_refused_and_remembers_nothing() {
        use maru_engine::joins::JoinPiece;
        let cache = Arc::new(LabelCache::open_in_memory().unwrap());
        let s = setup(vec![line("새", 100.0, 100.0), line("게임", 230.0, 100.0)], true, |_| {}, Some(cache.clone())).await;
        let (old_id, rx) = s.engine.scan(SCREEN);
        collect(rx).await;
        collect(s.engine.scan(SCREEN).1).await; // a newer scan replaced it

        let pieces = vec![JoinPiece { text: "새".into(), rect: Rect::new(100.0, 100.0, 120.0, 22.0) }, JoinPiece { text: "게임".into(), rect: Rect::new(230.0, 100.0, 120.0, 22.0) }];
        let events = collect(s.engine.scan_join(old_id, 2, pieces)).await;
        assert!(events.iter().any(|e| matches!(e, Event::Error { .. })));
        assert!(!events.iter().any(|e| matches!(e, Event::ScanItems { .. })));
        assert!(cache.joins_for("game.exe").is_empty());
        assert_eq!(s.engine.forget_join(old_id, "새 게임"), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_join_made_in_one_program_is_not_made_in_another() {
        let cache = Arc::new(LabelCache::open_in_memory().unwrap());
        cache.put_join("other.exe", &["새".to_string(), "게임".to_string()], "새 게임", 1);
        let s = setup(vec![line("새", 100.0, 100.0), line("게임", 230.0, 100.0)], true, |_| {}, Some(cache)).await;
        let items = items_of(&collect(s.engine.scan(SCREEN).1).await);
        assert_eq!(items.len(), 2, "the scan runs in game.exe, where nothing was joined");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_second_visit_to_the_same_screen_needs_no_model() {
        let cache = Arc::new(LabelCache::open_in_memory().unwrap());
        let s = setup(vec![line("인벤토리", 100.0, 100.0), line("확인", 300.0, 400.0)], true, |_| {}, Some(cache.clone())).await;
        collect(s.engine.scan(SCREEN).1).await;
        assert_eq!(s.model_requests(), 1);
        assert_eq!(cache.len(), 2, "the labels were remembered");

        let events = collect(s.engine.scan(SCREEN).1).await;
        assert_eq!(s.model_requests(), 1, "no second request");
        let items = items_of(&events);
        assert!(items.iter().all(|i| i.source == LabelSource::Cache), "{items:?}");
        assert_eq!(items.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(), ["Inventory", "OK"]);
        assert!(model_labels(&events).is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn glossary_entries_are_fixed_labels_and_never_asked() {
        let s = setup(
            vec![line("인벤토리", 100.0, 100.0), line("상점", 100.0, 200.0)],
            true,
            |cfg| {
                let mut p = AppProfile { name: "game".into(), exe: vec!["game.exe".into()], ..Default::default() };
                p.glossary.insert("인벤토리".into(), "Bag".into());
                cfg.profiles.push(p);
            },
            None,
        )
        .await;
        let events = collect(s.engine.scan(SCREEN).1).await;
        let items = items_of(&events);
        assert_eq!((items[0].label.as_str(), items[0].source), ("Bag", LabelSource::Glossary));
        let asked = s.server.as_ref().unwrap().requests.lock().unwrap().iter().map(|(_, b)| b.clone()).collect::<Vec<_>>();
        assert!(asked.iter().all(|b| !b.contains("0|인벤토리")), "the glossary word was not sent to the model: {asked:?}");
        assert_eq!(model_labels(&events), vec![(1, "Shop".to_string())]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn words_you_know_are_faded_or_hidden_and_not_sent_to_the_model() {
        for (mode, shown, known_flag) in [(GlossMode::FadeKnown, 2usize, true), (GlossMode::HideKnown, 1, false), (GlossMode::All, 2, false)] {
            let s = setup(vec![line("학교", 100.0, 100.0), line("상점", 100.0, 200.0)], true, |cfg| cfg.learning.gloss_mode = mode, None).await;
            s.vocab.mark_known("학교", true, maru_srs::now_secs()).unwrap();
            let events = collect(s.engine.scan(SCREEN).1).await;
            let items = items_of(&events);
            assert_eq!(items.len(), shown, "{mode:?}");
            if let Some(school) = items.iter().find(|i| i.text == "학교") {
                assert_eq!(school.known, known_flag, "{mode:?}");
            }
            let asked = s.server.as_ref().unwrap().requests.lock().unwrap().iter().map(|(_, b)| b.clone()).collect::<Vec<_>>();
            let sent_school = asked.iter().any(|b| b.contains("|학교"));
            assert_eq!(sent_school, mode == GlossMode::All, "{mode:?}: a known word is only sent when every word is wanted");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn without_a_model_the_dictionary_labels_still_appear_with_one_clear_error() {
        let s = setup(vec![line("학교", 100.0, 100.0), line("인벤토리", 100.0, 200.0)], false, |_| {}, None).await;
        let events = collect(s.engine.scan(SCREEN).1).await;
        assert_eq!(items_of(&events)[0].label, "school");
        let errors: Vec<_> = events.iter().filter(|e| matches!(e, Event::Error { stage: Stage::Llm, .. })).collect();
        assert_eq!(errors.len(), 1);
        let completed = completions(&events);
        assert_eq!(completed[0].0, vec![0, 1]);
        assert!(completed[0].1.as_ref().unwrap().contains("unavailable"));
        assert_eq!(names(&events).last(), Some(&"done"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_screen_with_no_korean_just_finishes() {
        let s = setup(vec![line("Press any key", 10.0, 10.0)], true, |_| {}, None).await;
        let events = collect(s.engine.scan(SCREEN).1).await;
        assert!(items_of(&events).is_empty());
        assert_eq!(s.model_requests(), 0, "nothing to ask");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_screenshot_can_be_scanned_exactly_like_the_screen() {
        let s = setup(vec![line("인벤토리", 100.0, 100.0)], true, |_| {}, None).await;
        let events = collect(s.engine.scan_image(image::RgbImage::new(800, 600)).1).await;
        let started = events.iter().find_map(|e| if let Event::ScanStarted { screen, .. } = e { Some(*screen) } else { None }).unwrap();
        assert_eq!(started, Rect::new(0.0, 0.0, 800.0, 600.0), "a picture is its own coordinate space");
        assert_eq!(items_of(&events)[0].rect.x, 100.0);
        assert_eq!(model_labels(&events), vec![(0, "Inventory".to_string())]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn no_ocr_engine_is_reported_not_hidden() {
        let server = spawn_mock(labeller).await;
        let cfg = Config::default();
        let engine = Engine::new(EngineParts {
            config: cfg.clone(),
            analyzer: Analyzer::heuristic(),
            dict: None,
            vocab: Arc::new(Vocab::open_in_memory(SrsConfig::default()).unwrap()),
            llm: Some(LlmClient::new(&server.url, &cfg.llm)),
            llm_status: None,
            platform: Platform { selection: Arc::new(FakeSelection::default()), screen: Arc::new(FakeScreen::new(vec![])), os_ocr: None },
            ocr: None,
            scan_ocr: None,
            live_ocr: None,
            live_llm: None,
            labels: None,
        });
        let events = collect(engine.scan(SCREEN).1).await;
        assert!(events.iter().any(|e| matches!(e, Event::Error { stage: Stage::Ocr, message, .. } if message.contains("OCR"))));
    }
}

/// An AnkiConnect that has the note type already and no words yet; every call is logged.
fn ankiconnect(_path: &str, body: &str) -> MockResponse {
    let req: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    let result = match req["action"].as_str().unwrap_or_default() {
        "version" => serde_json::json!(6),
        "deckNames" => serde_json::json!(["Default", "Korean"]),
        "modelNames" => serde_json::json!(["Maru Korean"]),
        "modelFieldNames" => serde_json::json!(maru_srs::anki::FIELDS),
        "modelStyling" => serde_json::json!({ "css": format!("/* maru-model-v{} */", maru_srs::anki::MODEL_VERSION) }),
        "findNotes" => serde_json::json!([]),
        "addNote" => serde_json::json!(77),
        "notesInfo" => serde_json::json!([{ "noteId": 77, "cards": [770, 771], "fields": {}, "tags": [] }]),
        _ => serde_json::json!(null),
    };
    MockResponse { status: 200, chunks: vec![serde_json::json!({ "result": result, "error": null }).to_string().into_bytes()], delay_ms: 0 }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_incomplete_anki_add_is_reported_and_retried_without_audio_or_another_note() {
    use maru_engine::anki_flow::{AnkiAddOptions, AnkiAdded};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let created = Arc::new(AtomicBool::new(false));
    let fail = Arc::new(AtomicBool::new(true));
    let adds = Arc::new(AtomicUsize::new(0));
    let (seen, broken, calls) = (created.clone(), fail.clone(), adds.clone());
    let server = spawn_mock(move |path, body| {
        let request: serde_json::Value = serde_json::from_str(body).unwrap();
        let action = request["action"].as_str().unwrap();
        let result = match action {
            "findNotes" => Some(if seen.load(Ordering::Acquire) { serde_json::json!([77]) } else { serde_json::json!([]) }),
            "addNote" => { calls.fetch_add(1, Ordering::SeqCst); seen.store(true, Ordering::Release); Some(serde_json::json!(77)) },
            "changeDeck" if broken.load(Ordering::Acquire) => return MockResponse {status:200,chunks:vec![br#"{"result":null,"error":"temporary move failure"}"#.to_vec()],delay_ms:0},
            _ => None,
        };
        result.map_or_else(|| ankiconnect(path,body), |result| MockResponse {status:200,chunks:vec![serde_json::json!({"result":result,"error":null}).to_string().into_bytes()],delay_ms:0})
    }).await;
    let mut cfg=Config::default();cfg.learning.anki.enabled=true;cfg.learning.anki.url=server.url;cfg.learning.anki.sync_after_add=false;
    let h=build(cfg,None,None);
    h.vocab.record_lookup(&maru_srs::LookupInfo {key:"학교".into(),surface:"학교".into(),meaning:Some("school".into()),..Default::default()},1).unwrap();
    let options=||AnkiAddOptions {key:"학교".into(),deck:"Korean".into(),tags:vec![],reverse:maru_core::config::ReverseMode::Subdeck,app:Some("game.exe".into()),word_audio:None,sentence_audio:None};
    let result=h.engine.anki_add(options()).await.unwrap();
    assert!(matches!(result,AnkiAdded::Added {warning:Some(_),cards:0,..}));
    assert_eq!(h.engine.anki_count().unwrap(),1);
    fail.store(false,Ordering::Release);
    let permit=h.engine.begin_anki_add("학교","Korean").await.unwrap();
    assert!(!permit.needs_audio());
    assert!(h.engine.begin_anki_add("학교","Korean").await.is_err());
    let result=h.engine.finish_anki_add(permit,options()).await.unwrap();
    assert!(matches!(result,AnkiAdded::Added {warning:None,cards:2,reverse_deck:Some(_),..}));
    assert_eq!(adds.load(Ordering::SeqCst),1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_anki_refresh_clears_missing_progress_and_a_failed_refresh_preserves_it() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let fail=Arc::new(AtomicBool::new(false));let flag=fail.clone();
    let server=spawn_mock(move |_,_|MockResponse {status:200,chunks:vec![if flag.load(Ordering::Acquire) { br#"{"result":null,"error":"unavailable"}"#.to_vec() } else { br#"{"result":[],"error":null}"#.to_vec() }],delay_ms:0}).await;
    let mut cfg=Config::default();cfg.learning.anki.enabled=true;cfg.learning.anki.url=server.url;
    let h=build(cfg,None,None);
    h.vocab.set_anki_state("학교",Some(45.0),1).unwrap();
    h.engine.anki_refresh().await.unwrap();
    assert_eq!(h.vocab.get("학교").unwrap().unwrap().anki_interval,None);
    h.vocab.set_anki_state("학교",Some(45.0),2).unwrap();
    fail.store(true,Ordering::Release);
    assert!(h.engine.anki_refresh().await.is_err());
    assert_eq!(h.vocab.get("학교").unwrap().unwrap().anki_interval,Some(45.0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_word_goes_to_the_chosen_deck_and_the_program_remembers_it() {
    use maru_engine::anki_flow::{AnkiAddOptions, AnkiAdded};
    let anki = spawn_mock(ankiconnect).await;
    let mut cfg = Config::default();
    cfg.learning.anki.enabled = true;
    cfg.learning.anki.note_type = "Basic".into(); // an old config: it is replaced by our own note type
    cfg.learning.anki.url = anki.url.clone();
    let h = build(cfg, None, None);
    h.vocab.record_lookup(&maru_srs::LookupInfo { key: "학교".into(), surface: "학교에서".into(), pos: "noun".into(), meaning: Some("school".into()), level: Some("TOPIK 1".into()), context: None }, 100).unwrap();

    let before = h.engine.anki_overview("학교", Some("hades.exe")).await.unwrap();
    assert!(before.connected && before.existing.is_none());
    assert_eq!(before.suggestions.first().map(|s| s.deck.as_str()), Some("Korean::Hades"), "named after the program the first time");
    assert!(before.suggestions.iter().any(|s| s.deck == "Korean" && s.exists));
    assert!(before.tags.contains(&"app::hades".to_string()) && before.tags.contains(&"topik-1".to_string()), "{:?}", before.tags);

    let opts = AnkiAddOptions { key: "학교".into(), deck: " Korean :: Games ".into(), tags: before.tags.clone(), reverse: before.reverse, app: Some("hades.exe".into()), word_audio: None, sentence_audio: None };
    let added = h.engine.anki_add(opts).await.unwrap();
    assert!(matches!(&added, AnkiAdded::Added { deck, reverse_deck: Some(r), cards: 2, sync: true, warning: None } if deck == "Korean::Games" && r == "Korean::Games::Reverse"), "{added:?}");

    let word = h.vocab.get("학교").unwrap().unwrap();
    assert!(word.saved && word.anki_interval == Some(0.0), "the word is saved and in Anki");
    let after = h.engine.anki_overview("학교", Some("hades.exe")).await.unwrap();
    assert_eq!(after.suggestions.first().map(|s| s.deck.as_str()), Some("Korean::Games"), "next time the same program suggests the same deck");

    let calls: Vec<String> = anki.requests.lock().unwrap().iter().filter_map(|(_, b)| serde_json::from_str::<serde_json::Value>(b).ok()).map(|v| v["action"].as_str().unwrap_or_default().to_string()).collect();
    assert!(calls.contains(&"createDeck".to_string()) && calls.contains(&"changeDeck".to_string()), "{calls:?}");
    assert!(h.engine.anki_add(AnkiAddOptions { key: "학교".into(), deck: "  ".into(), tags: vec![], reverse: before.reverse, app: None, word_audio: None, sentence_audio: None }).await.unwrap_err().to_string().contains("deck"), "no deck, no add");
}

// ───────────── live subtitles ─────────────

fn live_line(text: &str) -> maru_engine::LiveLine {
    let mut capture = Capture::typed(text);
    capture.source = CaptureSource::Continuous;
    maru_engine::LiveLine { capture, appeared: std::time::Instant::now(), ready_ms: 120, ocr_ms: 30 }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_live_line_is_translation_only_and_goes_to_the_history_with_its_timings() {
    let h = harness().await;
    let events = collect(h.engine.translate_live(live_line(SENTENCE)).1).await;

    assert_eq!(names(&events).first(), Some(&"started"));
    assert!(names(&events).contains(&"translation") && names(&events).contains(&"translation_done"));
    assert!(!names(&events).iter().any(|n| matches!(*n, "breakdown" | "gloss" | "note")), "no breakdown or word meanings in the live lane: {:?}", names(&events));
    assert_eq!(text_of(&events, Variant::Primary), "I studied at school.");
    assert_eq!(h.requests().len(), 1, "one model call, not a translation plus word meanings");
    assert!(!h.requests()[0].contains("Words:"));

    let history = h.engine.history();
    assert_eq!(history.len(), 1);
    assert!(history[0].live);
    assert_eq!(history[0].translation, "I studied at school.");
    let (first, total) = (history[0].first_ms.expect("time to the first word"), history[0].total_ms.expect("time to the end"));
    assert!(first <= total, "{first} ≤ {total}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn earlier_live_lines_reach_the_model_as_chat_turns() {
    let h = harness().await;
    collect(h.engine.translate_live(live_line(SENTENCE)).1).await;
    collect(h.engine.translate_live(live_line("저는 학생이에요")).1).await;
    let reqs = h.requests();
    assert!(!reqs[0].contains("\"assistant\""), "the first line has no earlier lines");
    assert!(reqs[1].contains("\"assistant\"") && reqs[1].contains("I studied at school.") && reqs[1].contains(SENTENCE), "{}", reqs[1]);
    assert!(!reqs[1].contains("Previous lines"), "turns, not the single-message list of the normal lane");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_context_can_be_switched_off() {
    let h = harness_with(model, |c| c.continuous.context_lines = 0).await;
    collect(h.engine.translate_live(live_line(SENTENCE)).1).await;
    collect(h.engine.translate_live(live_line("저는 학생이에요")).1).await;
    assert!(!h.requests()[1].contains("\"assistant\""));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_live_line_that_comes_back_is_answered_from_the_cache() {
    let h = harness().await;
    collect(h.engine.translate_live(live_line(SENTENCE)).1).await;
    let events = collect(h.engine.translate_live(live_line(SENTENCE)).1).await;
    let Event::Started { cached, .. } = &events[0] else { panic!() };
    assert!(*cached);
    assert_eq!(h.requests().len(), 1, "no second model call");
    assert_eq!(text_of(&events, Variant::Primary), "I studied at school.");
    assert_eq!(h.engine.history().len(), 1, "an immediate repeat is the same entry");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_live_cache_does_not_hide_word_meanings_from_a_normal_translation() {
    let h = harness().await;
    collect(h.engine.translate_live(live_line(SENTENCE)).1).await;
    let events = collect(h.engine.translate(Capture::typed(SENTENCE)).1).await;
    let Event::Started { cached, .. } = &events[0] else { panic!() };
    assert!(!*cached, "the normal lane has its own entry");
    assert_eq!(events.iter().filter(|e| matches!(e, Event::Gloss { .. })).count(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn english_on_a_korean_screen_is_left_alone_by_the_live_lane() {
    let h = harness().await;
    let events = collect(h.engine.translate_live(live_line("Press A to continue")).1).await;
    assert_eq!(names(&events), ["done"]);
    assert!(h.requests().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_lines_do_not_cancel_each_other() {
    let h = harness().await;
    let (_, a) = h.engine.translate_live(live_line(SENTENCE));
    let (_, b) = h.engine.translate_live(live_line("저는 학생이에요"));
    let (a, b) = tokio::join!(collect(a), collect(b));
    assert!(names(&a).contains(&"translation_done") && names(&b).contains(&"translation_done"));
    assert_eq!(h.engine.history().len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_active_live_translation_keeps_pending_lines_and_gives_them_the_previous_answer() {
    let h = harness_with(|_, _| MockResponse { status: 200, chunks: sse_answer(&["The answer."]), delay_ms: 50 }, |c| c.continuous.max_in_flight = 1).await;
    let (first_id, a) = h.engine.translate_live(live_line(SENTENCE));
    let (second_id, b) = h.engine.translate_live(live_line("저는 학생이에요"));
    let (a, b) = tokio::join!(collect(a), collect(b));
    assert!(names(&a).contains(&"translation_done") && names(&b).contains(&"translation_done"));
    let requests = h.requests();
    assert_eq!(requests.len(), 2);
    assert!(!requests[0].contains("\"assistant\""));
    assert!(requests[1].contains(SENTENCE) && requests[1].contains("The answer.") && requests[1].contains("\"assistant\""), "{}", requests[1]);
    let history = h.engine.history();
    assert_eq!(history.iter().map(|i| i.id).collect::<Vec<_>>(), [first_id, second_id]);
    assert!(history.iter().all(|i| i.appeared_at.is_some_and(|t| t <= i.at)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_active_live_translations_leave_the_third_waiting_without_losing_it() {
    let h = harness_with(|_, _| MockResponse { status: 200, chunks: sse_answer(&["First ", "second ", "third."]), delay_ms: 200 }, |c| c.continuous.max_in_flight = 2).await;
    let (_, a) = h.engine.translate_live(live_line(SENTENCE));
    let (_, b) = h.engine.translate_live(live_line("저는 학생이에요"));
    let (_, c) = h.engine.translate_live(live_line("안녕하세요"));
    tokio::time::timeout(Duration::from_secs(2), async {
        while h.requests().len() < 2 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }).await.expect("the first two requests start");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(h.requests().len(), 2, "the third cannot reach the model while both places are occupied");
    let (a, b, c) = tokio::join!(collect(a), collect(b), collect(c));
    assert!([a, b, c].iter().all(|e| names(e).contains(&"translation_done")));
    assert_eq!(h.requests().len(), 3);
    assert_eq!(h.engine.history().len(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_live_request_releases_the_place_for_the_next_line() {
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let h = harness_with(move |_, _| {
        if calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            MockResponse { status: 503, chunks: vec![b"unavailable".to_vec()], delay_ms: 0 }
        } else {
            MockResponse { status: 200, chunks: sse_answer(&["An answer."]), delay_ms: 0 }
        }
    }, |c| c.continuous.max_in_flight = 1).await;
    let (_, a) = h.engine.translate_live(live_line(SENTENCE));
    let (_, b) = h.engine.translate_live(live_line("저는 학생이에요"));
    let (a, b) = tokio::join!(collect(a), collect(b));
    assert!(names(&a).contains(&"error"));
    assert_eq!(text_of(&b, Variant::Primary), "An answer.");
    assert_eq!(h.requests().len(), 2);
    assert_eq!(h.engine.history().len(), 1);
}

struct CountingOcr(std::sync::atomic::AtomicUsize);

impl maru_ocr::OcrEngine for CountingOcr {
    fn name(&self) -> &'static str { "counting" }
    fn recognize(&self, _: &image::RgbImage) -> maru_ocr::Result<maru_ocr::OcrOutput> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(maru_ocr::OcrOutput { lines: vec![maru_ocr::OcrLine { text: SENTENCE.into(), rect: maru_core::Rect::new(0.0, 0.0, 100.0, 20.0), words: vec![], confidence: 1.0 }] })
    }
}

fn ocr_engine(cpu_ocr: bool, normal: Arc<CountingOcr>, live: Option<Arc<CountingOcr>>) -> Engine {
    let mut config = Config::default();
    config.continuous.cpu_ocr = cpu_ocr;
    config.continuous.interval_ms = 50;
    config.continuous.stable_ms = 50;
    Engine::new(EngineParts {
        config, analyzer: Analyzer::heuristic(), dict: None,
        vocab: Arc::new(Vocab::open_in_memory(SrsConfig::default()).unwrap()),
        llm: None, llm_status: None,
        platform: Platform { selection: Arc::new(FakeSelection::default()), screen: Arc::new(FakeScreen::new(vec![FakeScreen::blank(100, 30)])), os_ocr: None },
        ocr: Some(normal), scan_ocr: None, live_ocr: live.map(|o| o as Arc<dyn maru_ocr::OcrEngine>), labels: None, live_llm: None,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn opting_into_cpu_live_ocr_leaves_manual_region_ocr_on_its_original_engine() {
    let normal = Arc::new(CountingOcr(std::sync::atomic::AtomicUsize::new(0)));
    let cpu = Arc::new(CountingOcr(std::sync::atomic::AtomicUsize::new(0)));
    let engine = ocr_engine(true, normal.clone(), Some(cpu.clone()));
    let region = maru_core::Rect::new(0.0, 0.0, 100.0, 30.0);
    engine.capture_region(region).await.unwrap();
    let before = normal.0.load(std::sync::atomic::Ordering::SeqCst);
    assert!(before > 0);
    let mut watcher = engine.start_continuous(region).unwrap();
    let event = tokio::time::timeout(Duration::from_secs(2), watcher.rx.recv()).await.unwrap().unwrap();
    watcher.stop();
    assert!(matches!(event, maru_engine::LiveEvent::Line(_)));
    assert!(cpu.0.load(std::sync::atomic::Ordering::SeqCst) > 0);
    assert_eq!(normal.0.load(std::sync::atomic::Ordering::SeqCst), before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unavailable_cpu_live_ocr_is_reported_instead_of_silently_changing_recognition() {
    let normal = Arc::new(CountingOcr(std::sync::atomic::AtomicUsize::new(0)));
    let engine = ocr_engine(true, normal, None);
    let err = engine.start_continuous(maru_core::Rect::new(0.0, 0.0, 100.0, 30.0)).err().expect("missing CPU OCR");
    assert!(err.to_string().contains("CPU PP-OCR for live subtitles is unavailable"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_subtitles_keep_the_original_ocr_until_cpu_ocr_is_requested() {
    let normal = Arc::new(CountingOcr(std::sync::atomic::AtomicUsize::new(0)));
    let cpu = Arc::new(CountingOcr(std::sync::atomic::AtomicUsize::new(0)));
    let engine = ocr_engine(false, normal.clone(), Some(cpu.clone()));
    let mut watcher = engine.start_continuous(maru_core::Rect::new(0.0, 0.0, 100.0, 30.0)).unwrap();
    let event = tokio::time::timeout(Duration::from_secs(2), watcher.rx.recv()).await.unwrap().unwrap();
    watcher.stop();
    assert!(matches!(event, maru_engine::LiveEvent::Line(_)));
    assert!(normal.0.load(std::sync::atomic::Ordering::SeqCst) > 0);
    assert_eq!(cpu.0.load(std::sync::atomic::Ordering::SeqCst), 0);
}
