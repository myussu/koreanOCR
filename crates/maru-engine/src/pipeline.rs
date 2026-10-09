//! The request pipelines. Order of events is a contract with the UI:
//!
//! `Started` → `Breakdown` (instant, offline) → `Translation`/`Gloss` (streaming, concurrent)
//! → `Timing`s → `Done`. Errors are events too, never panics, and an LLM failure still
//! leaves the user with the breakdown and dictionary meanings.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use maru_core::config::GlossMode;
use maru_core::event::{DeepSection, Side, Stage, Variant};
use maru_core::{Capture, CaptureSource, Direction, Eojeol, Event, Gloss, WordBox, WordState};
use maru_lang::hangul::detect_direction;
use maru_llm::{DeepRequest, EnKoRequest, GlossRequest, GlossWord, KoEnRequest, LlmChunk, LlmClient, PromptContext};
use maru_srs::{Context, LookupInfo, now_secs};
use tokio::sync::oneshot;

use crate::annotate;
use crate::cache::{CacheKey, Cached};
use crate::engine::{DeepDiveRequest, EventTx, Inner, lock, status_of};
use crate::memory::HistoryItem;

/// Cap on words sent for glossing: keeps the answer fast for very long selections.
const MAX_GLOSS_WORDS: usize = 40;
const NO_LLM: &str = "No language model is running, so only the offline breakdown is shown. Start one with `maru setup llm`.";

#[derive(Clone)]
struct Emitter {
    tx: EventTx,
    id: u64,
}

impl Emitter {
    fn send(&self, e: Event) {
        let _ = self.tx.send(e);
    }
    fn error(&self, stage: Stage, message: impl Into<String>) {
        self.send(Event::Error { id: self.id, stage, message: message.into() });
    }
    fn timing(&self, stage: Stage, since: Instant) {
        self.send(Event::Timing { id: self.id, stage, ms: since.elapsed().as_millis().min(u128::from(u32::MAX)) as u32 });
    }
    fn done(&self) {
        self.send(Event::Done { id: self.id });
    }
}

/// Collapse whitespace and cut very long text at a sentence boundary.
fn prepare_text(raw: &str, max_chars: usize) -> (String, bool) {
    let text = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= max_chars {
        return (text, false);
    }
    let head: String = text.chars().take(max_chars).collect();
    let cut = head
        .char_indices()
        .filter(|(_, c)| matches!(c, '.' | '!' | '?' | '。' | '！' | '？' | '…'))
        .map(|(i, c)| i + c.len_utf8())
        .next_back()
        .or_else(|| head.rfind(' '))
        .unwrap_or(head.len());
    (head[..cut].trim().to_string(), true)
}

fn core_text(s: &str) -> &str {
    s.trim_matches(|c: char| !c.is_alphanumeric() && !maru_lang::hangul::is_hangul(c))
}

fn parts_of(e: &Eojeol) -> String {
    e.morphs
        .iter()
        .filter(|m| m.role != maru_core::Role::Symbol)
        .map(maru_lang::display_form)
        .collect::<Vec<_>>()
        .join(" + ")
}

fn gloss_words(eojeols: &[Eojeol], skip_known: bool) -> Vec<GlossWord> {
    eojeols
        .iter()
        .filter(|e| e.content && !(skip_known && e.state == WordState::Known))
        .take(MAX_GLOSS_WORDS)
        .map(|e| GlossWord {
            index: e.index,
            surface: core_text(&e.surface).to_string(),
            parts: parts_of(e),
            candidates: e.dict_glosses.clone(),
        })
        .collect()
}

/// Analyse Korean text, annotate it with what we know offline, and emit the breakdown.
async fn analyze_and_emit(inner: &Arc<Inner>, em: &Emitter, side: Side, text: &str, words: Vec<WordBox>) -> Vec<Eojeol> {
    let t = Instant::now();
    let work = {
        let inner = inner.clone();
        let text = text.to_string();
        move || {
            let mut analysis = inner.analyzer.analyze(&text);
            annotate::assign_rects(&mut analysis.eojeols, &words);
            annotate::annotate(&mut analysis.eojeols, inner.dict.as_deref(), Some(&inner.vocab), 3);
            analysis
        }
    };
    let Ok(analysis) = tokio::task::spawn_blocking(work).await else {
        em.error(Stage::Morphology, "the Korean analyser crashed");
        return Vec::new();
    };
    em.timing(Stage::Morphology, t);
    em.send(Event::Breakdown {
        id: em.id,
        side,
        eojeols: analysis.eojeols.clone(),
        romanization: analysis.romanization,
        pronunciation: analysis.pronunciation,
    });
    analysis.eojeols
}

