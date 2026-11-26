//! Anki cards from a live line you starred: the words in it, each with its meaning in that line, and the line itself as
//! a sentence card. Opening a line ([`Engine::mine_sheet`]) costs at most one model request for the meanings; word cards
//! go through the same path as the panel's "Add to Anki", with this line as their example sentence. With the learner's
//! dictionary installed (`senses.rs`) the meanings are its senses, in English and Spanish.

use maru_core::WordState;
use maru_core::config::ReverseMode;
use maru_llm::{GlossRequest, LlmChunk, PromptContext};
use maru_srs::anki_sentence::{SentenceAdded, SentenceRequest, SentenceWord};
use maru_srs::{Context, LookupInfo, Media, now_secs, tag_of};
use serde::{Deserialize, Serialize};

use crate::anki_flow::{AnkiAddOptions, AnkiAddPermit, AnkiAdded, DeckSuggestion, app_display_name, slug, suggest_decks, tags_for, year_month};
use crate::engine::{Engine, EngineError, Result};

/// A word of the line, as the mining sheet offers it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MineWord {
    /// Dictionary form: what the card asks.
    pub key: String,
    /// As written in the line.
    pub surface: String,
    pub pos: String,
    pub level: Option<String>,
    /// How the written form is built (`했어요 = 하다 + 았/었 + 어요`).
    pub formula: String,
    /// The dictionary's short senses.
    pub dictionary: Vec<String>,
    /// Its meaning in this line (from the model), else the first dictionary sense.
    pub meaning: String,
    /// The same in Spanish, from the learner's dictionary (or the model for a word it lacks); empty without it.
    pub meaning_es: String,
    /// How the two meanings were arrived at; `None` when the learner's dictionary had no part in it.
    pub source: Option<crate::senses::MeaningSource>,
    /// The learner's dictionary's senses for the word, to choose another one.
    pub senses: Vec<crate::senses::SenseChoice>,
    pub in_context: bool,
    pub known: bool,
    pub in_anki: bool,
    /// Ticked at first: not known, not in Anki, with a meaning.
    pub pick: bool,
}

/// Everything the mining sheet shows for one line.
#[derive(Debug, Clone, Serialize)]
pub struct MineSheet {
    pub sentence: String,
    pub translation: String,
    pub app: Option<String>,
    pub app_name: Option<String>,
    pub words: Vec<MineWord>,
    pub decks: Vec<String>,
    pub suggestions: Vec<DeckSuggestion>,
    pub reverse: ReverseMode,
    pub audio: bool,
    pub sentence_in_anki: bool,
    /// Anki off or unreachable: the sheet still shows the words; adding explains what to do.
    pub anki_error: Option<String>,
    /// Why meanings are the dictionary's rather than the line's.
    pub note: Option<String>,
    /// The learner's dictionary's attribution, shown with its meanings.
    pub credit: Option<String>,
    /// The connection that gave the meanings; `None` when no model was needed, or none answered (the dictionary).
    pub answered: Option<crate::routes::Answered>,
    /// The connections asked before it that did not answer, each with a plain sentence.
    pub attempts: Vec<crate::routes::Attempt>,
    /// The connections this line can be asked again with ("Retry with …").
    pub retry: Vec<crate::routes::RetryOption>,
    /// Live subtitles were running: this PC's model is not loaded beside them.
    pub live_running: bool,
}

/// The line being mined and where its cards go.
#[derive(Debug, Clone, Deserialize)]
pub struct MineLine {
    pub sentence: String,
    pub translation: String,
    pub app: Option<String>,
    pub deck: String,
    pub reverse: ReverseMode,
}

/// A word picked on the sheet, with the meaning as you left it.
#[derive(Debug, Clone, Deserialize)]
pub struct MineChoice {
    pub key: String,
    pub surface: String,
    #[serde(default)]
    pub pos: String,
    #[serde(default)]
    pub level: Option<String>,
    pub meaning: String,
    #[serde(default)]
    pub meaning_es: String,
}

