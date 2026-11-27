//! Making Anki cards from a line in History (live or captured): the shell adds the sounds (speech lives here) around
//! the engine's mining steps, and reports each card separately so one failure does not hide the others. Cards that
//! were added without sound get it later from `anki_fill_sounds`. The panel's "Make cards" hands its line to History
//! through `open_cards` / `take_pending_cards`.

use std::sync::{Arc, Mutex};

use maru_core::config::ReverseMode;
use maru_engine::anki_flow::{AnkiAdded, Media};
use maru_engine::mining::{MineChoice, MineLine, MineSheet, MinedSentence};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::state::AppState;

type S<'a> = State<'a, Arc<AppState>>;

/// A line the panel wants cards from: History opens its sheet when it next draws.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CardsLine {
    /// The capture's request id, which is also its History item's id.
    pub id: Option<u64>,
    pub sentence: String,
    pub translation: String,
    pub app: Option<String>,
}

/// At most one line waits: a second "Make cards" replaces the first, which was never opened.
static PENDING_CARDS: Mutex<Option<CardsLine>> = Mutex::new(None);

fn put_pending(line: CardsLine) -> Result<(), String> {
    if line.sentence.trim().is_empty() {
        return Err("There is no Korean line here to make cards from.".into());
    }
    *PENDING_CARDS.lock().unwrap_or_else(|p| p.into_inner()) = Some(line);
    Ok(())
}

fn take_pending() -> Option<CardsLine> {
    PENDING_CARDS.lock().unwrap_or_else(|p| p.into_inner()).take()
}

/// The panel's "Make cards": open History with this line's cards sheet.
#[tauri::command]
pub async fn open_cards(app: AppHandle, line: CardsLine) -> Result<(), String> {
    put_pending(line)?;
    crate::windows::open_hub(&app, "history").map_err(|e| e.to_string())
}

/// The line the panel asked for, once (History calls it every time it draws).
#[tauri::command]
pub fn take_pending_cards() -> Option<CardsLine> {
    take_pending()
}

/// The words of a line with their meaning in it, and what Anki already has.
#[tauri::command]
pub async fn mine_sheet(state: S<'_>, sentence: String, translation: String, app: Option<String>, ask: Option<maru_engine::routes::CardAsk>) -> Result<MineSheet, String> {
    state.engine().mine_sheet_with(&sentence, &translation, app.as_deref(), &ask.unwrap_or_default()).await.map_err(|e| e.to_string())
}

/// The line translated again for its sentence card ("Retry with …"; or the card-sentence connection when one is set).
#[tauri::command]
pub async fn mine_sentence_translation(state: S<'_>, sentence: String, app: Option<String>, ask: Option<maru_engine::routes::CardAsk>) -> Result<maru_engine::mining::CardSentence, String> {
    Ok(state.engine().card_sentence(&sentence, app.as_deref(), &ask.unwrap_or_default()).await)
}

/// Which connection and model each task uses and what is known about it; nothing is asked.
#[tauri::command]
pub fn task_status(state: S<'_>) -> Vec<maru_engine::routes::TaskStatus> {
    state.engine().task_status()
}

/// New cards already waiting in the two decks the sheet shows (asked again when a deck is typed). Only reads Anki.
#[tauri::command]
pub async fn mine_queue(state: S<'_>, word_deck: String, sentence_deck: String) -> Result<maru_engine::mining::CardQueue, String> {
    state.engine().card_queue(&word_deck, &sentence_deck).await.map_err(|e| e.to_string())
}

/// The settings' Test button: one short fixed question to the task's connection.
#[tauri::command]
pub async fn task_test(state: S<'_>, task: maru_core::tasks::Task) -> Result<maru_engine::routes::TaskStatus, String> {
    Ok(state.engine().test_task(task).await)
}

/// The settings' "Test your choice": the same question to a connection picked on the page but not saved yet.
#[tauri::command]
pub async fn connection_test(state: S<'_>, connection: String) -> Result<maru_engine::routes::ConnectionCheck, String> {
    Ok(state.engine().test_connection(&connection).await)
}

