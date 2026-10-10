//! The engine: one object the UI talks to.
//!
//! It owns no UI and does no rendering. Every operation either returns data or
//! returns an [`EventRx`] that yields [`Event`]s as the work progresses, so any
//! front end (the Tauri overlay, the CLI, a test) consumes the same stream.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use maru_core::{AppInfo, Capture, Config, Eojeol, Event, Point, Rect, WordState, WordStatus};
use maru_dict::Dictionary;
use maru_llm::{LlmClient, LlmStatus};
use maru_ocr::OcrEngine;
use maru_platform::Platform;
use maru_srs::{AnkiClient, Rating, Vocab, WordRecord, now_secs};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::cache::Cache;
use crate::capture::{CaptureError, Capturer};
use crate::continuous::{self, ContinuousHandle};
use crate::memory::{History, HistoryItem, Memory};
use crate::pipeline;

pub type EventRx = mpsc::UnboundedReceiver<Event>;
pub(crate) type EventTx = mpsc::UnboundedSender<Event>;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error(transparent)]
    Capture(#[from] CaptureError),
    #[error("vocabulary: {0}")]
    Vocab(String),
    #[error("{0}")]
    Other(String),
}

impl From<maru_srs::SrsError> for EngineError {
    fn from(e: maru_srs::SrsError) -> Self {
        EngineError::Vocab(e.to_string())
    }
}

pub type Result<T, E = EngineError> = std::result::Result<T, E>;

/// Everything the engine is built from. Each piece is optional or replaceable,
/// which is also how the tests swap in fakes.
pub struct EngineParts {
    pub config: Config,
    pub analyzer: maru_lang::Analyzer,
    pub dict: Option<Arc<Dictionary>>,
    pub vocab: Arc<Vocab>,
    pub llm: Option<LlmClient>,
    /// Lets requests wait for a model that is still loading. `None` = assume ready.
    pub llm_status: Option<tokio::sync::watch::Receiver<LlmStatus>>,
    pub platform: Platform,
    pub ocr: Option<Arc<dyn OcrEngine>>,
}

/// A click on a word, with the context needed to explain it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeepDiveRequest {
    pub eojeol: Eojeol,
    pub sentence: String,
    pub translation: Option<String>,
    /// The meaning currently shown for the word (becomes the flash-card answer).
    pub meaning: Option<String>,
    pub app: Option<AppInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReviewResult {
    pub status: WordStatus,
    pub interval_days: f32,
    pub due: i64,
}

pub(crate) struct Inner {
    pub cfg: Config,
    pub analyzer: maru_lang::Analyzer,
    pub dict: Option<Arc<Dictionary>>,
    pub vocab: Arc<Vocab>,
    pub llm: Option<LlmClient>,
    pub llm_status: Option<tokio::sync::watch::Receiver<LlmStatus>>,
    pub capturer: Capturer,
    pub platform: Platform,
    pub ocr: Option<Arc<dyn OcrEngine>>,
    pub cache: Mutex<Cache>,
    pub memory: Mutex<Memory>,
    pub history: Mutex<History>,
    pub next_id: AtomicU64,
    pub translate_task: Mutex<Option<JoinHandle<()>>>,
    pub deep_task: Mutex<Option<JoinHandle<()>>>,
    pub anki: Option<AnkiClient>,
    pub runtime: tokio::runtime::Handle,
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

pub(crate) fn status_of(rec: &WordRecord) -> WordStatus {
    WordStatus {
        key: rec.key.clone(),
        state: rec.state,
        lookups: rec.lookups,
        saved: rec.saved,
        meaning: rec.meaning.clone(),
        level: rec.level.clone(),
        due: rec.due,
    }
}

#[derive(Clone)]
pub struct Engine {
    pub(crate) inner: Arc<Inner>,
}

impl Engine {
    /// Must be called from within a Tokio runtime (tasks are spawned onto it).
    pub fn new(parts: EngineParts) -> Self {
        let cfg = parts.config;
        let capturer = Capturer::new(parts.platform.clone(), parts.ocr.clone(), cfg.capture.clone(), cfg.ocr.min_confidence);
        let anki = cfg.learning.anki.enabled.then(|| AnkiClient::new(cfg.learning.anki.clone()));
        let inner = Inner {
            analyzer: parts.analyzer,
            dict: parts.dict,
            vocab: parts.vocab,
            llm: parts.llm,
            llm_status: parts.llm_status,
            capturer,
            platform: parts.platform,
            ocr: parts.ocr,
            cache: Mutex::new(Cache::new(256)),
            memory: Mutex::new(Memory::new(cfg.llm.context_lines)),
            history: Mutex::new(History::new(cfg.continuous.history_len)),
            next_id: AtomicU64::new(1),
            translate_task: Mutex::new(None),
            deep_task: Mutex::new(None),
            anki,
            runtime: tokio::runtime::Handle::current(),
            cfg,
        };
        Self { inner: Arc::new(inner) }
    }