/// A line translated again for its card.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CardSentence {
    /// `None`: no connection is chosen for it (the card keeps the line's translation), or none answered.
    pub translation: Option<String>,
    pub answered: Option<crate::routes::Answered>,
    pub attempts: Vec<crate::routes::Attempt>,
    pub note: Option<String>,
}

/// What adding the sentence card did.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum MinedSentence {
    Added { deck: String },
    Exists { deck: String },
}

impl Engine {
    /// The words of a line with their meaning in it, and what Anki already has.
    pub async fn mine_sheet(&self, sentence: &str, translation: &str, app: Option<&str>) -> Result<MineSheet> {
        self.mine_sheet_with(sentence, translation, app, &Default::default()).await
    }

    /// As [`Engine::mine_sheet`], asking for the meanings the way `ask` says: one named connection instead of
    /// the task's order, or only once live subtitles have stopped.
    pub async fn mine_sheet_with(&self, sentence: &str, translation: &str, app: Option<&str>, ask: &crate::routes::CardAsk) -> Result<MineSheet> {
        if ask.after_live {
            crate::routes::until_live_stops(&self.inner).await;
        }
        let live_running = self.live_active();
        let sentence = sentence.trim().to_string();
        if sentence.is_empty() {
            return Err(EngineError::Other("There is no line to make cards from.".into()));
        }
        let inner = self.inner.clone();
        let text = sentence.clone();
        let eojeols = tokio::task::spawn_blocking(move || {
            let mut eojeols = inner.analyzer.analyze(&text).eojeols;
            crate::annotate::annotate(&mut eojeols, inner.dict.as_deref(), Some(&inner.vocab), 3);
            eojeols
        })
        .await
        .map_err(|_| EngineError::Other("the Korean analyser crashed".into()))?;
        let mut eojeols: Vec<_> = eojeols.into_iter().filter(|e| e.content).collect();
        let mut seen = std::collections::HashSet::new();
        eojeols.retain(|e| seen.insert(e.vocab_key().to_string()));

        let profile = app.and_then(|a| self.inner.cfg.profile_for(a));
        // With the learner's dictionary the meanings are its senses; without it, the model's words as before.
        let (glosses, senses, note, answered, attempts) = match self.inner.nikl.get() {
            Some(nikl) => {
                let found = crate::senses::line_senses(&self.inner, nikl, &sentence, &eojeols, ask).await;
                (Vec::new(), found.words, found.note, found.answered, found.attempts)
            }
            None => {
                let (glosses, note, answered, attempts) = self.line_meanings(&sentence, &eojeols, profile, ask).await;
                (glosses, Vec::new(), note, answered, attempts)
            }
        };

        let cfg = self.inner.cfg.learning.anki.clone();
        let mut words: Vec<MineWord> = eojeols
            .iter()
            .map(|e| {
                use crate::senses::MeaningSource;
                let in_line = glosses.iter().find(|(i, _)| *i == e.index).map(|(_, m)| m.clone());
                let sense = senses.iter().find(|s| s.index == e.index);
                MineWord {
                    key: e.vocab_key().to_string(),
                    surface: e.surface.clone(),
                    pos: e.pos.label().to_string(),
                    level: e.level.clone(),
                    formula: e.formula.clone(),
                    dictionary: match sense {
                        Some(s) if !s.options.is_empty() => s.options.iter().map(|o| o.en.clone()).collect(),
                        _ => e.dict_glosses.clone(),
                    },
                    in_context: in_line.is_some() || sense.is_some_and(|s| matches!(s.source, MeaningSource::Chosen | MeaningSource::Model)),
                    meaning: sense.map(|s| s.en.clone()).filter(|m| !m.trim().is_empty()).or(in_line).or_else(|| e.dict_glosses.first().cloned()).unwrap_or_default(),
                    meaning_es: sense.map(|s| s.es.clone()).unwrap_or_default(),
                    source: sense.map(|s| s.source),
                    senses: sense.map(|s| s.options.clone()).unwrap_or_default(),
                    known: e.state == WordState::Known,
                    in_anki: false,
                    pick: false,
                }
            })
            .collect();

        let (mut decks, mut anki_error, mut sentence_in_anki) = (Vec::new(), None, false);
        match self.anki_client() {
            Err(e) => anki_error = Some(e.to_string()),
            Ok(anki) => match anki.decks().await {
                Err(e) => anki_error = Some(e.to_string()),
                Ok(d) => {
                    decks = d;
                    for w in &mut words {
                        w.in_anki = anki.find(&w.key).await.ok().flatten().is_some();
                    }
                    sentence_in_anki = anki.find_sentence(&sentence).await.ok().flatten().is_some();
                }
            },
        }
        for w in &mut words {
            w.pick = !w.known && !w.in_anki && !w.meaning.trim().is_empty();
        }
        let recent = self.inner.vocab.recent_decks(6).unwrap_or_default();
        let remembered = app.and_then(|a| self.inner.vocab.deck_for_app(a).ok().flatten());
        Ok(MineSheet {
            sentence,
            translation: translation.trim().to_string(),
            app: app.map(str::to_owned),
            app_name: app.map(|a| app_display_name(a, profile)).filter(|n| !n.is_empty()),
            words,
            suggestions: suggest_decks(&cfg.deck, app, profile, remembered.as_deref(), &recent, &decks),
            decks,
            reverse: cfg.reverse,
            audio: cfg.audio,
            sentence_in_anki,
            anki_error,
            note,
            credit: self.learner_dictionary_credit().map(str::to_owned),
            answered,
            attempts,
            retry: self.card_retry_options(),
            live_running,
        })
    }

