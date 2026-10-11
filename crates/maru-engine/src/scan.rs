//! Screen scan: find every piece of Korean text on a screen and give each a short English label.
//!
//! Order of events (a contract with the overlay): `ScanStarted` → `ScanItems` → `ScanLabel`s → `Done`.
//! `ScanItems` arrives as soon as the screen has been read and carries everything that is free:
//! the dictionary's meaning, glossary entries, and labels remembered from earlier scans. Only
//! what is still unknown goes to the model, in a few batched requests whose answers stream
//! back line by line, so the screen fills in while the model is still writing.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use maru_core::config::{AppProfile, GlossMode};
use maru_core::event::Stage;
use maru_core::{Event, LabelSource, Point, Rect, ScanItem, WordState};
use maru_llm::{LabelRequest, PromptContext};
use maru_ocr::OcrOutput;
use maru_srs::now_secs;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::annotate;
use crate::capture::clip;
use crate::engine::{EventTx, Inner};
use crate::labels::key;
use crate::pipeline::{Emitter, await_llm, effective_gloss_mode};

/// A screen full of text can have hundreds of lines; labelling more than this is clutter.
const MAX_ITEMS: usize = 150;
/// Strings per model request: short answers, so the first labels appear quickly.
const BATCH: usize = 24;
/// Requests in flight at once (the model server has two slots).
const PARALLEL_BATCHES: usize = 2;
/// Below this the "text" is almost always texture or an icon.
const MIN_CONFIDENCE: f32 = 0.45;
const MIN_SIDE_PX: f32 = 8.0;
/// A long dialogue line is clipped before it goes to the model.
const MAX_PROMPT_CHARS: usize = 120;

/// One Korean line the OCR found.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Found {
    pub rect: Rect,
    pub text: String,
}

/// The lines worth labelling: contain Hangul, are readable, and are not the detector's duplicates.
pub(crate) fn found_items(out: &OcrOutput) -> Vec<Found> {
    let mut items: Vec<Found> = Vec::new();
    for line in &out.lines {
        let text = line.text.split_whitespace().collect::<Vec<_>>().join(" ");
        if !text.chars().any(|c| ('가'..='힣').contains(&c)) {
            continue;
        }
        if line.confidence < MIN_CONFIDENCE || line.rect.w < MIN_SIDE_PX || line.rect.h < MIN_SIDE_PX {
            continue;
        }
        // Overlapping boxes sometimes report the same line twice.
        if items.iter().any(|i| i.text == text && i.rect.overlap_fraction(&line.rect) > 0.5) {
            continue;
        }
        items.push(Found { rect: line.rect, text });
        if items.len() == MAX_ITEMS {
            break;
        }
    }
    items
}

/// Everything we can say about the found lines without asking the model.
fn offline_items(inner: &Inner, found: &[Found], mode: GlossMode, profile: Option<&AppProfile>, scope: &str, first_id: u32) -> Vec<ScanItem> {
    let glossary: HashMap<String, &String> = profile.map(|p| p.glossary.iter().map(|(k, v)| (key(k), v)).collect()).unwrap_or_default();
    let unglossed: Vec<&str> = found.iter().map(|f| f.text.as_str()).filter(|t| !glossary.contains_key(&key(t))).collect();
    let remembered = inner.labels.as_ref().map(|c| c.get_many(scope, &unglossed)).unwrap_or_default();

    let mut items = Vec::with_capacity(found.len());
    for f in found {
        let mut eojeols = inner.analyzer.analyze(&f.text).eojeols;
        annotate::annotate(&mut eojeols, inner.dict.as_deref(), Some(&inner.vocab), 1);
        let content: Vec<_> = eojeols.iter().filter(|e| e.content).collect();
        let known = !content.is_empty() && content.iter().all(|e| e.state == WordState::Known);
        if known && mode == GlossMode::HideKnown {
            continue;
        }
        let mut meanings: Vec<&str> = Vec::new();
        for g in content.iter().filter_map(|e| e.dict_glosses.first()) {
            if !meanings.contains(&g.as_str()) {
                meanings.push(g);
            }
        }
        let dictionary = meanings.into_iter().take(3).collect::<Vec<_>>().join(" · ");

        let k = key(&f.text);
        let (label, source) = if let Some(fixed) = glossary.get(&k) {
            ((*fixed).clone(), LabelSource::Glossary)
        } else if let Some(label) = remembered.get(&k) {
            (label.clone(), LabelSource::Cache)
        } else if !dictionary.is_empty() {
            (dictionary, LabelSource::Dictionary)
        } else {
            (String::new(), LabelSource::Pending)
        };
        items.push(ScanItem { item: first_id + items.len() as u32, rect: f.rect, text: f.text.clone(), label, source, known: known && mode != GlossMode::All });
    }
    items
}