    pub fn config(&self) -> &Config {
        &self.inner.cfg
    }
    pub fn analyzer(&self) -> maru_lang::Analyzer {
        self.inner.analyzer.clone()
    }
    pub fn analyzer_name(&self) -> &'static str {
        self.inner.analyzer.name()
    }
    pub fn has_llm(&self) -> bool {
        self.inner.llm.is_some()
    }
    /// Current model status (for the UI's status line).
    pub fn llm_status(&self) -> LlmStatus {
        match (&self.inner.llm, &self.inner.llm_status) {
            (None, _) => LlmStatus::Disabled,
            (Some(_), Some(rx)) => rx.borrow().clone(),
            (Some(_), None) => LlmStatus::Ready,
        }
    }
    /// The OCR engine in use (for evaluation tools).
    pub fn ocr_engine(&self) -> Option<Arc<dyn OcrEngine>> {
        self.inner.ocr.clone()
    }
    pub fn platform(&self) -> &Platform {
        &self.inner.platform
    }
    pub fn has_ocr(&self) -> bool {
        self.inner.ocr.is_some()
    }
    pub fn has_dictionary(&self) -> bool {
        self.inner.dict.as_ref().is_some_and(|d| !d.is_empty())
    }

    /// Prime the model's KV cache with the task prompts so the first request is fast.
    pub fn warm_up(&self) -> Option<JoinHandle<()>> {
        let llm = self.inner.llm.clone()?;
        Some(self.inner.runtime.spawn(async move { llm.warm_up().await }))
    }

    // ───────────── capture ─────────────

    pub async fn capture_selection(&self) -> Result<Capture> {
        Ok(self.inner.capturer.selection().await?)
    }
    pub async fn capture_region(&self, rect: Rect) -> Result<Capture> {
        Ok(self.inner.capturer.region(rect).await?)
    }
    pub async fn capture_hover(&self, point: Point) -> Result<Capture> {
        Ok(self.inner.capturer.hover(point).await?)
    }

    /// Watch `region` and emit a capture whenever its text changes.
    pub fn start_continuous(&self, region: Rect) -> Result<ContinuousHandle> {
        let ocr = self.inner.ocr.clone().ok_or(CaptureError::NoOcr)?;
        Ok(continuous::spawn(self.inner.platform.screen.clone(), ocr, self.inner.cfg.continuous.clone(), region))
    }

    // ───────────── translation ─────────────

