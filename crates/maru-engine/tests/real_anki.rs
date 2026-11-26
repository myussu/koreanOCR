//! The engine's Anki path against a real Anki with AnkiConnect: the manual tests M-9, M-24, M-39 and M-40 as far as
//! they can run without a person (word card, add recovery, cards from a line, sounds added later, refresh progress).
//!
//! Skipped unless `MARU_REAL_ANKI=1` with Anki open, and `MARU_REAL_ANKI_PROFILE` names the profile that may be
//! written to. Every change goes through `maru-srs/tests/support/guarded_anki.rs`: refused when another profile is
//! open, when it would leave the throwaway deck, and always for a sync. The model is scripted (word meanings from a
//! real model are a separate check). `MARU_TEST_WORD_WAV` and `MARU_TEST_LINE_WAV` attach real speech files (made
//! with `maru tts`); without them the sounds are generated silence. It leaves no notes, deck or sound files behind;
//! Maru's two note types stay (AnkiConnect cannot delete a note type).
//!
//! Use an empty test profile: the test stops if Anki already has the three words it adds.

#[path = "../../maru-srs/tests/support/guarded_anki.rs"]
mod guarded_anki;

use std::io::Cursor;
use std::sync::Arc;

use guarded_anki::GuardedAnki;
use maru_core::config::{Config, ReverseMode};
use maru_dict::Dictionary;
use maru_engine::anki_flow::{AnkiAddOptions, AnkiAdded, Media};
use maru_engine::mining::{MineChoice, MineLine, MinedSentence};
use maru_engine::{Engine, EngineParts};
use maru_lang::Analyzer;
use maru_llm::LlmClient;
use maru_llm::testing::{MockResponse, sse_answer, spawn_mock};
use maru_platform::Platform;
use maru_platform::testing::{FakeScreen, FakeSelection};
use maru_srs::{Context, LookupInfo, SrsConfig, Vocab, now_secs};
use serde_json::{Value, json};

const DECK: &str = "Maru test (safe to delete)";
const SENTENCE: &str = "저는 학교에서 공부했어요";
const TRANSLATION: &str = "I studied at school.";
const WORDS: [&str; 3] = ["저", "학교", "공부하다"];
const KAIKKI: &str = r#"
{"word":"학교","lang_code":"ko","pos":"noun","senses":[{"glosses":["school"]}]}
{"word":"공부하다","lang_code":"ko","pos":"verb","senses":[{"glosses":["to study; to learn"]}]}
{"word":"저","lang_code":"ko","pos":"pronoun","senses":[{"glosses":["I (humble)"]}]}
"#;

/// The word meanings a model would give for the line.
fn model(_path: &str, body: &str) -> MockResponse {
    let answer: Vec<&str> = if body.contains("Words:") { vec!["0|I|\n1|school|\n2|to study|past tense\n"] } else { vec!["?"] };
    MockResponse { status: 200, chunks: sse_answer(&answer), delay_ms: 1 }
}

/// A tenth of a second of silence, unless a real speech file was given.
fn sound(name: &str, real: &str) -> Media {
    if let Some(path) = std::env::var_os(real) {
        let data = std::fs::read(&path).unwrap_or_else(|e| panic!("{real}: {e}"));
        return Media { filename: name.into(), data };
    }
    let samples = 1600u32;
    let mut data = Vec::new();
    data.extend_from_slice(b"RIFF");
    data.extend_from_slice(&(36 + samples * 2).to_le_bytes());
    data.extend_from_slice(b"WAVEfmt ");
    data.extend_from_slice(&16u32.to_le_bytes());
    data.extend_from_slice(&1u16.to_le_bytes());
    data.extend_from_slice(&1u16.to_le_bytes());
    data.extend_from_slice(&16000u32.to_le_bytes());
    data.extend_from_slice(&32000u32.to_le_bytes());
    data.extend_from_slice(&2u16.to_le_bytes());
    data.extend_from_slice(&16u16.to_le_bytes());
    data.extend_from_slice(b"data");
    data.extend_from_slice(&(samples * 2).to_le_bytes());
    data.resize(data.len() + samples as usize * 2, 0);
    Media { filename: name.into(), data }
}

struct Run {
    engine: Engine,
    vocab: Arc<Vocab>,
    guard: Arc<GuardedAnki>,
    deck: String,
    stamp: i64,
}

impl Run {
    /// The note of a word (or of the line), as Anki describes it.
    async fn note(&self, query: &str) -> Value {
        let ids = self.guard.read("findNotes", json!({ "query": query })).await;
        let ids = ids.as_array().cloned().unwrap_or_default();
        assert_eq!(ids.len(), 1, "one note for {query}: {ids:?}");
        self.guard.read("notesInfo", json!({ "notes": ids })).await[0].clone()
    }