/// Strings that still need the model, grouped so identical texts are asked once:
/// `(key, text, items that share it)`.
fn needs_model(items: &[ScanItem]) -> Vec<(String, String, Vec<u32>)> {
    let mut groups: Vec<(String, String, Vec<u32>)> = Vec::new();
    for it in items.iter().filter(|i| matches!(i.source, LabelSource::Pending | LabelSource::Dictionary) && !i.known) {
        let k = key(&it.text);
        match groups.iter_mut().find(|g| g.0 == k) {
            Some(g) => g.2.push(it.item),
            None => groups.push((k, it.text.chars().take(MAX_PROMPT_CHARS).collect(), vec![it.item])),
        }
    }
    groups
}

/// What a scan reads: the live screen, or a picture of one (screenshots, for `maru scan`).
pub(crate) enum Source {
    Screen(Rect),
    Image(image::RgbImage),
}

pub(crate) async fn run(inner: Arc<Inner>, id: u64, source: Source, tx: EventTx) {
    let em = Emitter::new(tx, id);
    let started = Instant::now();
    let screen = match &source {
        Source::Screen(r) => *r,
        Source::Image(img) => Rect::new(0.0, 0.0, img.width() as f32, img.height() as f32),
    };
    em.send(Event::ScanStarted { id, screen });

    let Some(ocr) = inner.scan_ocr.clone().or_else(|| inner.ocr.clone()) else {
        em.error(Stage::Ocr, "No OCR engine is available, so the screen cannot be read. Run `maru setup ocr`.");
        em.done();
        return;
    };

    // ── capture + OCR (blocking: GDI and ONNX) ──
    let read = {
        let inner = inner.clone();
        tokio::task::spawn_blocking(move || -> Result<(OcrOutput, std::time::Duration, std::time::Duration), String> {
            let t = Instant::now();
            let (image, area) = match source {
                Source::Screen(wanted) => {
                    let area = clip(wanted, inner.platform.screen.screen_bounds());
                    if area.is_empty() {
                        return Err(format!("the area {wanted:?} is outside the screen"));
                    }
                    (inner.platform.screen.grab(area).map_err(|e| e.to_string())?, area)
                }
                Source::Image(image) => {
                    let area = Rect::new(0.0, 0.0, image.width() as f32, image.height() as f32);
                    (image, area)
                }
            };
            let grab = t.elapsed();
            let t = Instant::now();
            let out = ocr.recognize(&image).map_err(|e| e.to_string())?;
            Ok((out.to_screen(Point { x: area.x, y: area.y }, 1.0), grab, t.elapsed()))
        })
        .await
    };
    let (out, grab, read_time) = match read {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            em.error(Stage::Capture, e);
            em.done();
            return;
        }
        Err(_) => {
            em.error(Stage::Ocr, "the screen reader crashed");
            em.done();
            return;
        }
    };
    em.send(Event::Timing { id, stage: Stage::Capture, ms: grab.as_millis() as u32 });
    em.send(Event::Timing { id, stage: Stage::Ocr, ms: read_time.as_millis() as u32 });

    // ── the free part: dictionary, glossary, remembered labels ──
    let app = inner.platform.screen.foreground_app();
    let scope = app.as_ref().map(|a| a.exe.clone()).unwrap_or_default();
    *crate::engine::lock(&inner.scan_scope) = (id, scope.clone());
    // Lines the person once joined by hand are joined again.
    let found = {
        let rules = inner.labels.as_ref().map(|c| c.joins_for(&scope)).unwrap_or_default();
        crate::joins::apply_rules(found_items(&out), &rules)
    };
    label(&inner, &em, found, 0, &scope, started).await;

    em.send(Event::Timing { id, stage: Stage::Total, ms: started.elapsed().as_millis() as u32 });
    em.done();
}