    fn next_id(&self) -> u64 {
        self.inner.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn spawn_into(&self, slot: &Mutex<Option<JoinHandle<()>>>, fut: impl std::future::Future<Output = ()> + Send + 'static) {
        let handle = self.inner.runtime.spawn(fut);
        if let Some(previous) = lock(slot).replace(handle) {
            previous.abort(); // superseded: its channel closes, the UI also drops stale ids
        }
    }

    /// Translate and break down `capture`. A newer call cancels an older one.
    pub fn translate(&self, capture: Capture) -> (u64, EventRx) {
        let id = self.next_id();
        let (tx, rx) = mpsc::unbounded_channel();
        let inner = self.inner.clone();
        self.spawn_into(&self.inner.translate_task, pipeline::translate(inner, id, capture, tx));
        (id, rx)
    }

    /// Instant, offline word lookup (hover mode): breakdown + dictionary, no model call.
    pub fn lookup_word(&self, capture: Capture) -> (u64, EventRx) {
        let id = self.next_id();
        let (tx, rx) = mpsc::unbounded_channel();
        let inner = self.inner.clone();
        self.spawn_into(&self.inner.translate_task, pipeline::lookup(inner, id, capture, tx));
        (id, rx)
    }

    /// Explain one word: dictionary entries, how its form is built, and a short
    /// tutor-style explanation. Records the lookup for the learning loop.
    pub fn deep_dive(&self, request: DeepDiveRequest) -> (u64, EventRx) {
        let id = self.next_id();
        let (tx, rx) = mpsc::unbounded_channel();
        let inner = self.inner.clone();
        self.spawn_into(&self.inner.deep_task, pipeline::deep_dive(inner, id, request, tx));
        (id, rx)
    }

    /// Analyse Korean text exactly as the overlay would (used by `maru explain`).
    pub fn analyze(&self, text: &str) -> Vec<Eojeol> {
        let mut e = self.inner.analyzer.analyze(text).eojeols;
        crate::annotate::annotate(&mut e, self.inner.dict.as_deref(), Some(&self.inner.vocab), 3);
        e
    }

    /// Forget cached translations (benchmarks and "retranslate" use this).
    pub fn clear_cache(&self) {
        *lock(&self.inner.cache) = Cache::new(256);
    }

    /// Recent lines, oldest first.
    pub fn history(&self) -> Vec<HistoryItem> {
        lock(&self.inner.history).items()
    }

    // ───────────── vocabulary ─────────────

    pub fn save_word(&self, key: &str) -> Result<WordStatus> {
        self.inner.vocab.save(key, now_secs())?;
        // A card with an empty answer is useless: borrow the dictionary's meaning and level.
        if let Some(dict) = &self.inner.dict
            && let Ok(entries) = dict.lookup(key)
            && let Some(entry) = entries.first()
        {
            let meaning = entry.senses.first().map(|s| maru_dict::short_gloss(&s.gloss));
            let _ = self.inner.vocab.enrich(key, meaning.as_deref(), entry.level.as_deref());
        }
        let rec = self.inner.vocab.get(key)?.ok_or_else(|| EngineError::Vocab(format!("unknown word: {key}")))?;
        self.push_to_anki(rec.clone());
        Ok(status_of(&rec))
    }

    pub fn unsave_word(&self, key: &str) -> Result<()> {
        Ok(self.inner.vocab.unsave(key)?)
    }

    pub fn set_known(&self, key: &str, known: bool) -> Result<WordStatus> {
        Ok(status_of(&self.inner.vocab.mark_known(key, known, now_secs())?))
    }

    /// Mark every dictionary word at TOPIK `level` or easier as known (glosses fade for all of them).
    pub fn mark_known_up_to_level(&self, level: u8) -> Result<u32> {
        let dict = self.inner.dict.as_ref().ok_or_else(|| EngineError::Other("no dictionary is installed".into()))?;
        let lemmas = dict.lemmas_up_to_topik(level).map_err(|e| EngineError::Other(e.to_string()))?;
        Ok(self.inner.vocab.mark_known_bulk(lemmas.iter().map(String::as_str), now_secs())?)
    }

    pub fn review_word(&self, key: &str, rating: u8) -> Result<ReviewResult> {
        let rating = Rating::from_u8(rating).ok_or_else(|| EngineError::Other("rating must be 1 (again) to 4 (easy)".into()))?;
        let outcome = self.inner.vocab.review(key, rating, now_secs())?;
        let rec = self.inner.vocab.get(key)?.ok_or_else(|| EngineError::Vocab(format!("unknown word: {key}")))?;
        Ok(ReviewResult { status: status_of(&rec), interval_days: outcome.interval_days, due: outcome.due })
    }

    /// Sentences a word was met in (newest first), for the review card.
    pub fn word_contexts(&self, key: &str, limit: u32) -> Result<Vec<maru_srs::Context>> {
        Ok(self.inner.vocab.contexts(key, limit)?)
    }

    /// Most recently looked-up words first.
    pub fn recent_words(&self, limit: u32) -> Result<Vec<WordStatus>> {
        Ok(self.inner.vocab.recent(limit)?.iter().map(status_of).collect())
    }

    pub fn due_words(&self, limit: u32) -> Result<Vec<WordStatus>> {
        Ok(self.inner.vocab.due(now_secs(), limit)?.iter().map(status_of).collect())
    }

    pub fn vocab_stats(&self) -> Result<maru_srs::VocabStats> {
        Ok(self.inner.vocab.stats(now_secs())?)
    }

    fn push_to_anki(&self, rec: WordRecord) {
        let Some(anki) = self.inner.anki.as_ref() else { return };
        let inner = self.inner.clone();
        let contexts = self.inner.vocab.contexts(&rec.key, 2).unwrap_or_default();
        let _ = anki; // borrowed above only to test the setting; the task re-borrows from `inner`
        self.inner.runtime.spawn(async move {
            if let Some(anki) = inner.anki.as_ref()
                && let Err(e) = anki.add(&rec, &contexts).await
            {
                tracing::warn!("could not add {} to Anki: {e}", rec.key);
            }
        });
    }

    /// Send one saved word to Anki now. `Ok(None)` = Anki already has that card.
    pub async fn export_anki(&self, key: &str) -> Result<Option<u64>> {
        let anki = self
            .inner
            .anki
            .as_ref()
            .ok_or_else(|| EngineError::Other("Anki export is disabled: set [learning.anki] enabled = true".into()))?;
        let rec = self.inner.vocab.get(key)?.ok_or_else(|| EngineError::Vocab(format!("unknown word: {key}")))?;
        let contexts = self.inner.vocab.contexts(key, 2)?;
        Ok(anki.add(&rec, &contexts).await?)
    }

    /// Write every saved word as a tab-separated file Anki can import. Returns the card count.
    pub fn export_tsv(&self, path: &Path) -> Result<usize> {
        let words = self.inner.vocab.saved()?;
        let cards: Vec<_> = words
            .into_iter()
            .map(|w| {
                let ctx = self.inner.vocab.contexts(&w.key, 2).unwrap_or_default();
                (w, ctx)
            })
            .collect();
        let file = std::fs::File::create(path).map_err(|e| EngineError::Other(format!("{}: {e}", path.display())))?;
        maru_srs::export_tsv(std::io::BufWriter::new(file), &cards)?;
        Ok(cards.len())
    }

    /// For tests and the CLI: current state of a word.
    pub fn word_state(&self, key: &str) -> WordState {
        self.inner.vocab.get(key).ok().flatten().map(|r| r.state).unwrap_or(WordState::New)
    }

    pub fn started_at() -> Instant {
        Instant::now()
    }
}
