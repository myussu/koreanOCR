//! A pass-through in front of a real AnkiConnect, for the opt-in tests that touch a person's Anki.
//!
//! The tests talk to this address instead of AnkiConnect's. Reads go straight through. Before every
//! call that changes something it asks Anki which profile is open and refuses unless it is the one
//! the test was told to use; it lets a change through only if it stays inside the throwaway deck
//! (or Maru's own note types and the sound files this run stored), and it never lets a sync or a
//! profile switch through. A test can also make a named call fail once, to see how Maru recovers.

// Shared by several test files; each uses a part of it.
#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Calls that only read.
const READS: [&str; 19] = [
    "version", "getActiveProfile", "deckNames", "deckNamesAndIds", "getDecks", "modelNames", "modelFieldNames", "modelStyling", "modelTemplates",
    "findNotes", "findCards", "notesInfo", "cardsInfo", "getIntervals", "areSuspended", "areDue", "getMediaFilesNames", "retrieveMediaFile", "apiReflect",
];

/// Calls that are never forwarded, whatever the profile.
const NEVER: [&str; 6] = ["sync", "loadProfile", "guiExitAnki", "importPackage", "exportPackage", "reloadCollection"];

struct State {
    upstream: String,
    profile: String,
    deck: String,
    http: reqwest::Client,
    fail: Mutex<Vec<String>>,
    stored: Mutex<Vec<String>>,
    refused: Mutex<Vec<String>>,
    calls: Mutex<Vec<String>>,
}

pub struct GuardedAnki {
    /// The address the code under test uses as its AnkiConnect.
    pub url: String,
    state: Arc<State>,
}

impl GuardedAnki {
    /// `profile` is the only Anki profile changes may go to; `deck` (and its subdecks) the only deck.
    pub async fn start(upstream: &str, profile: &str, deck: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free local port");
        let url = format!("http://{}", listener.local_addr().expect("the local address"));
        let state = Arc::new(State {
            upstream: upstream.to_string(),
            profile: profile.to_string(),
            deck: deck.to_string(),
            http: reqwest::Client::new(),
            fail: Mutex::default(),
            stored: Mutex::default(),
            refused: Mutex::default(),
            calls: Mutex::default(),
        });
        let shared = state.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let state = shared.clone();
                tokio::spawn(async move {
                    let _ = serve(stream, state).await;
                });
            }
        });
        Self { url, state }
    }

    /// The next call of this action fails without reaching Anki.
    pub fn fail_next(&self, action: &str) {
        self.state.fail.lock().unwrap().push(action.to_string());
    }

    /// Why calls were refused; empty when every change was allowed.
    pub fn refused(&self) -> Vec<String> {
        self.state.refused.lock().unwrap().clone()
    }

    /// Every action that was asked for, in order.
    pub fn calls(&self) -> Vec<String> {
        self.state.calls.lock().unwrap().clone()
    }

    /// The sound files this run put into Anki's media folder.
    pub fn stored_media(&self) -> Vec<String> {
        self.state.stored.lock().unwrap().clone()
    }

    /// Ask the real AnkiConnect directly; for looking at the result. Changes go through [`Self::url`].
    pub async fn read(&self, action: &str, params: Value) -> Value {
        assert!(READS.contains(&action), "{action} is not a read: send it through the guard");
        self.state.ask(action, params).await.unwrap_or_else(|e| panic!("{action}: {e}"))
    }

    /// A change sent through the guard, as the code under test would send it.
    pub async fn change(&self, action: &str, params: Value) -> Result<Value, String> {
        let reply: Value = self
            .state
            .http
            .post(&self.url)
            .json(&json!({ "action": action, "version": 6, "params": params }))
            .send()
            .await
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        match reply["error"].as_str() {
            Some(error) => Err(error.to_string()),
            None => Ok(reply["result"].clone()),
        }
    }
}

impl State {
    async fn forward(&self, body: &Value) -> Result<Value, String> {
        let reply = self.http.post(&self.upstream).json(body).send().await.map_err(|e| format!("Anki did not answer: {e}"))?;
        reply.json().await.map_err(|e| format!("Anki did not answer in JSON: {e}"))
    }

    async fn ask(&self, action: &str, params: Value) -> Result<Value, String> {
        let reply = self.forward(&json!({ "action": action, "version": 6, "params": params })).await?;
        match reply["error"].as_str() {
            Some(error) => Err(error.to_string()),
            None => Ok(reply["result"].clone()),
        }
    }

    fn in_deck(&self, deck: &str) -> bool {
        deck == self.deck || deck.starts_with(&format!("{}::", self.deck))
    }

    /// Every card given must be in the throwaway deck.
    async fn cards_in_deck(&self, cards: &Value) -> Result<(), String> {
        let ids = cards.as_array().filter(|a| !a.is_empty()).ok_or("no cards were named")?;
        let infos = self.ask("cardsInfo", json!({ "cards": ids })).await?;
        let infos = infos.as_array().filter(|a| a.len() == ids.len()).ok_or("Anki did not describe every card")?;
        match infos.iter().find(|c| !self.in_deck(c["deckName"].as_str().unwrap_or_default())) {
            Some(card) => Err(format!("a card is in the deck \"{}\", outside \"{}\"", card["deckName"].as_str().unwrap_or("?"), self.deck)),
            None => Ok(()),
        }
    }