/// Give `found` its labels: what is free at once (`ScanItems`), then the model's answers, streamed
/// (`ScanLabel`). Item numbers start at `first_id`, so a later join can add items to a scan that is on screen.
async fn label(inner: &Arc<Inner>, em: &Emitter, found: Vec<Found>, first_id: u32, scope: &str, started: Instant) {
    let id = em.id();
    let scope = scope.to_string();
    let profile = inner.cfg.profile_for(&scope).cloned();
    let mode = effective_gloss_mode(inner, profile.as_ref());
    let t = Instant::now();
    let items = {
        let (inner, profile, scope) = (inner.clone(), profile.clone(), scope.clone());
        tokio::task::spawn_blocking(move || offline_items(&inner, &found, mode, profile.as_ref(), &scope, first_id)).await.unwrap_or_default()
    };
    em.send(Event::Timing { id, stage: Stage::Morphology, ms: t.elapsed().as_millis() as u32 });
    let groups = needs_model(&items);
    em.send(Event::ScanItems { id, items });

    // ── the rest: the model, in batches, streamed ──
    if !groups.is_empty()
        && let Some(client) = await_llm(inner, em).await
    {
        let ctx = PromptContext {
            hint: profile.as_ref().and_then(|p| p.hint.clone()),
            glossary: profile.as_ref().map(|p| p.glossary.clone()).unwrap_or_default(),
            history: Vec::new(),
            from_ocr: true,
        };
        let first_label = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let gate = Arc::new(Semaphore::new(PARALLEL_BATCHES));
        let mut batches = JoinSet::new(); // dropped with this task, so a newer scan cancels the requests
        for batch in groups.chunks(BATCH) {
            let (client, em, ctx, gate, inner, scope, first_label) =
                (client.clone(), em.clone(), ctx.clone(), gate.clone(), inner.clone(), scope.clone(), first_label.clone());
            let batch = batch.to_vec();
            batches.spawn(async move {
                let Ok(_slot) = gate.acquire().await else { return };
                let asked: Vec<(usize, &str)> = batch.iter().enumerate().map(|(n, g)| (n, g.1.as_str())).collect();
                let learned = Mutex::new(Vec::<(String, String)>::new());
                let result = client
                    .labels(&LabelRequest { items: &asked, ctx: &ctx }, &mut |line| {
                        let Some((_, text, ids)) = batch.get(line.index) else { return };
                        if line.label == "?" {
                            return; // not text: keep whatever the dictionary said
                        }
                        if !first_label.swap(true, std::sync::atomic::Ordering::Relaxed) {
                            em.send(Event::Timing { id: em.id(), stage: Stage::FirstToken, ms: started.elapsed().as_millis() as u32 });
                        }
                        for &item in ids {
                            em.send(Event::ScanLabel { id: em.id(), item, label: line.label.clone(), source: LabelSource::Model });
                        }
                        learned.lock().unwrap_or_else(|p| p.into_inner()).push((text.clone(), line.label));
                    })
                    .await;
                if let Err(e) = result {
                    em.error(Stage::Llm, e.to_string());
                }
                // Remember what was learned, even from a half-finished answer.
                let learned = learned.into_inner().unwrap_or_else(|p| p.into_inner());
                if !learned.is_empty()
                    && let Some(cache) = inner.labels.clone()
                {
                    let _ = tokio::task::spawn_blocking(move || cache.put_many(&scope, &learned, now_secs())).await;
                }
            });
        }
        while batches.join_next().await.is_some() {}
    }
}

