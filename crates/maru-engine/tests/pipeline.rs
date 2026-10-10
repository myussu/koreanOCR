//! The whole pipeline, against a scripted stand-in for llama-server.

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

fn build(cfg: Config, llm: Option<LlmClient>, server: Option<MockServer>) -> Harness {
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
        let table = [("인벤토리", "Inventory"), ("확인", "OK"), ("상점", "Shop"), ("학교", "School")];
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
    assert!(matches!(&added, AnkiAdded::Added { deck, reverse_deck: Some(r), cards: 2, sync: true } if deck == "Korean::Games" && r == "Korean::Games::Reverse"), "{added:?}");

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