    async fn word_note(&self, word: &str) -> Value {
        self.note(&format!("\"note:Maru Korean\" \"Word:{word}\"")).await
    }

    async fn decks_of(&self, note: &Value) -> Vec<String> {
        let cards = self.guard.read("cardsInfo", json!({ "cards": note["cards"] })).await;
        cards.as_array().unwrap().iter().map(|c| c["deckName"].as_str().unwrap_or_default().to_string()).collect()
    }

    fn look_up(&self, key: &str, surface: &str, pos: &str, meaning: &str, level: Option<&str>) {
        let context = Context { sentence: SENTENCE.into(), translation: Some(TRANSLATION.into()), app: Some("game.exe".into()), screenshot: None };
        let info = LookupInfo { key: key.into(), surface: surface.into(), pos: pos.into(), meaning: Some(meaning.into()), level: level.map(str::to_owned), context: Some(context) };
        self.vocab.record_lookup(&info, now_secs()).unwrap();
    }

    fn options(&self, key: &str, tags: Vec<String>, word_audio: Option<Media>, sentence_audio: Option<Media>) -> AnkiAddOptions {
        AnkiAddOptions { key: key.into(), deck: self.deck.clone(), tags, reverse: ReverseMode::Subdeck, app: Some("game.exe".into()), word_audio, sentence_audio, context: None }
    }
}

fn field(note: &Value, name: &str) -> String {
    note["fields"][name]["value"].as_str().unwrap_or_default().to_string()
}