fn prompt_context(inner: &Inner, capture: &Capture, profile: Option<&maru_core::config::AppProfile>) -> PromptContext {
    let app = capture.app.as_ref().map(|a| a.exe.as_str()).unwrap_or("");
    PromptContext {
        hint: profile.and_then(|p| p.hint.clone()),
        glossary: profile.map(|p| p.glossary.clone()).unwrap_or_default(),
        history: lock(&inner.memory).context(app, Instant::now()),
        from_ocr: matches!(capture.source, CaptureSource::Ocr | CaptureSource::Continuous),
    }
}

/// Forward model output to the UI and remember it for the cache.
fn forward(em: &Emitter, collected: &Mutex<Cached>, side: Side, first_token: &AtomicBool, started: Instant, chunk: LlmChunk) {
    match chunk {
        LlmChunk::Text { variant, delta } => {
            if !first_token.swap(true, Ordering::Relaxed) {
                em.timing(Stage::FirstToken, started);
            }
            em.send(Event::Translation { id: em.id, variant, delta });
        }
        LlmChunk::TextDone { variant, text } => {
            em.send(Event::TranslationDone { id: em.id, variant, text: text.clone() });
            lock(collected).translations.push((variant, text));
        }
        LlmChunk::Gloss(g) => {
            em.send(Event::Gloss { id: em.id, side, gloss: g.clone() });
            lock(collected).glosses.push((side, g));
        }
        LlmChunk::Note(n) => {
            em.send(Event::Note { id: em.id, text: n.clone() });
            lock(collected).note = Some(n);
        }
        LlmChunk::Deep { .. } => {}
    }
}

async fn run_glosses(
    llm: &LlmClient,
    em: &Emitter,
    collected: &Mutex<Cached>,
    side: Side,
    sentence: &str,
    words: &[GlossWord],
    ctx: &PromptContext,
) {
    if words.is_empty() {
        return;
    }
    let started = Instant::now();
    let seen = AtomicBool::new(true); // first-token timing is for the translation only
    let result = llm
        .glosses(&GlossRequest { sentence, words, ctx }, &mut |c| forward(em, collected, side, &seen, started, c))
        .await;
    if let Err(e) = result {
        em.error(Stage::Llm, format!("word meanings unavailable: {e}"));
    }
}

fn effective_gloss_mode(inner: &Inner, profile: Option<&maru_core::config::AppProfile>) -> GlossMode {
    profile.and_then(|p| p.gloss_mode).unwrap_or(inner.cfg.learning.gloss_mode)
}