#[derive(Debug, Deserialize)]
pub struct MineAddRequest {
    /// The saved live line, marked as made into cards when something was added.
    pub saved_id: Option<i64>,
    pub sentence: String,
    pub translation: String,
    pub app: Option<String>,
    pub deck: String,
    /// The deck of the sentence card; absent: the same deck as the words.
    #[serde(default)]
    pub sentence_deck: Option<String>,
    /// Reverse cards for the words (sentences never get one).
    pub reverse: bool,
    pub audio: bool,
    pub sentence_card: bool,
    pub words: Vec<MineChoice>,
}

/// What happened to one card.
#[derive(Debug, Serialize)]
pub struct MineOutcome {
    pub what: String,
    /// `word` or `sentence`.
    pub kind: &'static str,
    /// `added`, `exists` or `failed`.
    pub result: &'static str,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct MineReply {
    pub outcomes: Vec<MineOutcome>,
    /// Why some cards have no sound.
    pub audio_note: Option<String>,
}

/// The configured reverse mode when reverse cards are wanted (a subdeck if the setting is off), else none.
fn reverse_for(wanted: bool, configured: ReverseMode) -> ReverseMode {
    match (wanted, configured) {
        (false, _) => ReverseMode::Off,
        (true, ReverseMode::Off) => ReverseMode::Subdeck,
        (true, mode) => mode,
    }
}

/// What a card without sound is told, and where the sound can be added later.
pub(crate) const SOUND_LATER: &str = "Settings → Anki → Add missing sounds fills it in once speech works.";
const SPEECH_OFF: &str = "Speech is off or not ready (Settings → Speech).";

fn days(interval: f32) -> String {
    if interval < 1.0 { "still being learned".into() } else { format!("every {} days", interval.round()) }
}

/// Add the picked words and (if wanted) the line itself. The line's sound is made once and shared by every card.
#[tauri::command]
pub async fn mine_add(state: S<'_>, request: MineAddRequest) -> Result<MineReply, String> {
    let engine = state.engine();
    let cfg = engine.config().learning.anki.clone();
    if !request.sentence_card && request.words.is_empty() {
        return Err("Pick at least one word, or the sentence card.".into());
    }
    let line = MineLine {
        sentence: request.sentence.clone(),
        translation: request.translation.clone(),
        app: request.app.clone(),
        deck: request.deck.clone(),
        reverse: reverse_for(request.reverse, cfg.reverse),
        sentence_deck: request.sentence_deck.clone(),
    };
    let want_audio = request.audio && cfg.audio;
    // Every card's sound takes a moment: Anki is asked to sync only once the whole batch is in.
    let _batch = engine.anki_batch();
    let mut audio_note = None;
    let mut sound = |made: Result<Option<Media>, String>, what: &str| match made {
        Ok(Some(media)) => Some(media),
        Ok(None) => {
            audio_note.get_or_insert_with(|| format!("{SPEECH_OFF} The cards were added without sound. {SOUND_LATER}"));
            None
        }
        Err(e) => {
            audio_note.get_or_insert_with(|| format!("Added without {what}: {e}. {SOUND_LATER}"));
            None
        }
    };
    let sentence_audio = if want_audio { sound(crate::commands::spoken(&state, request.sentence.trim()).await, "the line's sound") } else { None };

    let mut outcomes = Vec::new();
    for word in &request.words {
        let added = async {
            let permit = engine.begin_mine_word(&line, word).await.map_err(|e| e.to_string())?;
            let word_audio = if want_audio && permit.needs_audio() { sound(crate::commands::spoken(&state, word.key.trim()).await, "the word's sound") } else { None };
            engine.finish_mine_word(permit, &line, word, word_audio, sentence_audio.clone()).await.map_err(|e| e.to_string())
        }
        .await;
        outcomes.push(match added {
            Ok(AnkiAdded::Added { deck, reverse_deck, cards, warning, .. }) => MineOutcome {
                what: word.key.clone(),
                kind: "word",
                result: if warning.is_some() { "failed" } else { "added" },
                detail: warning.unwrap_or_else(|| match reverse_deck {
                    Some(reverse) => format!("{cards} cards: {deck}, reverse in {reverse}"),
                    None => format!("{cards} card{} in {deck}", if cards == 1 { "" } else { "s" }),
                }),
            },
            Ok(AnkiAdded::Exists { deck, interval_days, .. }) => MineOutcome { what: word.key.clone(), kind: "word", result: "exists", detail: format!("already in {deck} ({})", days(interval_days)) },
            Err(e) => MineOutcome { what: word.key.clone(), kind: "word", result: "failed", detail: e },
        });
    }
    if request.sentence_card {
        outcomes.push(match engine.mine_sentence(&line, &request.words, sentence_audio.clone()).await {
            Ok(MinedSentence::Added { deck }) => MineOutcome { what: request.sentence.clone(), kind: "sentence", result: "added", detail: format!("sentence card in {deck}") },
            Ok(MinedSentence::Exists { deck }) => MineOutcome { what: request.sentence.clone(), kind: "sentence", result: "exists", detail: format!("already in {deck}") },
            Err(e) => MineOutcome { what: request.sentence.clone(), kind: "sentence", result: "failed", detail: e.to_string() },
        });
    }
    if let Some(saved) = request.saved_id
        && outcomes.iter().any(|o| o.result != "failed")
        && let Err(e) = engine.mark_mined(saved)
    {
        tracing::warn!("marking the line as made into cards: {e}");
    }
    // When reading goes to another voice server, the voice on this PC was loaded only for these cards' sound.
    if want_audio && state.services.release_card_voice().await {
        tracing::info!("the card voice was unloaded after making cards");
    }
    Ok(MineReply { outcomes, audio_note })
}