    async fn notes_in_deck(&self, notes: &Value) -> Result<(), String> {
        let ids = notes.as_array().filter(|a| !a.is_empty()).ok_or("no notes were named")?;
        let infos = self.ask("notesInfo", json!({ "notes": ids })).await?;
        let infos = infos.as_array().filter(|a| a.len() == ids.len()).ok_or("Anki did not describe every note")?;
        let cards: Vec<Value> = infos.iter().flat_map(|n| n["cards"].as_array().cloned().unwrap_or_default()).collect();
        self.cards_in_deck(&Value::Array(cards)).await
    }

    fn remember_media(&self, audio: &Value) {
        let mut stored = self.stored.lock().unwrap();
        for file in audio.as_array().into_iter().flatten().filter_map(|a| a["filename"].as_str()) {
            if !stored.iter().any(|s| s == file) {
                stored.push(file.to_string());
            }
        }
    }

    /// Whether this change stays inside what the test may touch.
    async fn allowed(&self, action: &str, p: &Value) -> Result<(), String> {
        let maru_type = |name: &Value| name.as_str().is_some_and(|n| n.starts_with("Maru Korean"));
        match action {
            "createDeck" => self.in_deck(p["deck"].as_str().unwrap_or_default()).then_some(()).ok_or_else(|| format!("the deck \"{}\" is not the throwaway deck", p["deck"])),
            "deleteDecks" => {
                let decks = p["decks"].as_array().filter(|a| !a.is_empty()).ok_or("no decks were named")?;
                decks.iter().all(|d| self.in_deck(d.as_str().unwrap_or_default())).then_some(()).ok_or_else(|| "only the throwaway deck may be deleted".to_string())
            }
            "addNote" => {
                let note = &p["note"];
                if !self.in_deck(note["deckName"].as_str().unwrap_or_default()) {
                    return Err(format!("the note goes to the deck {}, outside the throwaway deck", note["deckName"]));
                }
                self.remember_media(&note["audio"]);
                Ok(())
            }
            "changeDeck" => {
                if !self.in_deck(p["deck"].as_str().unwrap_or_default()) {
                    return Err(format!("cards would move to the deck {}, outside the throwaway deck", p["deck"]));
                }
                self.cards_in_deck(&p["cards"]).await
            }
            "updateNoteFields" => {
                self.notes_in_deck(&json!([p["note"]["id"]])).await?;
                self.remember_media(&p["note"]["audio"]);
                Ok(())
            }
            "deleteNotes" | "addTags" | "removeTags" => self.notes_in_deck(&p["notes"]).await,
            "suspend" | "unsuspend" | "forgetCards" | "setDueDate" => self.cards_in_deck(&p["cards"]).await,
            "deleteMediaFile" => {
                let file = p["filename"].as_str().unwrap_or_default();
                self.stored.lock().unwrap().iter().any(|s| s == file).then_some(()).ok_or_else(|| format!("{file} was not stored by this run"))
            }
            "createModel" => maru_type(&p["modelName"]).then_some(()).ok_or_else(|| "only Maru's note types may be created".to_string()),
            "updateModelTemplates" | "updateModelStyling" => maru_type(&p["model"]["name"]).then_some(()).ok_or_else(|| "only Maru's note types may be updated".to_string()),
            "modelFieldAdd" => maru_type(&p["modelName"]).then_some(()).ok_or_else(|| "only Maru's note types may get a field".to_string()),
            other => Err(format!("{other} is not a change these tests are allowed to make")),
        }
    }

    async fn handle(&self, body: Value) -> Value {
        let action = body["action"].as_str().unwrap_or_default().to_string();
        self.calls.lock().unwrap().push(action.clone());
        let refuse = |why: String| {
            self.refused.lock().unwrap().push(format!("{action}: {why}"));
            json!({ "result": null, "error": format!("refused by the test guard: {why}") })
        };
        if NEVER.contains(&action.as_str()) || action == "multi" {
            return refuse("this call is never sent to a real Anki by the tests".into());
        }
        {
            let mut fail = self.fail.lock().unwrap();
            if let Some(at) = fail.iter().position(|a| *a == action) {
                fail.remove(at);
                return json!({ "result": null, "error": format!("{action} failed (made to fail by the test)") });
            }
        }
        if !READS.contains(&action.as_str()) {
            match self.ask("getActiveProfile", json!({})).await {
                Ok(open) if open.as_str() == Some(self.profile.as_str()) => {}
                Ok(open) => return refuse(format!("the open Anki profile is {open}, not \"{}\"", self.profile)),
                Err(e) => return refuse(format!("could not ask which profile is open: {e}")),
            }
            if let Err(why) = self.allowed(&action, &body["params"]).await {
                return refuse(why);
            }
        }
        match self.forward(&body).await {
            Ok(reply) => reply,
            Err(e) => json!({ "result": null, "error": e }),
        }
    }
}

async fn serve(mut stream: TcpStream, state: Arc<State>) -> std::io::Result<()> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 16 * 1024];
    let (head_end, length) = loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(at) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..at]).to_ascii_lowercase();
            let length = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
            break (at + 4, length);
        }
    };
    while buf.len() < head_end + length {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let reply = match serde_json::from_slice::<Value>(&buf[head_end..]) {
        Ok(body) => state.handle(body).await,
        Err(e) => json!({ "result": null, "error": format!("the request was not JSON: {e}") }),
    };
    let body = reply.to_string();
    let head = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body.as_bytes()).await?;
    stream.shutdown().await
}