pub(crate) async fn translate(inner: Arc<Inner>, id: u64, capture: Capture, tx: EventTx) {
    let em = Emitter { tx, id };
    let started = Instant::now();

    let (text, truncated) = prepare_text(&capture.text, inner.cfg.capture.max_chars);
    let Some(direction) = detect_direction(&text) else {
        em.error(Stage::Capture, "There is no Korean or English text in the selection.");
        em.done();
        return;
    };
    let app_exe = capture.app.as_ref().map(|a| a.exe.clone()).unwrap_or_default();
    let profile = inner.cfg.profile_for(&app_exe).cloned();
    let register = profile.as_ref().and_then(|p| p.register).unwrap_or(inner.cfg.learning.register);
    let mut capture = capture;
    capture.text = text.clone();

    let profile_key = (profile.as_ref().map(|p| p.name.clone()), profile.as_ref().map(|p| p.glossary.clone()), profile.as_ref().and_then(|p| p.hint.clone()));
    let key = CacheKey::new(&text, direction, register, &profile_key);
    let cached = lock(&inner.cache).get(&key);

    em.send(Event::Started { id, direction, capture: capture.clone(), cached: cached.is_some() });
    if truncated {
        em.send(Event::Note { id, text: "Long selection: showing the first part only.".into() });
    }

    let skip_known = effective_gloss_mode(&inner, profile.as_ref()) != GlossMode::All;
    let words = capture.words.clone();

    if let Some(hit) = cached {
        replay(&inner, &em, direction, register, &text, words, hit).await;
        em.timing(Stage::Total, started);
        em.done();
        return;
    }

    let ctx = prompt_context(&inner, &capture, profile.as_ref());
    let collected = Mutex::new(Cached::default());

    match direction {
        Direction::KoToEn => {
            let eojeols = analyze_and_emit(&inner, &em, Side::Source, &text, words).await;
            let Some(llm) = inner.llm.clone() else {
                em.error(Stage::Llm, NO_LLM);
                em.timing(Stage::Total, started);
                em.done();
                return;
            };
            let gloss_list = gloss_words(&eojeols, skip_known);
            let first_token = AtomicBool::new(false);
            let llm_started = Instant::now();

            let translate_fut = async {
                let r = llm
                    .translate_ko_en(&KoEnRequest { text: &text, ctx: &ctx }, &mut |c| {
                        forward(&em, &collected, Side::Source, &first_token, llm_started, c)
                    })
                    .await;
                if let Err(e) = r {
                    em.error(Stage::Llm, e.to_string());
                }
            };
            tokio::join!(translate_fut, run_glosses(&llm, &em, &collected, Side::Source, &text, &gloss_list, &ctx));
            em.timing(Stage::Llm, llm_started);
        }
        Direction::EnToKo => {
            let Some(llm) = inner.llm.clone() else {
                em.error(Stage::Llm, NO_LLM);
                em.timing(Stage::Total, started);
                em.done();
                return;
            };
            let primary = Variant::from_register(register);
            let (primary_tx, primary_rx) = oneshot::channel::<String>();
            let primary_tx = Mutex::new(Some(primary_tx));
            let first_token = AtomicBool::new(false);
            let llm_started = Instant::now();

            let translate_fut = async {
                let r = llm
                    .translate_en_ko(&EnKoRequest { text: &text, register, ctx: &ctx }, &mut |c| {
                        if let LlmChunk::TextDone { variant, text } = &c
                            && *variant == primary
                            && let Some(sender) = lock(&primary_tx).take()
                        {
                            let _ = sender.send(text.clone());
                        }
                        forward(&em, &collected, Side::Target, &first_token, llm_started, c)
                    })
                    .await;
                if let Err(e) = r {
                    em.error(Stage::Llm, e.to_string());
                }
            };
            // As soon as the requested register is written, break the Korean down and
            // gloss it while the other registers are still streaming.
            let follow_up = async {
                let Ok(korean) = primary_rx.await else { return };
                let eojeols = analyze_and_emit(&inner, &em, Side::Target, &korean, Vec::new()).await;
                let list = gloss_words(&eojeols, skip_known);
                run_glosses(&llm, &em, &collected, Side::Target, &korean, &list, &ctx).await;
            };
            tokio::join!(translate_fut, follow_up);
            em.timing(Stage::Llm, llm_started);
        }
    }

    let result = std::mem::take(&mut *lock(&collected));
    if result.translations.is_empty() {
        em.error(Stage::Llm, "The model did not return a translation. Check that it is loaded and responding.");
    } else {
        let main = result
            .translations
            .iter()
            .find(|(v, _)| matches!(v, Variant::Primary) || *v == Variant::from_register(register))
            .or_else(|| result.translations.first())
            .map(|(_, t)| t.clone())
            .unwrap_or_default();
        lock(&inner.memory).push(&app_exe, &text, &main, Instant::now());
        lock(&inner.history).push(HistoryItem {
            id,
            at: now_secs(),
            source: text.clone(),
            translation: main,
            direction,
            app: capture.app.as_ref().map(|a| a.exe.clone()),
        });
        lock(&inner.cache).put(key, result);
    }
    em.timing(Stage::Total, started);
    em.done();
}

/// Re-send a cached result as the same event stream, re-annotating so your
/// current vocabulary state is shown (it may have changed since).
async fn replay(
    inner: &Arc<Inner>,
    em: &Emitter,
    direction: Direction,
    register: maru_core::Register,
    text: &str,
    words: Vec<WordBox>,
    hit: Cached,
) {
    match direction {
        Direction::KoToEn => {
            analyze_and_emit(inner, em, Side::Source, text, words).await;
        }
        Direction::EnToKo => {
            let primary = Variant::from_register(register);
            if let Some((_, korean)) = hit.translations.iter().find(|(v, _)| *v == primary) {
                analyze_and_emit(inner, em, Side::Target, korean, Vec::new()).await;
            }
        }
    }
    for (variant, t) in &hit.translations {
        em.send(Event::Translation { id: em.id, variant: *variant, delta: t.clone() });
        em.send(Event::TranslationDone { id: em.id, variant: *variant, text: t.clone() });
    }
    for (side, gloss) in &hit.glosses {
        em.send(Event::Gloss { id: em.id, side: *side, gloss: Gloss { ..gloss.clone() } });
    }
    if let Some(note) = &hit.note {
        em.send(Event::Note { id: em.id, text: note.clone() });
    }
}