    /// Each word's meaning in this line, by its index, in the model's own words (no learner's dictionary): from
    /// the card-meanings connection and then this PC's model, with a note when a connection or all of them failed.
    async fn line_meanings(
        &self,
        sentence: &str,
        eojeols: &[maru_core::Eojeol],
        profile: Option<&maru_core::config::AppProfile>,
        ask: &crate::routes::CardAsk,
    ) -> (Vec<(usize, String)>, Option<String>, Option<crate::routes::Answered>, Vec<crate::routes::Attempt>) {
        let words = crate::pipeline::gloss_words(eojeols, false);
        if words.is_empty() {
            return (Vec::new(), None, None, Vec::new());
        }
        let inner = &self.inner;
        let offline = "Meanings are the dictionary's: check that each one fits the line.";
        let ctx = PromptContext {
            hint: profile.and_then(|p| p.hint.clone()),
            glossary: profile.map(|p| p.glossary.clone()).unwrap_or_default(),
            ..Default::default()
        };
        let request = GlossRequest { sentence, words: &words, ctx: &ctx };
        let request = &request;
        let ask_one = |llm: maru_llm::LlmClient| async move {
            let mut found = Vec::new();
            let result = llm
                .glosses(request, &mut |chunk| {
                    if let LlmChunk::Gloss(g) = chunk
                        && request.words.iter().any(|w| w.index == g.index)
                    {
                        found.push((g.index, g.meaning));
                    }
                })
                .await;
            match result {
                Err(e) if found.is_empty() => Err(e),
                _ => Ok((!found.is_empty()).then_some(found)),
            }
        };
        let routes = inner.routes();
        if !routes.card_meanings.is_empty() || ask.connection.is_some() {
            let order = crate::routes::candidates(&routes, &routes.card_meanings, ask);
            let outcome = crate::routes::ask_in_order(inner, order, ask_one).await;
            let note = match &outcome.value {
                Some(_) => outcome.note(""),
                None => Some(outcome.note(offline).unwrap_or_else(|| offline.to_string())),
            };
            return (outcome.value.unwrap_or_default(), note, outcome.answered, outcome.attempts);
        }
        // An engine built without task routes: the capture model, as before `[tasks]`.
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let em = crate::pipeline::Emitter::new(tx, 0);
        let live = inner.live_active.load(std::sync::atomic::Ordering::Acquire);
        let Some((llm, native)) = crate::pipeline::snapshot_model(inner, &em, live).await else {
            let why = std::iter::from_fn(|| rx.try_recv().ok()).find_map(|e| match e {
                maru_core::Event::Error { message, .. } => Some(message),
                _ => None,
            });
            return (Vec::new(), Some(why.map_or(offline.to_string(), |w| format!("{offline} ({w})"))), None, Vec::new());
        };
        if native {
            return (Vec::new(), Some(format!("{offline} The live translation model gives no word meanings; turn live subtitles off for meanings in context.")), None, Vec::new());
        }
        match ask_one(llm).await {
            Err(e) => (Vec::new(), Some(format!("{offline} ({e})")), None, Vec::new()),
            Ok(found) => (found.unwrap_or_default(), None, None, Vec::new()),
        }
    }