/// A join the person made by hand: `pieces` (labels on screen) become one item `item` of scan `scan_id`.
/// The join is remembered for the program the scan was made in, so the next scan makes it again.
pub(crate) async fn join(inner: Arc<Inner>, scan_id: u64, item: u32, pieces: Vec<crate::joins::JoinPiece>, tx: EventTx) {
    let em = Emitter::new(tx, scan_id);
    let started = Instant::now();
    let scope = {
        let current = crate::engine::lock(&inner.scan_scope);
        if current.0 != scan_id {
            drop(current);
            em.error(Stage::Capture, "That scan is no longer on screen; scan again and join the labels.");
            em.done();
            return;
        }
        current.1.clone()
    };
    let ordered = crate::joins::reading_order(pieces.into_iter().map(|p| Found { rect: p.rect, text: p.text }).collect());
    let parts: Vec<String> = ordered.iter().map(|f| f.text.clone()).collect();
    let Some(joined) = crate::joins::join_pieces(ordered) else {
        em.done();
        return;
    };
    if let Some(cache) = inner.labels.clone() {
        let (scope, text) = (scope.clone(), joined.text.clone());
        let _ = tokio::task::spawn_blocking(move || cache.put_join(&scope, &parts, &text, now_secs())).await;
    }
    label(&inner, &em, vec![joined], item, &scope, started).await;
    em.done();
}

#[cfg(test)]
mod tests {
    use maru_ocr::OcrLine;

    use super::*;

    fn line(text: &str, x: f32, y: f32, conf: f32) -> OcrLine {
        OcrLine { text: text.into(), rect: Rect::new(x, y, 80.0, 20.0), words: vec![], confidence: conf }
    }

    #[test]
    fn only_readable_korean_lines_are_kept() {
        let out = OcrOutput {
            lines: vec![
                line("인벤토리", 10.0, 10.0, 0.95),
                line("Inventory", 10.0, 40.0, 0.99),  // no Hangul
                line("HP 87/100", 10.0, 70.0, 0.99),  // no Hangul
                line("확인", 10.0, 100.0, 0.30),       // too uncertain
                line("  저장   하기 ", 10.0, 130.0, 0.9), // whitespace is normalised
                line("인벤토리", 12.0, 12.0, 0.95),    // the detector's duplicate
                OcrLine { text: "작".into(), rect: Rect::new(0.0, 0.0, 3.0, 3.0), words: vec![], confidence: 0.9 }, // a speck
            ],
        };
        let found = found_items(&out);
        assert_eq!(found.iter().map(|f| f.text.as_str()).collect::<Vec<_>>(), ["인벤토리", "저장 하기"]);
    }

    #[test]
    fn the_same_text_in_two_places_is_two_items_but_one_question() {
        let out = OcrOutput { lines: vec![line("확인", 10.0, 10.0, 0.9), line("확인", 400.0, 300.0, 0.9)] };
        let found = found_items(&out);
        assert_eq!(found.len(), 2, "both buttons need a label");
        let items: Vec<ScanItem> = found
            .iter()
            .enumerate()
            .map(|(i, f)| ScanItem { item: i as u32, rect: f.rect, text: f.text.clone(), label: String::new(), source: LabelSource::Pending, known: false })
            .collect();
        let groups = needs_model(&items);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].2, vec![0, 1]);
    }

    #[test]
    fn already_labelled_and_known_items_are_not_sent_to_the_model() {
        let item = |n: u32, text: &str, source, known| ScanItem { item: n, rect: Rect::default(), text: text.into(), label: String::new(), source, known };
        let items = vec![
            item(0, "취소", LabelSource::Cache, false),
            item(1, "설정", LabelSource::Glossary, false),
            item(2, "학교", LabelSource::Dictionary, true),
            item(3, "상점", LabelSource::Dictionary, false),
            item(4, "ㅁㄴ글자", LabelSource::Pending, false),
        ];
        let groups = needs_model(&items);
        assert_eq!(groups.iter().map(|g| g.1.as_str()).collect::<Vec<_>>(), ["상점", "ㅁㄴ글자"]);
    }

    #[test]
    fn long_lines_are_clipped_for_the_prompt() {
        let long = "가".repeat(300);
        let items = vec![ScanItem { item: 0, rect: Rect::default(), text: long, label: String::new(), source: LabelSource::Pending, known: false }];
        assert_eq!(needs_model(&items)[0].1.chars().count(), MAX_PROMPT_CHARS);
    }
}