/// Hover mode: no model, just the breakdown and dictionary for the word under the pointer.
pub(crate) async fn lookup(inner: Arc<Inner>, id: u64, capture: Capture, tx: EventTx) {
    let em = Emitter { tx, id };
    let started = Instant::now();
    let (text, _) = prepare_text(&capture.text, 80);
    if detect_direction(&text) != Some(Direction::KoToEn) {
        em.error(Stage::Capture, "Hover lookup works on Korean words.");
        em.done();
        return;
    }
    let mut capture = capture;
    capture.text = text.clone();
    let words = capture.words.clone();
    em.send(Event::Started { id, direction: Direction::KoToEn, capture, cached: false });
    analyze_and_emit(&inner, &em, Side::Source, &text, words).await;
    em.timing(Stage::Total, started);
    em.done();
}

pub(crate) async fn deep_dive(inner: Arc<Inner>, id: u64, req: DeepDiveRequest, tx: EventTx) {
    let em = Emitter { tx, id };
    let started = Instant::now();
    let e = &req.eojeol;
    let key = e.vocab_key().to_string();

    // 1. Learning loop: this deliberate click is a lookup.
    if inner.cfg.learning.track_lookups {
        let screenshot = None;
        let info = LookupInfo {
            key: key.clone(),
            surface: e.surface.clone(),
            pos: e.pos.label().to_string(),
            meaning: req.meaning.clone().or_else(|| e.dict_glosses.first().cloned()),
            level: e.level.clone(),
            context: Some(Context {
                sentence: req.sentence.clone(),
                translation: req.translation.clone(),
                app: req.app.as_ref().map(|a| a.exe.clone()),
                screenshot,
            }),
        };
        match inner.vocab.record_lookup(&info, now_secs()) {
            Ok(rec) => em.send(Event::WordState { id, status: status_of(&rec) }),
            Err(err) => em.error(Stage::Dictionary, format!("could not record the lookup: {err}")),
        }
    }

    // 2. Offline: dictionary entries + how the form is built.
    let entries = inner.dict.as_ref().map(|d| d.lookup_word(e).unwrap_or_default()).unwrap_or_default();
    let conjugation = maru_lang::conjugation_steps(e);
    let dictionary_summary = entries
        .iter()
        .take(3)
        .map(|en| format!("{}: {}", en.pos, en.senses.iter().take(3).map(|s| s.gloss.as_str()).collect::<Vec<_>>().join("; ")))
        .collect::<Vec<_>>()
        .join(" | ");
    em.send(Event::DeepEntries { id, entries, conjugation });
    em.timing(Stage::Dictionary, started);

    // 3. Streaming tutor explanation.
    let Some(llm) = inner.llm.clone() else {
        em.error(Stage::Llm, NO_LLM);
        em.done();
        return;
    };
    let ctx = PromptContext::default();
    let result = llm
        .deep_dive(
            &DeepRequest { key: &key, surface: &e.surface, sentence: &req.sentence, dictionary: &dictionary_summary, ctx: &ctx },
            &mut |chunk| {
                if let LlmChunk::Deep { section, delta } = chunk {
                    em.send(Event::DeepDelta { id, section: section_of(section), delta });
                }
            },
        )
        .await;
    if let Err(err) = result {
        em.error(Stage::Llm, err.to_string());
    }
    em.timing(Stage::Total, started);
    em.done();
}

fn section_of(s: DeepSection) -> DeepSection {
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitespace_is_collapsed() {
        assert_eq!(prepare_text("  저는\n학생\t입니다  ", 100), ("저는 학생 입니다".to_string(), false));
    }

    #[test]
    fn long_text_is_cut_at_a_sentence_end() {
        let text = "첫 번째 문장입니다. 두 번째 문장입니다. 세 번째 문장이 아주 길게 계속 이어집니다";
        let (cut, truncated) = prepare_text(text, 30);
        assert!(truncated);
        assert_eq!(cut, "첫 번째 문장입니다. 두 번째 문장입니다.");
    }

    #[test]
    fn long_text_without_punctuation_is_cut_at_a_space() {
        let (cut, truncated) = prepare_text(&"word ".repeat(40), 22);
        assert!(truncated);
        assert!(cut.ends_with("word") && cut.chars().count() <= 22, "{cut:?}");
    }
}
