//! End-to-end check against a real Anki with AnkiConnect, in a throwaway deck that is deleted afterwards.
//!
//! Skipped unless `MARU_REAL_ANKI=1` (Anki must be open; `MARU_ANKI_URL` overrides the address) and
//! `MARU_REAL_ANKI_PROFILE` names the Anki profile that may be written to: every change goes through
//! `support/guarded_anki.rs`, which refuses it when another profile is open or when it would leave the throwaway
//! deck. It creates the `Maru Korean` and `Maru Korean Sentence` note types if they are missing, as Maru does on
//! first use (AnkiConnect cannot delete a note type, so those stay), and leaves no notes, deck or sound files
//! behind. It never asks Anki to sync.

#[path = "support/guarded_anki.rs"]
mod guarded_anki;

use maru_core::WordState;
use maru_core::config::{AnkiConfig, ReverseMode};
use maru_srs::anki_sentence::{SentenceAdded, SentenceRequest, SentenceWord};
use maru_srs::{AddRequest, Added, AnkiClient, Context, Media, WordRecord, now_secs};
use serde_json::{Value, json};

const DECK: &str = "Maru test (safe to delete)";

async fn raw(url: &str, action: &str, params: Value) -> Value {
    let reply: Value = reqwest::Client::new()
        .post(url)
        .json(&json!({ "action": action, "version": 6, "params": params }))
        .send()
        .await
        .expect("AnkiConnect answers")
        .json()
        .await
        .expect("AnkiConnect replies in JSON");
    assert!(reply["error"].is_null(), "{action}: {}", reply["error"]);
    reply["result"].clone()
}

/// A tenth of a second of silence: a real, playable WAV file.
fn wav(name: &str) -> Media {
    let samples = 1600u32;
    let mut data = Vec::new();
    data.extend_from_slice(b"RIFF");
    data.extend_from_slice(&(36 + samples * 2).to_le_bytes());
    data.extend_from_slice(b"WAVEfmt ");
    data.extend_from_slice(&16u32.to_le_bytes());
    data.extend_from_slice(&1u16.to_le_bytes()); // PCM
    data.extend_from_slice(&1u16.to_le_bytes()); // mono
    data.extend_from_slice(&16000u32.to_le_bytes());
    data.extend_from_slice(&32000u32.to_le_bytes());
    data.extend_from_slice(&2u16.to_le_bytes());
    data.extend_from_slice(&16u16.to_le_bytes());
    data.extend_from_slice(b"data");
    data.extend_from_slice(&(samples * 2).to_le_bytes());
    data.resize(data.len() + samples as usize * 2, 0);
    Media { filename: name.into(), data }
}

fn word(key: &str, meaning: &str) -> WordRecord {
    WordRecord {
        key: key.into(),
        surface: key.into(),
        pos: "noun".into(),
        meaning: Some(meaning.into()),
        level: None,
        lookups: 1,
        saved: false,
        marked_known: false,
        stability: None,
        difficulty: None,
        due: None,
        reps: 0,
        lapses: 0,
        anki_interval: None,
        state: WordState::New,
    }
}

/// Ask AnkiConnect for something during clean-up; a failure is reported, not fatal.
async fn tidy(url: &str, action: &str, params: Value) {
    let reply = reqwest::Client::new().post(url).json(&json!({ "action": action, "version": 6, "params": params })).send().await;
    match reply {
        Ok(r) => match r.json::<Value>().await {
            Ok(v) if v["error"].is_null() => {}
            other => eprintln!("clean-up {action}: {other:?}"),
        },
        Err(e) => eprintln!("clean-up {action}: {e}"),
    }
}

#[tokio::test]
async fn word_cards_sentence_cards_and_sounds_added_later_work_in_a_real_anki() {
    if std::env::var_os("MARU_REAL_ANKI").is_none() {
        eprintln!("skipped: set MARU_REAL_ANKI=1 with Anki open to run");
        return;
    }
    let Ok(profile) = std::env::var("MARU_REAL_ANKI_PROFILE") else {
        panic!("set MARU_REAL_ANKI_PROFILE to the Anki profile this test may write to (it must be the one open in Anki)");
    };
    let anki_url = std::env::var("MARU_ANKI_URL").unwrap_or_else(|_| "http://127.0.0.1:8765".into());
    let guard = guarded_anki::GuardedAnki::start(&anki_url, &profile, DECK).await;
    let url = guard.url.clone();
    let stamp = now_secs();
    let files = [format!("maru_test_{stamp}_w.wav"), format!("maru_test_{stamp}_s.wav"), format!("maru_test_{stamp}_later.wav")];
    let outcome = tokio::spawn(checks(url.clone(), stamp, files.clone())).await;

    // Leave nothing behind, whatever happened above.
    let reply: Value = reqwest::Client::new()
        .post(&url)
        .json(&json!({ "action": "findNotes", "version": 6, "params": { "query": format!("\"deck:{DECK}\"") } }))
        .send()
        .await
        .expect("AnkiConnect answers")
        .json()
        .await
        .expect("JSON");
    let leftovers: Vec<u64> = reply["result"].as_array().map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
    if !leftovers.is_empty() {
        tidy(&url, "deleteNotes", json!({ "notes": leftovers })).await;
    }
    // Deleting the deck takes its Reverse subdeck with it.
    tidy(&url, "deleteDecks", json!({ "decks": [DECK], "cardsToo": true })).await;
    for f in guard.stored_media() {
        tidy(&url, "deleteMediaFile", json!({ "filename": f })).await;
    }
    if let Err(e) = outcome {
        std::panic::resume_unwind(e.into_panic());
    }
    assert!(guard.refused().is_empty(), "the guard refused: {:?}", guard.refused());
    assert!(!guard.calls().iter().any(|c| c == "sync"), "nothing asked Anki to sync");
}

