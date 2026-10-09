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
    assert!(h.engine.export_anki("학교").await.is_err(), "disabled by default with an actionable message");
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
    });
    let events = collect(engine.translate(Capture::typed(SENTENCE)).1).await;
    assert!(events.iter().any(|e| matches!(e, Event::Error { message, .. } if message.contains("out of VRAM"))));
    assert!(events.iter().any(|e| matches!(e, Event::Breakdown { .. })));
}
