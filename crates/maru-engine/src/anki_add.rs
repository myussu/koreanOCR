//! Validate and reserve an add before the front end requests costly audio.
use super::*;
use maru_srs::{ExistingNote, anki_store::PendingAdd};

pub struct AnkiAddPermit {
    _guard: tokio::sync::OwnedMutexGuard<()>,
    key: String,
    deck: String,
    existing: Option<ExistingNote>,
    pending: Option<PendingAdd>,
}

impl AnkiAddPermit {
    pub fn needs_audio(&self) -> bool {
        self.existing.is_none() && self.pending.is_none()
    }
}

impl Engine {
    pub async fn begin_anki_add(&self, key: &str, deck: &str) -> Result<AnkiAddPermit> {
        let guard = self
            .inner
            .anki_add_gate
            .clone()
            .try_lock_owned()
            .map_err(|_| {
                EngineError::Other(
                    "Another Anki add is still running; wait for it to finish.".into(),
                )
            })?;
        let anki = self.anki_client()?;
        self.word(key)?;
        let deck = clean_deck(deck);
        if deck.is_empty() {
            return Err(EngineError::Other("choose a deck first".into()));
        }
        anki.ensure_model().await.map_err(anki_error)?;
        let existing = anki.find(key).await.map_err(anki_error)?;
        let cfg = anki.config();
        let mut pending = self
            .inner
            .vocab
            .pending_add(key, &cfg.url, &cfg.note_type)?;
        if pending
            .as_ref()
            .is_some_and(|p| existing.as_ref().map(|e| e.note_id) != Some(p.note_id))
        {
            self.inner
                .vocab
                .finish_pending_add(key, &cfg.url, &cfg.note_type)?;
            pending = None;
        }
        Ok(AnkiAddPermit {
            _guard: guard,
            key: key.into(),
            deck,
            existing,
            pending,
        })
    }

    pub async fn finish_anki_add(
        &self,
        permit: AnkiAddPermit,
        opts: AnkiAddOptions,
    ) -> Result<AnkiAdded> {
        if permit.key != opts.key || permit.deck != clean_deck(&opts.deck) {
            return Err(EngineError::Other(
                "The Anki add changed while it was being prepared.".into(),
            ));
        }
        let anki = self.anki_client()?;
        let cfg = anki.config();
        let word = self.word(&opts.key)?;
        let contexts = self.inner.vocab.contexts(&opts.key, 2)?;
        let added = if let Some(pending) = &permit.pending {
            let result = anki
                .finish_add(pending.note_id, &pending.deck, pending.reverse)
                .await;
            let (reverse_deck, cards, warning) = match result {
                Ok((reverse_deck, cards)) => (reverse_deck, cards, None),
                Err(error) => (
                    None,
                    0,
                    Some(format!(
                        "The note is in Anki, but setup is still unfinished: {error}. Retry when Anki is ready."
                    )),
                ),
            };
            Added::New {
                note_id: pending.note_id,
                deck: pending.deck.clone(),
                reverse_deck,
                cards,
                warning,
            }
        } else if let Some(existing) = permit.existing {
            self.inner.vocab.remember_anki_membership(&opts.key)?;
            return Ok(AnkiAdded::Exists {
                note_id: existing.note_id,
                deck: existing.deck,
                interval_days: existing.interval_days,
            });
        } else {
            let req = AddRequest {
                pronunciation: Some(maru_lang::pronounce::pronounce_word(&word.key))
                    .filter(|p| *p != word.key),
                word,
                contexts,
                deck: permit.deck.clone(),
                tags: opts.tags,
                reverse: opts.reverse,
                word_audio: opts.word_audio,
                sentence_audio: opts.sentence_audio,
                source: opts
                    .app
                    .as_deref()
                    .map(|a| app_display_name(a, self.inner.cfg.profile_for(a)))
                    .filter(|n| !n.is_empty()),
            };
            anki.add(&req).await.map_err(anki_error)?
        };
        match added {
            Added::Exists(existing) => {
                self.inner.vocab.remember_anki_membership(&opts.key)?;
                Ok(AnkiAdded::Exists {
                    note_id: existing.note_id,
                    deck: existing.deck,
                    interval_days: existing.interval_days,
                })
            }
            Added::New {
                note_id,
                deck,
                reverse_deck,
                cards,
                mut warning,
            } => {
                let now = now_secs();
                let app = permit
                    .pending
                    .as_ref()
                    .and_then(|p| p.app.as_deref())
                    .or(opts.app.as_deref());
                let local = (|| -> maru_srs::Result<()> {
                    if warning.is_some() {
                        self.inner.vocab.remember_pending_add(&PendingAdd {
                            key: opts.key.clone(),
                            endpoint: cfg.url.clone(),
                            note_type: cfg.note_type.clone(),
                            note_id,
                            deck: deck.clone(),
                            reverse: permit.pending.as_ref().map_or(opts.reverse, |p| p.reverse),
                            app: app.map(str::to_owned),
                        })?;
                    } else {
                        self.inner
                            .vocab
                            .finish_pending_add(&opts.key, &cfg.url, &cfg.note_type)?;
                    }
                    self.inner.vocab.remember_deck(&deck, app, now)?;
                    self.inner.vocab.set_anki_state(&opts.key, Some(0.0), now)?;
                    self.inner.vocab.remember_anki_membership(&opts.key)
                })();
                if let Err(error) = local {
                    let message = format!(
                        "The note is in Anki, but Maru could not save its local tracking: {error}"
                    );
                    warning = Some(warning.map_or(message.clone(), |w| format!("{w} {message}")));
                }
                let sync = cfg.sync_after_add;
                if sync {
                    self.schedule_anki_sync();
                }
                Ok(AnkiAdded::Added {
                    deck,
                    reverse_deck,
                    cards,
                    sync,
                    warning,
                })
            }
        }
    }
}