#[derive(Debug, Serialize)]
pub struct FillReply {
    pub filled: usize,
    /// Notes still without sound after this batch.
    pub left: usize,
    /// Why it stopped early (speech off or failing, Anki gone).
    pub error: Option<String>,
}

/// Give Maru's Anki notes that have no sound their sounds, a batch at a time; stops at the first failure, since the
/// next note would fail the same way.
#[tauri::command]
pub async fn anki_fill_sounds(state: S<'_>) -> Result<FillReply, String> {
    const BATCH: usize = 30;
    let engine = state.engine();
    let silent = engine.anki_silent_notes(BATCH).await.map_err(|e| e.to_string())?;
    let _batch = engine.anki_batch();
    let (mut filled, mut error) = (0, None);
    'notes: for note in &silent.notes {
        let mut media = [None, None];
        for (slot, text) in media.iter_mut().zip([&note.word, &note.sentence]) {
            let Some(text) = text else { continue };
            match crate::commands::spoken(&state, text).await {
                Ok(Some(m)) => *slot = Some(m),
                Ok(None) => {
                    error = Some(SPEECH_OFF.to_string());
                    break 'notes;
                }
                Err(e) => {
                    error = Some(format!("Speech failed: {e}"));
                    break 'notes;
                }
            }
        }
        match engine.anki_add_sounds(note.note_id, media[0].as_ref(), media[1].as_ref()).await {
            Ok(true) => filled += 1,
            Ok(false) => {}
            Err(e) => {
                error = Some(e.to_string());
                break;
            }
        }
    }
    state.services.release_card_voice().await;
    Ok(FillReply { filled, left: silent.total.saturating_sub(filled), error })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverse_cards_follow_the_setting_and_a_wish_for_them_overrides_off() {
        assert_eq!(reverse_for(false, ReverseMode::Subdeck), ReverseMode::Off);
        assert_eq!(reverse_for(true, ReverseMode::SameDeck), ReverseMode::SameDeck);
        assert_eq!(reverse_for(true, ReverseMode::Off), ReverseMode::Subdeck);
    }

    #[test]
    fn a_line_handed_to_history_is_taken_once_and_an_empty_one_is_refused() {
        let line = |text: &str| CardsLine { id: Some(7), sentence: text.into(), translation: "t".into(), app: Some("game.exe".into()) };
        assert!(put_pending(line("  ")).is_err());
        put_pending(line("첫 줄")).unwrap();
        put_pending(line("둘째 줄")).unwrap();
        assert_eq!(take_pending().map(|l| l.sentence), Some("둘째 줄".to_string()), "the newer line replaces one never opened");
        assert_eq!(take_pending(), None);
    }

    #[test]
    fn intervals_read_as_plain_words() {
        assert_eq!(days(0.0), "still being learned");
        assert_eq!(days(12.4), "every 12 days");
    }
}