async fn checks(url: String, stamp: i64, files: [String; 3]) {
    let anki = AnkiClient::new(AnkiConfig { enabled: true, url: url.clone(), sync_after_add: false, ..Default::default() });
    // Made-up words, so a real collection never has them already.
    let (heard, silent) = (format!("마루시험{stamp}"), format!("마루조용{stamp}"));
    let line = format!("이것은 마루시험{stamp} 문장이에요");
    anki.ping().await.expect("Anki is open with AnkiConnect");
    let base = AddRequest {
        word: word(&heard, "a test word"),
        contexts: vec![Context { sentence: line.clone(), translation: Some("This is a test sentence.".into()), ..Default::default() }],
        pronunciation: None,
        deck: DECK.into(),
        tags: vec!["maru".into(), "maru-test".into()],
        reverse: ReverseMode::Subdeck,
        word_audio: Some(wav(&files[0])),
        sentence_audio: Some(wav(&files[1])),
        source: Some("Maru test".into()),
    };
    // A word card with both sounds and its reverse card in the Reverse subdeck.
    let Added::New { note_id, reverse_deck, cards, warning, .. } = anki.add(&base).await.expect("word added") else { panic!("the test word existed already") };
    assert_eq!((cards, warning), (2, None));
    assert_eq!(reverse_deck.as_deref(), Some(format!("{DECK}::Reverse").as_str()));
    let info = raw(&url, "notesInfo", json!({ "notes": [note_id] })).await;
    let field = |name: &str| info[0]["fields"][name]["value"].as_str().unwrap_or_default().to_string();
    assert!(field("WordAudio").contains("[sound:"), "word sound: {}", field("WordAudio"));
    assert!(field("SentenceAudio").contains("[sound:"), "sentence sound: {}", field("SentenceAudio"));
    assert!(field("Sentence").contains(&format!("<b>{heard}</b>")), "{}", field("Sentence"));
    let decks: Vec<String> = raw(&url, "cardsInfo", json!({ "cards": info[0]["cards"] })).await.as_array().unwrap().iter().map(|c| c["deckName"].as_str().unwrap().to_string()).collect();
    assert_eq!(decks, [DECK.to_string(), format!("{DECK}::Reverse")]);
    assert!(matches!(anki.add(&base).await.unwrap(), Added::Exists(_)), "the same word is not added twice");

    // A word and a sentence added without sound are found, and get their sound later.
    let quiet = AddRequest { word: word(&silent, "a quiet word"), word_audio: None, sentence_audio: None, reverse: ReverseMode::Off, ..base.clone() };
    let Added::New { note_id: quiet_id, .. } = anki.add(&quiet).await.expect("silent word added") else { panic!("the quiet word existed already") };
    let sentence = SentenceRequest { sentence: line.clone(), translation: "This is a test sentence.".into(), words: vec![SentenceWord { word: heard.clone(), meaning: "a test word".into() }], deck: DECK.into(), tags: vec!["maru-test".into()], audio: None, source: Some("Maru test".into()) };
    let SentenceAdded::New { note_id: sentence_id, .. } = anki.add_sentence(&sentence).await.expect("sentence added") else { panic!("the test line existed already") };
    assert_eq!(raw(&url, "notesInfo", json!({ "notes": [sentence_id] })).await[0]["cards"].as_array().unwrap().len(), 1, "no reverse card for sentences");
    assert!(matches!(anki.add_sentence(&sentence).await.unwrap(), SentenceAdded::Exists(_)), "the same line is not added twice");

    let found = anki.silent_notes(500).await.expect("silent search");
    let ids: Vec<u64> = found.notes.iter().map(|n| n.note_id).collect();
    assert!(ids.contains(&quiet_id) && ids.contains(&sentence_id) && !ids.contains(&note_id), "silent: {ids:?}");
    let quiet_note = found.notes.iter().find(|n| n.note_id == quiet_id).unwrap();
    assert_eq!(quiet_note.word.as_deref(), Some(silent.as_str()));
    assert_eq!(quiet_note.sentence.as_deref(), Some(line.as_str()));
    assert!(anki.add_sounds(sentence_id, None, Some(&wav(&files[2]))).await.unwrap());
    let after = anki.silent_notes(500).await.unwrap();
    assert!(!after.notes.iter().any(|n| n.note_id == sentence_id), "the sentence has its sound now");
    let sound = raw(&url, "notesInfo", json!({ "notes": [sentence_id] })).await[0]["fields"]["SentenceAudio"]["value"].as_str().unwrap_or_default().to_string();
    assert!(sound.contains("[sound:"), "{sound}");
    assert!(!anki.add_sounds(sentence_id, None, Some(&wav(&files[2]))).await.unwrap(), "a sound already there is kept");
}
