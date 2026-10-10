//! Words into Anki: which deck to suggest, which tags to give, and what happens when you add one.
//!
//! The suggestions and tags are plain functions so they can be tested without Anki.

use std::time::Duration;

use maru_core::config::{AnkiConfig, AppProfile, ReverseMode};
pub use maru_srs::Media;
use maru_srs::{AddRequest, Added, WordRecord, now_secs, tag_of};
use serde::Serialize;

use crate::engine::{Engine, EngineError, Result};

/// A deck worth offering for a word, and why.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DeckSuggestion {
    pub deck: String,
    pub reason: String,
    /// Anki already has this deck (otherwise adding creates it).
    pub exists: bool,
}

/// A deck name as Anki wants it: parts separated by `::`, none of them empty or padded.
pub fn clean_deck(raw: &str) -> String {
    raw.split("::").map(str::trim).filter(|p| !p.is_empty()).collect::<Vec<_>>().join("::")
}

/// A program's name as people say it: `eldenring.exe` → `Eldenring`, a profile's name when it has one.
pub fn app_display_name(exe: &str, profile: Option<&AppProfile>) -> String {
    if let Some(p) = profile.filter(|p| !p.name.trim().is_empty()) {
        return p.name.trim().to_string();
    }
    let file = exe.rsplit(['\\', '/']).next().unwrap_or(exe);
    let stem = file.rsplit_once('.').map_or(file, |(s, ext)| if ext.eq_ignore_ascii_case("exe") { s } else { file });
    let mut chars = stem.trim().chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Decks to offer, best first: what you chose last time for this program, the deck its profile names,
/// a deck named after the program, decks used lately, and the default.
pub fn suggest_decks(
    base: &str,
    app: Option<&str>,
    profile: Option<&AppProfile>,
    remembered_for_app: Option<&str>,
    recent: &[String],
    existing: &[String],
) -> Vec<DeckSuggestion> {
    let mut out: Vec<DeckSuggestion> = Vec::new();
    let mut offer = |deck: &str, reason: String| {
        let deck = clean_deck(deck);
        if deck.is_empty() || out.iter().any(|s| s.deck.eq_ignore_ascii_case(&deck)) {
            return;
        }
        let exists = existing.iter().any(|d| d.eq_ignore_ascii_case(&deck));
        out.push(DeckSuggestion { deck, reason, exists });
    };
    let name = app.map(|a| app_display_name(a, profile)).filter(|n| !n.is_empty());
    if let (Some(deck), Some(name)) = (remembered_for_app, &name) {
        offer(deck, format!("what you chose last time in {name}"));
    }
    if let Some(deck) = profile.and_then(|p| p.anki_deck.as_deref()) {
        offer(deck, format!("the deck set for {}", name.as_deref().unwrap_or("this program")));
    }
    if let Some(name) = &name {
        offer(&format!("{}::{name}", clean_deck(base)), format!("named after {name}"));
    }
    for deck in recent.iter().take(4) {
        offer(deck, "used lately".into());
    }
    offer(base, "your default deck".into());
    out
}

/// `TOPIK 1` → `topik-1`.
fn slug(s: &str) -> String {
    let t: String = s.trim().chars().map(|c| if c.is_alphanumeric() { c.to_lowercase().next().unwrap_or(c) } else { '-' }).collect();
    t.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-")
}

/// `2026-10`, from Unix seconds (civil-from-days, so no date library is needed).
pub fn year_month(secs: i64) -> String {
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year}-{month:02}")
}