    /// Translate a line for its sentence card with the card-sentence connection, then this PC's model; `ask` names
    /// one connection instead. With no connection chosen for the task and none named, nothing is asked.
    pub async fn card_sentence(&self, sentence: &str, app: Option<&str>, ask: &crate::routes::CardAsk) -> CardSentence {
        if ask.after_live {
            crate::routes::until_live_stops(&self.inner).await;
        }
        let routes = self.inner.routes();
        let order = crate::routes::candidates(&routes, &routes.card_sentence, ask);
        if order.is_empty() {
            return CardSentence { translation: None, answered: None, attempts: Vec::new(), note: None };
        }
        let profile = app.and_then(|a| self.inner.cfg.profile_for(a));
        let ctx = PromptContext {
            hint: profile.and_then(|p| p.hint.clone()),
            glossary: profile.map(|p| p.glossary.clone()).unwrap_or_default(),
            ..Default::default()
        };
        let request = maru_llm::LiveRequest { text: sentence.trim(), ctx: &ctx, turns: &[] };
        let request = &request;
        let outcome = crate::routes::ask_in_order(&self.inner, order, |client| async move {
            let text = client.translate_live(request, &mut |_| {}).await?;
            Ok(Some(text.trim().to_string()).filter(|t| !t.is_empty()))
        })
        .await;
        let note = outcome.note("The line keeps the translation it had.");
        CardSentence { translation: outcome.value, answered: outcome.answered, attempts: outcome.attempts, note }
    }

    /// Record the word as met in this line, with the meaning you chose, and reserve its Anki add.
    pub async fn begin_mine_word(&self, line: &MineLine, word: &MineChoice) -> Result<AnkiAddPermit> {
        let key = word.key.trim();
        if key.is_empty() || word.meaning.trim().is_empty() {
            return Err(EngineError::Other(format!("\"{}\" needs a meaning before it can become a card.", word.surface)));
        }
        let info = LookupInfo {
            key: key.to_string(),
            surface: word.surface.clone(),
            pos: word.pos.clone(),
            meaning: Some(word.meaning.trim().to_string()),
            level: word.level.clone(),
            context: Some(context_of(line)),
        };
        self.inner.vocab.record_lookup(&info, now_secs())?;
        self.begin_anki_add(key, &line.deck).await
    }

    /// Add the reserved word card with this line as its example.
    pub async fn finish_mine_word(&self, permit: AnkiAddPermit, line: &MineLine, word: &MineChoice, word_audio: Option<Media>, sentence_audio: Option<Media>) -> Result<AnkiAdded> {
        let cfg = &self.inner.cfg.learning.anki;
        let app = line.app.as_deref();
        let record = self.inner.vocab.get(word.key.trim())?.ok_or_else(|| EngineError::Vocab(format!("unknown word: {}", word.key)))?;
        let tags = tags_for(cfg, &record, app, app.and_then(|a| self.inner.cfg.profile_for(a)), now_secs());
        let opts = AnkiAddOptions {
            key: word.key.trim().to_string(),
            deck: line.deck.clone(),
            tags,
            reverse: line.reverse,
            app: line.app.clone(),
            word_audio,
            sentence_audio,
            context: Some(context_of(line)),
            meaning_es: Some(word.meaning_es.trim().to_string()).filter(|m| !m.is_empty()),
        };
        self.finish_anki_add(permit, opts).await
    }

