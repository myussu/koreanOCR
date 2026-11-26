//! The guard the real-Anki tests write through (`support/guarded_anki.rs`), against a stand-in AnkiConnect:
//! it is what keeps those tests out of the wrong profile and the wrong deck, so it is tested itself.

#[path = "support/guarded_anki.rs"]
mod guarded_anki;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use guarded_anki::GuardedAnki;
use serde_json::{Value, json};

const DECK: &str = "Maru test (safe to delete)";

/// An AnkiConnect with `profile` open whose cards are all in `card_deck`; it records what reaches it.
fn anki(profile: &'static str, card_deck: &'static str) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut buf = Vec::new();
            let mut chunk = [0u8; 8192];
            let body = loop {
                let Ok(n) = stream.read(&mut chunk) else { break None };
                buf.extend_from_slice(&chunk[..n]);
                if let Some(at) = buf.windows(4).position(|w| w == b"\r\n\r\n")
                    && let Ok(body) = serde_json::from_slice::<Value>(&buf[at + 4..])
                {
                    break Some(body);
                }
                if n == 0 {
                    break None;
                }
            };
            let Some(body) = body else { continue };
            let action = body["action"].as_str().unwrap_or_default().to_string();
            let result = match action.as_str() {
                "getActiveProfile" => json!(profile),
                "cardsInfo" => json!(body["params"]["cards"].as_array().unwrap().iter().map(|_| json!({ "deckName": card_deck })).collect::<Vec<_>>()),
                "notesInfo" => json!(body["params"]["notes"].as_array().unwrap().iter().map(|_| json!({ "cards": [1] })).collect::<Vec<_>>()),
                _ => json!(null),
            };
            log.lock().unwrap().push(action);
            let reply = json!({ "result": result, "error": null }).to_string();
            let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).as_bytes());
        }
    });
    (url, seen)
}

fn note_in(deck: &str) -> Value {
    json!({ "note": { "deckName": deck, "modelName": "Maru Korean", "fields": {}, "audio": [{ "filename": "maru_test_1.wav" }] } })
}

#[tokio::test]
async fn with_another_profile_open_reads_pass_and_every_change_is_refused() {
    let (url, seen) = anki("User 1", DECK);
    let guard = GuardedAnki::start(&url, "User 2", DECK).await;
    assert!(guard.change("deckNames", json!({})).await.is_ok(), "a read needs no profile");
    let refused = guard.change("addNote", note_in(DECK)).await.unwrap_err();
    assert!(refused.contains("\"User 1\"") && refused.contains("User 2"), "{refused}");
    assert!(guard.change("deleteDecks", json!({ "decks": [DECK] })).await.is_err());
    let reached = seen.lock().unwrap().clone();
    assert!(!reached.iter().any(|a| a == "addNote" || a == "deleteDecks"), "nothing was changed in the other profile: {reached:?}");
    assert_eq!(guard.refused().len(), 2);
}

#[tokio::test]
async fn in_the_right_profile_a_change_must_stay_in_the_throwaway_deck() {
    let (url, seen) = anki("User 2", "Coreano::2 Media");
    let guard = GuardedAnki::start(&url, "User 2", DECK).await;
    assert!(guard.change("addNote", note_in(&format!("{DECK}::Games"))).await.is_ok(), "a subdeck of the throwaway deck is inside it");
    assert!(guard.change("addNote", note_in("Coreano::2 Media")).await.unwrap_err().contains("outside the throwaway deck"));
    assert!(guard.change("createDeck", json!({ "deck": "Coreano::2 Media" })).await.is_err());
    assert!(guard.change("deleteDecks", json!({ "decks": [DECK, "Default"] })).await.is_err(), "one deck outside is enough to refuse");
    // The cards and notes Anki reports here live in a real deck.
    for (action, params) in [
        ("deleteNotes", json!({ "notes": [5] })),
        ("updateNoteFields", json!({ "note": { "id": 5, "fields": {} } })),
        ("suspend", json!({ "cards": [9] })),
        ("changeDeck", json!({ "cards": [9], "deck": DECK })),
    ] {
        let refused = guard.change(action, params).await.unwrap_err();
        assert!(refused.contains("Coreano::2 Media"), "{action}: {refused}");
    }
    assert!(guard.change("deleteMediaFile", json!({ "filename": "someone_elses.mp3" })).await.is_err(), "only sounds this run stored");
    assert!(guard.change("deleteMediaFile", json!({ "filename": "maru_test_1.wav" })).await.is_ok());
    assert!(guard.change("createModel", json!({ "modelName": "Basic" })).await.is_err());
    assert!(guard.change("updateModelStyling", json!({ "model": { "name": "Maru Korean", "css": "" } })).await.is_ok());
    let reached = seen.lock().unwrap().clone();
    let changes: Vec<&String> = reached.iter().filter(|a| !["getActiveProfile", "cardsInfo", "notesInfo"].contains(&a.as_str())).collect();
    assert_eq!(changes, ["addNote", "deleteMediaFile", "updateModelStyling"], "only the allowed changes reached Anki");
}

#[tokio::test]
async fn a_sync_or_a_profile_switch_never_reaches_anki_and_a_call_can_be_made_to_fail_once() {
    let (url, seen) = anki("User 2", DECK);
    let guard = GuardedAnki::start(&url, "User 2", DECK).await;
    for action in ["sync", "loadProfile", "multi", "guiBrowse"] {
        assert!(guard.change(action, json!({})).await.is_err(), "{action}");
    }
    guard.fail_next("changeDeck");
    assert!(guard.change("changeDeck", json!({ "cards": [9], "deck": DECK })).await.unwrap_err().contains("made to fail"));
    assert!(guard.change("changeDeck", json!({ "cards": [9], "deck": DECK })).await.is_ok(), "only once");
    let reached = seen.lock().unwrap().clone();
    assert!(!reached.iter().any(|a| ["sync", "loadProfile", "multi", "guiBrowse"].contains(&a.as_str())), "{reached:?}");
    assert_eq!(reached.iter().filter(|a| *a == "changeDeck").count(), 1);
}