/// The tags a word gets: the configured ones, the program's, and (if wanted) what the word is.
pub fn tags_for(cfg: &AnkiConfig, word: &WordRecord, app: Option<&str>, profile: Option<&AppProfile>, now: i64) -> Vec<String> {
    let mut tags: Vec<String> = cfg.tags.clone();
    if let Some(p) = profile {
        tags.extend(p.anki_tags.iter().cloned());
    }
    if cfg.auto_tags {
        if let Some(level) = &word.level {
            tags.push(slug(level));
        }
        if !word.pos.trim().is_empty() {
            tags.push(format!("pos::{}", slug(&word.pos)));
        }
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

// ───────────────────────────── the engine's side ─────────────────────────────

/// Everything the "Add to Anki" popover needs.
#[derive(Debug, Clone, Serialize)]
pub struct AnkiOverview {
    pub connected: bool,
    pub error: Option<String>,
    pub decks: Vec<String>,
    pub suggestions: Vec<DeckSuggestion>,
    pub tags: Vec<String>,
    /// Where Anki already has this word, if it does.
    pub existing: Option<ExistingInfo>,
    pub reverse: ReverseMode,
    /// The program the word came from, as shown to you.
    pub app_name: Option<String>,
    pub audio: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExistingInfo {
    pub note_id: u64,
    pub deck: String,
    pub interval_days: f32,
}

/// What adding a word did.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum AnkiAdded {
    Added { deck: String, reverse_deck: Option<String>, cards: usize, sync: bool },
    Exists { note_id: u64, deck: String, interval_days: f32 },
}

#[derive(Debug, Clone)]
pub struct AnkiAddOptions {
    pub key: String,
    pub deck: String,
    pub tags: Vec<String>,
    pub reverse: ReverseMode,
    pub app: Option<String>,
    pub word_audio: Option<Media>,
    pub sentence_audio: Option<Media>,
}

fn anki_error(e: maru_srs::SrsError) -> EngineError {
    EngineError::Other(e.to_string())
}

impl Engine {
    fn anki_client(&self) -> Result<&maru_srs::AnkiClient> {
        self.inner.anki.as_ref().ok_or_else(|| EngineError::Other("Anki is switched off: turn it on in Settings → Anki".into()))
    }

    fn word(&self, key: &str) -> Result<WordRecord> {
        self.inner.vocab.get(key)?.ok_or_else(|| EngineError::Vocab(format!("unknown word: {key}")))
    }

    /// Is Anki up, which decks does it have, and what would we suggest for this word.
    pub async fn anki_overview(&self, key: &str, app: Option<&str>) -> Result<AnkiOverview> {
        let anki = self.anki_client()?;
        let cfg = anki.config();
        let profile = app.and_then(|a| self.inner.cfg.profile_for(a));
        let word = self.word(key).ok();
        let tags = word.as_ref().map(|w| tags_for(cfg, w, app, profile, now_secs())).unwrap_or_default();
        let app_name = app.map(|a| app_display_name(a, profile)).filter(|n| !n.is_empty());
        let recent = self.inner.vocab.recent_decks(6).unwrap_or_default();
        let remembered = app.and_then(|a| self.inner.vocab.deck_for_app(a).ok().flatten());
        let base = |decks: &[String], error: Option<String>, existing: Option<ExistingInfo>| AnkiOverview {
            connected: error.is_none(),
            error,
            suggestions: suggest_decks(&cfg.deck, app, profile, remembered.as_deref(), &recent, decks),
            decks: decks.to_vec(),
            tags: tags.clone(),
            existing,
            reverse: cfg.reverse,
            app_name: app_name.clone(),
            audio: cfg.audio,
        };
        let decks = match anki.decks().await {
            Ok(d) => d,
            Err(e) => return Ok(base(&[], Some(e.to_string()), None)),
        };
        let existing = anki.find(key).await.ok().flatten().map(|n| ExistingInfo { note_id: n.note_id, deck: n.deck, interval_days: n.interval_days });
        Ok(base(&decks, None, existing))
    }

    /// Add a word to Anki. Remembers the deck for the program the word came from, marks the word
    /// saved, and asks Anki to sync with AnkiWeb a moment later (once, however many words came in).
    pub async fn anki_add(&self, opts: AnkiAddOptions) -> Result<AnkiAdded> {
        let anki = self.anki_client()?;
        let word = self.word(&opts.key)?;
        let contexts = self.inner.vocab.contexts(&opts.key, 2)?;
        let deck = clean_deck(&opts.deck);
        if deck.is_empty() {
            return Err(EngineError::Other("choose a deck first".into()));
        }
        let req = AddRequest {
            pronunciation: Some(maru_lang::pronounce::pronounce_word(&word.key)).filter(|p| *p != word.key),
            word,
            contexts,
            deck: deck.clone(),
            tags: opts.tags,
            reverse: opts.reverse,
            word_audio: opts.word_audio,
            sentence_audio: opts.sentence_audio,
            source: opts.app.as_deref().map(|a| app_display_name(a, self.inner.cfg.profile_for(a))).filter(|n| !n.is_empty()),
        };
        match anki.add(&req).await.map_err(anki_error)? {
            Added::Exists(n) => Ok(AnkiAdded::Exists { note_id: n.note_id, deck: n.deck, interval_days: n.interval_days }),
            Added::New { deck, reverse_deck, cards, .. } => {
                let now = now_secs();
                let _ = self.inner.vocab.remember_deck(&deck, opts.app.as_deref(), now);
                let _ = self.inner.vocab.set_anki_state(&req.word.key, Some(0.0), now);
                let sync = anki.config().sync_after_add;
                if sync {
                    self.schedule_anki_sync();
                }
                Ok(AnkiAdded::Added { deck, reverse_deck, cards, sync })
            }
        }
    }

    /// Put another sentence on a word's existing card. `false` if it was there already.
    pub async fn anki_add_example(&self, key: &str, sentence: &str, translation: Option<&str>, audio: Option<Media>) -> Result<bool> {
        let anki = self.anki_client()?;
        let word = self.word(key)?;
        let note = anki.find(key).await.map_err(anki_error)?.ok_or_else(|| EngineError::Other("Anki does not have that word any more".into()))?;
        let added = anki.add_example(note.note_id, sentence, translation, &word.surface, audio.as_ref()).await.map_err(anki_error)?;
        if added && anki.config().sync_after_add {
            self.schedule_anki_sync();
        }
        Ok(added)
    }

    /// Sync with AnkiWeb in a few seconds; asking again meanwhile does not start another.
    fn schedule_anki_sync(&self) {
        use std::sync::atomic::Ordering;
        if self.inner.anki_sync_pending.swap(true, Ordering::AcqRel) {
            return;
        }
        let inner = self.inner.clone();
        self.inner.runtime.spawn(async move {
            tokio::time::sleep(Duration::from_secs(4)).await;
            inner.anki_sync_pending.store(false, Ordering::Release);
            if let Some(anki) = inner.anki.as_ref()
                && let Err(e) = anki.sync().await
            {
                tracing::warn!("Anki sync: {e}");
            }
        });
    }

    /// Ask Anki to sync with AnkiWeb right now.
    pub async fn anki_sync(&self) -> Result<()> {
        self.anki_client()?.sync().await.map_err(anki_error)
    }

    /// Read how every card of ours stands in Anki and tell the vocabulary: words held for weeks become
    /// known, so their glosses fade. Returns how many words Anki has.
    pub async fn anki_refresh(&self) -> Result<usize> {
        let anki = self.anki_client()?;
        let states = anki.states().await.map_err(anki_error)?;
        let now = now_secs();
        for s in &states {
            // A suspended card says nothing about what you know; a card still being learned is 0 days.
            let interval = if s.queue < 0 { None } else if s.card_type == 2 { Some(s.interval_days) } else { Some(0.0) };
            if let Some(days) = interval {
                self.inner.vocab.set_anki_state(&s.word, Some(days), now)?;
            }
        }
        Ok(states.len())
    }

    /// Cards due now and new cards waiting, for the Review tab.
    pub async fn anki_due(&self) -> Result<maru_srs::DueCounts> {
        self.anki_client()?.due().await.map_err(anki_error)
    }

    /// Can we talk to Anki, and is the note type in place (created if not)? For the Settings check.
    pub async fn anki_check(&self) -> Result<String> {
        let anki = self.anki_client()?;
        let version = anki.ping().await.map_err(anki_error)?;
        anki.ensure_model().await.map_err(anki_error)?;
        let decks = anki.decks().await.map_err(anki_error)?.len();
        Ok(format!("Anki is reachable (AnkiConnect v{version}); the note type \"{}\" is ready; {decks} decks.", anki.config().note_type))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(name: &str, deck: Option<&str>, tags: &[&str]) -> AppProfile {
        AppProfile { name: name.into(), exe: vec!["eldenring.exe".into()], anki_deck: deck.map(Into::into), anki_tags: tags.iter().map(|t| (*t).into()).collect(), ..Default::default() }
    }

    fn word() -> WordRecord {
        WordRecord {
            key: "공부하다".into(),
            surface: "공부했어요".into(),
            pos: "verb".into(),
            meaning: None,
            level: Some("TOPIK 1".into()),
            lookups: 1,
            saved: false,
            marked_known: false,
            stability: None,
            difficulty: None,
            due: None,
            reps: 0,
            lapses: 0,
            anki_interval: None,
            state: maru_core::WordState::New,
        }
    }

    #[test]
    fn programs_get_readable_names() {
        assert_eq!(app_display_name("eldenring.exe", None), "Eldenring");
        assert_eq!(app_display_name(r"C:\Games\Hades\Hades.exe", None), "Hades");
        assert_eq!(app_display_name("firefox", None), "Firefox");
        assert_eq!(app_display_name("eldenring.exe", Some(&profile("Elden Ring", None, &[]))), "Elden Ring");
        assert_eq!(app_display_name("", None), "");
    }

    #[test]
    fn suggestions_put_what_you_chose_for_this_program_first() {
        let existing = vec!["Korean".to_string(), "Korean::Games".to_string()];
        let recent = vec!["Korean::YouTube".to_string(), "Korean::Games".to_string()];
        let p = profile("Elden Ring", Some("Korean::Games::Elden Ring"), &[]);
        let s = suggest_decks("Korean", Some("eldenring.exe"), Some(&p), Some("Korean::Games"), &recent, &existing);
        let names: Vec<&str> = s.iter().map(|x| x.deck.as_str()).collect();
        assert_eq!(names, ["Korean::Games", "Korean::Games::Elden Ring", "Korean::Elden Ring", "Korean::YouTube", "Korean"]);
        assert!(s[0].exists && !s[1].exists, "existing decks are marked, new ones will be created");
        assert!(s[0].reason.contains("last time") && s[0].reason.contains("Elden Ring"));
        assert!(s[1].reason.contains("deck set for Elden Ring"));
    }

    #[test]
    fn with_no_program_it_is_recent_decks_and_the_default() {
        let s = suggest_decks("Korean", None, None, None, &["korean::book".to_string(), "Korean::Book".to_string()], &[]);
        assert_eq!(s.iter().map(|x| x.deck.as_str()).collect::<Vec<_>>(), ["korean::book", "Korean"], "case-insensitive duplicates are dropped");
        let s = suggest_decks(" Korean:: ", Some("a.exe"), None, None, &[], &[]);
        assert_eq!(s[0].deck, "Korean::A", "stray colons and spaces do not leak into a deck name");
    }

    #[test]
    fn deck_names_are_tidied() {
        assert_eq!(clean_deck(" Korean :: Games ::"), "Korean::Games");
        assert_eq!(clean_deck("::"), "");
        assert_eq!(clean_deck("Korean::::A"), "Korean::A");
        assert_eq!(clean_deck("한국어"), "한국어");
    }

    #[test]
    fn tags_say_what_the_word_is_and_where_it_came_from() {
        let cfg = AnkiConfig::default();
        let p = profile("Elden Ring", None, &["game", "from rpg"]);
        let tags = tags_for(&cfg, &word(), Some("eldenring.exe"), Some(&p), 1_760_000_000);
        assert_eq!(tags, ["maru", "game", "from_rpg", "topik-1", "pos::verb", "app::elden-ring", "added::2025-10"]);
        let plain = tags_for(&AnkiConfig { auto_tags: false, ..cfg }, &word(), Some("x.exe"), None, 0);
        assert_eq!(plain, ["maru"], "automatic tags can be switched off");
    }

    #[test]
    fn the_month_comes_out_right_across_years_and_leap_days() {
        assert_eq!(year_month(0), "1970-01");
        assert_eq!(year_month(951_782_400), "2000-02", "2000-02-29 is in February");
        assert_eq!(year_month(951_868_800), "2000-03");
        assert_eq!(year_month(1_798_761_599), "2026-12");
        assert_eq!(year_month(1_798_761_600), "2027-01");
    }
}