    /// The line as a sentence card, listing the words picked from it.
    pub async fn mine_sentence(&self, line: &MineLine, words: &[MineChoice], audio: Option<Media>) -> Result<MinedSentence> {
        let anki = self.anki_client()?;
        let deck = crate::anki_flow::clean_deck(&line.deck);
        if deck.is_empty() {
            return Err(EngineError::Other("choose a deck first".into()));
        }
        let app = line.app.as_deref();
        let profile = app.and_then(|a| self.inner.cfg.profile_for(a));
        let req = SentenceRequest {
            sentence: line.sentence.clone(),
            translation: line.translation.clone(),
            words: words.iter().map(|w| SentenceWord { word: w.key.trim().to_string(), meaning: w.meaning.trim().to_string(), meaning_es: w.meaning_es.trim().to_string() }).collect(),
            deck,
            tags: sentence_tags(&self.inner.cfg.learning.anki, app, profile, now_secs()),
            audio,
            source: app.map(|a| app_display_name(a, profile)).filter(|n| !n.is_empty()),
        };
        let added = anki.add_sentence(&req).await.map_err(|e| EngineError::Other(e.to_string()))?;
        Ok(match added {
            SentenceAdded::New { deck, .. } => {
                if let Err(e) = self.inner.vocab.remember_deck(&deck, app, now_secs()) {
                    tracing::warn!("remembering the deck: {e}");
                }
                if anki.config().sync_after_add {
                    self.schedule_anki_sync();
                }
                MinedSentence::Added { deck }
            }
            SentenceAdded::Exists(existing) => MinedSentence::Exists { deck: existing.deck },
        })
    }
}

fn context_of(line: &MineLine) -> Context {
    Context {
        sentence: line.sentence.trim().to_string(),
        translation: Some(line.translation.trim().to_string()).filter(|t| !t.is_empty()),
        app: line.app.clone(),
        screenshot: None,
    }
}

/// A sentence card's tags: the configured ones, the program's, and (if wanted) `sentence`, the program and the month.
pub fn sentence_tags(cfg: &maru_core::config::AnkiConfig, app: Option<&str>, profile: Option<&maru_core::config::AppProfile>, now: i64) -> Vec<String> {
    let mut tags: Vec<String> = cfg.tags.clone();
    if let Some(p) = profile {
        tags.extend(p.anki_tags.iter().cloned());
    }
    if cfg.auto_tags {
        tags.push("sentence".into());
        if let Some(name) = app.map(|a| app_display_name(a, profile)).filter(|n| !n.is_empty()) {
            tags.push(format!("app::{}", slug(&name)));
        }
        tags.push(format!("added::{}", year_month(now)));
    }
    let mut seen: Vec<String> = Vec::new();
    for t in tags.iter().map(|t| tag_of(t)).filter(|t| !t.is_empty()) {
        if !seen.iter().any(|s| s.eq_ignore_ascii_case(&t)) {
            seen.push(t);
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sentence_card_is_tagged_as_a_sentence_from_its_program_and_month() {
        let cfg = maru_core::config::AnkiConfig::default();
        assert_eq!(sentence_tags(&cfg, Some("eldenring.exe"), None, 1_760_000_000), ["maru", "sentence", "app::eldenring", "added::2025-10"]);
        let plain = maru_core::config::AnkiConfig { auto_tags: false, ..cfg };
        assert_eq!(sentence_tags(&plain, Some("x.exe"), None, 0), ["maru"]);
    }
}