fn tags(note: &Value) -> Vec<String> {
    note["tags"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_anki_path_works_in_a_real_anki_from_a_word_card_to_sounds_added_later() {
    if std::env::var_os("MARU_REAL_ANKI").is_none() {
        eprintln!("skipped: set MARU_REAL_ANKI=1 with Anki open to run");
        return;
    }
    let Ok(profile) = std::env::var("MARU_REAL_ANKI_PROFILE") else {
        panic!("set MARU_REAL_ANKI_PROFILE to the Anki profile this test may write to (it must be the one open in Anki)");
    };
    let anki_url = std::env::var("MARU_ANKI_URL").unwrap_or_else(|_| "http://127.0.0.1:8765".into());
    let guard = Arc::new(GuardedAnki::start(&anki_url, &profile, DECK).await);
    for word in WORDS {
        let found = guard.read("findNotes", json!({ "query": format!("\"Word:{word}\"") })).await;
        assert!(found.as_array().is_some_and(Vec::is_empty), "this Anki profile already has {word}: use an empty test profile");
    }

    let outcome = tokio::spawn(checks(guard.clone())).await;

    // Leave nothing behind, whatever happened above.
    let leftovers = guard.read("findNotes", json!({ "query": format!("\"deck:{DECK}\"") })).await;
    if leftovers.as_array().is_some_and(|a| !a.is_empty())
        && let Err(e) = guard.change("deleteNotes", json!({ "notes": leftovers })).await
    {
        eprintln!("clean-up deleteNotes: {e}");
    }
    let decks = guard.read("deckNames", json!({})).await;
    if decks.as_array().is_some_and(|a| a.iter().any(|d| d == DECK))
        && let Err(e) = guard.change("deleteDecks", json!({ "decks": [DECK], "cardsToo": true })).await
    {
        eprintln!("clean-up deleteDecks: {e}");
    }
    for file in guard.stored_media() {
        if let Err(e) = guard.change("deleteMediaFile", json!({ "filename": file })).await {
            eprintln!("clean-up deleteMediaFile: {e}");
        }
    }
    if let Err(e) = outcome {
        std::panic::resume_unwind(e.into_panic());
    }
    assert!(guard.refused().is_empty(), "the guard refused: {:?}", guard.refused());
    assert!(!guard.calls().iter().any(|c| c == "sync"), "nothing asked Anki to sync");
}

async fn checks(guard: Arc<GuardedAnki>) {
    let server = spawn_mock(model).await;
    let mut cfg = Config::default();
    cfg.learning.anki.enabled = true;
    cfg.learning.anki.url = guard.url.clone();
    cfg.learning.anki.deck = DECK.into();
    cfg.learning.anki.sync_after_add = false;
    let dict = Dictionary::open_in_memory().unwrap();
    dict.import_kaikki(Cursor::new(KAIKKI), |_| {}).unwrap();
    dict.import_levels(Cursor::new("학교\t1\n")).unwrap();
    let vocab = Arc::new(Vocab::open_in_memory(SrsConfig::from_learning(&cfg.learning)).unwrap());
    let llm = LlmClient::new(&server.url, &cfg.llm);
    let engine = Engine::new(EngineParts {
        config: cfg,
        analyzer: Analyzer::heuristic(),
        dict: Some(Arc::new(dict)),
        vocab: vocab.clone(),
        llm: Some(llm),
        llm_status: None,
        platform: Platform { selection: Arc::new(FakeSelection::default()), screen: Arc::new(FakeScreen::new(vec![])), os_ocr: None },
        ocr: None,
        scan_ocr: None,
        live_ocr: None,
        live_llm: None,
        labels: None,
    });
    let stamp = now_secs();
    let run = Run { engine, vocab, guard, deck: format!("{DECK}::Games"), stamp };

    // M-9: Check Anki finds it and has both note types in place.
    let report = run.engine.anki_check().await.expect("Anki is open with AnkiConnect");
    assert!(report.contains("\"Maru Korean\"") && report.contains("\"Maru Korean Sentence\""), "{report}");
    eprintln!("check: {report}");

    word_card(&run).await;
    recovery(&run).await;
    let (pronoun_id, sentence_id) = cards_from_a_line(&run).await;
    sounds_later(&run, pronoun_id, sentence_id).await;
    progress(&run).await;
}

/// M-9: a word goes to the chosen deck with its fields, tags, sounds and a reverse card; it is added once.
async fn word_card(run: &Run) {
    run.look_up("학교", "학교에서", "noun", "school", Some("TOPIK 1"));
    let before = run.engine.anki_overview("학교", Some("game.exe")).await.unwrap();
    assert!(before.connected && before.existing.is_none(), "{:?}", before.error);
    let files = (format!("maru_test_{}_w.wav", run.stamp), format!("maru_test_{}_s.wav", run.stamp));
    let opts = run.options("학교", before.tags.clone(), Some(sound(&files.0, "MARU_TEST_WORD_WAV")), Some(sound(&files.1, "MARU_TEST_LINE_WAV")));
    let added = run.engine.anki_add(opts.clone()).await.expect("the word is added");
    let reverse = format!("{}::Reverse", run.deck);
    assert!(matches!(&added, AnkiAdded::Added { deck, reverse_deck: Some(r), cards: 2, sync: false, warning: None } if *deck == run.deck && *r == reverse), "{added:?}");

    let note = run.word_note("학교").await;
    assert_eq!(field(&note, "Meaning"), "school");
    assert!(field(&note, "Sentence").contains("<b>학교에서</b>"), "{}", field(&note, "Sentence"));
    assert_eq!(field(&note, "Translation"), TRANSLATION);
    assert!(field(&note, "WordAudio").contains("[sound:") && field(&note, "SentenceAudio").contains("[sound:"), "both sounds are attached");
    let has = tags(&note);
    for tag in ["maru", "topik-1", "pos::noun", "app::game"] {
        assert!(has.iter().any(|t| t == tag), "tag {tag} in {has:?}");
    }
    assert_eq!(run.decks_of(&note).await, [run.deck.clone(), reverse], "the English → Korean card is in the Reverse subdeck");
    let word = run.vocab.get("학교").unwrap().unwrap();
    assert!(word.saved && word.anki_interval == Some(0.0), "Maru knows the word is in Anki");

    let again = run.engine.anki_add(opts).await.unwrap();
    assert!(matches!(again, AnkiAdded::Exists { .. }), "the same word is not added twice: {again:?}");
    eprintln!("word card: added with sounds, tags {has:?}; a second add reports it exists");
}

/// M-24 step 5: a card setup that fails half way says so, and the retry finishes the same note.
async fn recovery(run: &Run) {
    run.look_up("공부하다", "공부했어요", "verb", "to study", None);
    run.guard.fail_next("changeDeck");
    let first = run.engine.anki_add(run.options("공부하다", vec!["maru".into()], None, None)).await.unwrap();
    let AnkiAdded::Added { warning: Some(warning), cards: 0, .. } = &first else { panic!("an unfinished add is reported: {first:?}") };
    assert!(warning.contains("unfinished"), "{warning}");
    let note = run.word_note("공부하다").await;
    assert_eq!(run.decks_of(&note).await, [run.deck.clone(), run.deck.clone()], "the note exists; its reverse card has not moved yet");

    let permit = run.engine.begin_anki_add("공부하다", &run.deck).await.expect("the retry is allowed");
    assert!(!permit.needs_audio(), "the retry makes no new sound");
    let second = run.engine.finish_anki_add(permit, run.options("공부하다", vec!["maru".into()], None, None)).await.unwrap();
    assert!(matches!(&second, AnkiAdded::Added { warning: None, cards: 2, reverse_deck: Some(_), .. }), "{second:?}");
    let after = run.word_note("공부하다").await;
    assert_eq!(after["noteId"], note["noteId"], "the same note, not a second one");
    assert_eq!(run.decks_of(&after).await, [run.deck.clone(), format!("{}::Reverse", run.deck)]);
    eprintln!("recovery: \"{warning}\" then the retry finished note {}", note["noteId"]);
}

/// M-39: the sheet knows what Anki has, a picked word becomes a card with this line, the line becomes a sentence
/// card, and doing it again adds nothing.
async fn cards_from_a_line(run: &Run) -> (u64, u64) {
    let sheet = run.engine.mine_sheet(SENTENCE, TRANSLATION, Some("game.exe")).await.unwrap();
    assert!(sheet.anki_error.is_none() && sheet.note.is_none(), "{:?} {:?}", sheet.anki_error, sheet.note);
    let word = |key: &str| sheet.words.iter().find(|w| w.key == key).unwrap_or_else(|| panic!("{key} in {:?}", sheet.words));
    assert!(word("학교").in_anki && !word("학교").pick && word("공부하다").in_anki, "words Anki has are not ticked");
    assert!(!word("저").in_anki && word("저").pick && word("저").in_context, "{:?}", word("저"));
    assert!(!sheet.sentence_in_anki && sheet.decks.contains(&run.deck));

    // Reverse cards off, as with `reverse = "off"`: the word gets its Korean → English card only.
    let line = MineLine { sentence: SENTENCE.into(), translation: TRANSLATION.into(), app: Some("game.exe".into()), deck: run.deck.clone(), reverse: ReverseMode::Off };
    let choice = MineChoice { key: "저".into(), surface: "저는".into(), pos: word("저").pos.clone(), level: None, meaning: "I (said humbly)".into() };
    let permit = run.engine.begin_mine_word(&line, &choice).await.unwrap();
    assert!(matches!(run.engine.finish_mine_word(permit, &line, &choice, None, None).await.unwrap(), AnkiAdded::Added { cards: 1, reverse_deck: None, warning: None, .. }));
    assert_eq!(run.engine.mine_sentence(&line, std::slice::from_ref(&choice), None).await.unwrap(), MinedSentence::Added { deck: run.deck.clone() });

    let pronoun = run.word_note("저").await;
    assert_eq!(field(&pronoun, "Meaning"), "I (said humbly)", "the meaning as edited on the sheet");
    assert!(field(&pronoun, "Sentence").contains("<b>저는</b>"), "{}", field(&pronoun, "Sentence"));
    assert_eq!(run.decks_of(&pronoun).await, std::slice::from_ref(&run.deck), "one card, in the chosen deck");
    let sentence = run.note(&format!("\"note:Maru Korean Sentence\" \"deck:{}\"", run.deck)).await;
    assert_eq!(field(&sentence, "Sentence"), SENTENCE);
    assert_eq!(field(&sentence, "Translation"), TRANSLATION);
    assert!(field(&sentence, "Words").contains("<b>저</b> — I (said humbly)"), "{}", field(&sentence, "Words"));
    assert_eq!(sentence["cards"].as_array().unwrap().len(), 1, "a sentence has no reverse card");
    assert!(tags(&sentence).iter().any(|t| t == "sentence"), "{:?}", tags(&sentence));

    let again = run.engine.mine_sheet(SENTENCE, TRANSLATION, Some("game.exe")).await.unwrap();
    assert!(again.sentence_in_anki && again.words.iter().all(|w| w.in_anki && !w.pick), "everything is in Anki now: {:?}", again.words);
    let permit = run.engine.begin_mine_word(&line, &choice).await.unwrap();
    assert!(matches!(run.engine.finish_mine_word(permit, &line, &choice, None, None).await.unwrap(), AnkiAdded::Exists { .. }));
    assert!(matches!(run.engine.mine_sentence(&line, std::slice::from_ref(&choice), None).await.unwrap(), MinedSentence::Exists { .. }));
    eprintln!("cards from a line: word card and sentence card added; a second time both exist");
    (pronoun["noteId"].as_u64().unwrap(), sentence["noteId"].as_u64().unwrap())
}

/// M-40: cards added without sound are found, get their sound, and a sound already there is kept.
async fn sounds_later(run: &Run, pronoun_id: u64, sentence_id: u64) {
    let silent = run.engine.anki_silent_notes(200).await.unwrap();
    let ids: Vec<u64> = silent.notes.iter().map(|n| n.note_id).collect();
    let school = run.word_note("학교").await["noteId"].as_u64().unwrap();
    assert!(ids.contains(&pronoun_id) && ids.contains(&sentence_id) && !ids.contains(&school), "silent notes: {ids:?}");
    let pronoun = silent.notes.iter().find(|n| n.note_id == pronoun_id).unwrap();
    assert_eq!((pronoun.word.as_deref(), pronoun.sentence.as_deref()), (Some("저"), Some(SENTENCE)), "the texts to speak, without markup");

    let word = sound(&format!("maru_test_{}_later_w.wav", run.stamp), "MARU_TEST_WORD_WAV");
    let line = sound(&format!("maru_test_{}_later_s.wav", run.stamp), "MARU_TEST_LINE_WAV");
    assert!(run.engine.anki_add_sounds(pronoun_id, Some(&word), Some(&line)).await.unwrap());
    assert!(run.engine.anki_add_sounds(sentence_id, None, Some(&line)).await.unwrap());
    let after = run.engine.anki_silent_notes(200).await.unwrap();
    assert!(!after.notes.iter().any(|n| n.note_id == pronoun_id || n.note_id == sentence_id), "they have their sounds now");
    let note = run.word_note("저").await;
    assert!(field(&note, "WordAudio").contains("[sound:") && field(&note, "SentenceAudio").contains("[sound:"));
    let kept = field(&note, "WordAudio");
    assert!(!run.engine.anki_add_sounds(pronoun_id, Some(&word), Some(&line)).await.unwrap(), "nothing left to fill");
    assert_eq!(field(&run.word_note("저").await, "WordAudio"), kept, "a sound already there is kept");
    eprintln!("sounds later: {} silent notes found, 2 filled, none of them listed afterwards", silent.total);
}

/// M-24 step 4: Refresh progress follows what happened in Anki: a suspended card keeps its place without counting
/// as learned, a deleted note is forgotten.
async fn progress(run: &Run) {
    assert_eq!(run.engine.anki_refresh().await.unwrap(), 3, "three word cards of ours");
    assert_eq!(run.engine.anki_count().unwrap(), 3);
    let interval = |key: &str| run.vocab.get(key).unwrap().unwrap().anki_interval;
    assert_eq!((interval("저"), interval("학교"), interval("공부하다")), (Some(0.0), Some(0.0), Some(0.0)), "new cards: in Anki, not learned yet");

    let pronoun = run.word_note("저").await;
    run.guard.change("suspend", json!({ "cards": pronoun["cards"] })).await.expect("suspend a test card");
    let study = run.word_note("공부하다").await;
    run.guard.change("deleteNotes", json!({ "notes": [study["noteId"]] })).await.expect("delete a test note");
    assert_eq!(run.engine.anki_refresh().await.unwrap(), 2);
    assert_eq!(run.engine.anki_count().unwrap(), 2, "the deleted note's word is no longer counted");
    assert_eq!((interval("저"), interval("학교"), interval("공부하다")), (None, Some(0.0), None), "suspended: no progress; deleted: forgotten");
    let overview = run.engine.anki_overview("저", Some("game.exe")).await.unwrap();
    assert!(overview.existing.is_some(), "a suspended word is still in Anki, so it is not offered again");
    eprintln!("progress: 3 words, then 2 after a suspension and a deletion made in Anki");
}

/// M-9, last row: with nothing listening where Anki should be, the check answers at once and says what to do.
#[tokio::test]
async fn with_anki_closed_the_check_answers_quickly_in_plain_words() {
    let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
    let mut cfg = Config::default();
    cfg.learning.anki.enabled = true;
    cfg.learning.anki.url = format!("http://{free}");
    let vocab = Arc::new(Vocab::open_in_memory(SrsConfig::from_learning(&cfg.learning)).unwrap());
    let engine = Engine::new(EngineParts {
        config: cfg,
        analyzer: Analyzer::heuristic(),
        dict: None,
        vocab,
        llm: None,
        llm_status: None,
        platform: Platform { selection: Arc::new(FakeSelection::default()), screen: Arc::new(FakeScreen::new(vec![])), os_ocr: None },
        ocr: None,
        scan_ocr: None,
        live_ocr: None,
        live_llm: None,
        labels: None,
    });
    let started = std::time::Instant::now();
    let error = engine.anki_check().await.expect_err("nothing is listening").to_string();
    assert!(started.elapsed() < std::time::Duration::from_secs(10), "no hang: {:?}", started.elapsed());
    assert!(error.contains("cannot reach AnkiConnect") && error.contains("Is Anki open"), "it says what is wrong and what to check: {error}");
}
