//! Reversible screen-label joins, including joins loaded from earlier sessions.
use std::collections::HashSet;
use std::time::Instant;

use maru_core::{Capture, CaptureSource, Event, LabelSource, ScanItem, ScanJoin, ScanPart};
use rusqlite::{Connection, OptionalExtension, params};

use crate::EventRx;
use crate::engine::{Engine, EngineError, Result};
use crate::joins::{JoinRule, follows, reading_order};
use crate::labels::{LabelCache, key};
use crate::ocr_corrections::with_scan;
use crate::pipeline::Emitter;
use crate::scan::{Found, LabelInput};

#[derive(Clone)]
pub(crate) struct JoinedFound {
    pub(crate) found: Found,
    pub(crate) join: Option<ScanJoin>,
    pub(crate) observed: Option<String>,
    aliases: Vec<String>,
}

impl JoinedFound {
    pub(crate) fn new(found: Found, join: Option<ScanJoin>) -> Self {
        Self {
            aliases: vec![key(&found.text)],
            found,
            join,
            observed: None,
        }
    }
}

pub(crate) fn init_store(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS join_spacing (
        scope TEXT NOT NULL, parts TEXT NOT NULL, as_word INTEGER NOT NULL,
        PRIMARY KEY(scope, parts));",
    )
}

fn rule_key(parts: &[String]) -> String {
    parts
        .iter()
        .map(|p| key(p))
        .collect::<Vec<_>>()
        .join("\u{1f}")
}

impl LabelCache {
    fn join_as_word(&self, scope: &str, parts: &[String]) -> bool {
        self.conn()
            .query_row(
                "SELECT as_word FROM join_spacing WHERE scope=?1 AND parts=?2",
                params![scope, rule_key(parts)],
                |r| r.get::<_, bool>(0),
            )
            .optional()
            .ok()
            .flatten()
            .unwrap_or(false)
    }

    fn remember_join(
        &self,
        scope: &str,
        parts: &[String],
        joined: &str,
        as_word: bool,
    ) -> rusqlite::Result<()> {
        if scope.is_empty() {
            return Ok(());
        }
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let parts = rule_key(parts);
        tx.execute(
            "INSERT INTO joins(scope,parts,joined,created) VALUES(?1,?2,?3,?4)
            ON CONFLICT(scope,parts) DO UPDATE SET joined=excluded.joined",
            params![scope, parts, joined, maru_srs::now_secs()],
        )?;
        tx.execute(
            "INSERT INTO join_spacing(scope,parts,as_word) VALUES(?1,?2,?3)
            ON CONFLICT(scope,parts) DO UPDATE SET as_word=excluded.as_word",
            params![scope, parts, as_word],
        )?;
        tx.commit()
    }

    fn forget_rules(&self, scope: &str, rules: &[Vec<String>]) -> rusqlite::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for parts in rules {
            let parts = rule_key(parts);
            tx.execute(
                "DELETE FROM joins WHERE scope=?1 AND parts=?2",
                params![scope, parts],
            )?;
            tx.execute(
                "DELETE FROM join_spacing WHERE scope=?1 AND parts=?2",
                params![scope, parts],
            )?;
        }
        tx.commit()
    }
}

fn combine(pieces: &[JoinedFound], as_word: bool) -> Option<JoinedFound> {
    let ordered = reading_order(pieces.iter().map(|p| p.found.clone()).collect());
    let first = ordered.first()?;
    let rect = ordered.iter().fold(first.rect, |r, p| r.union(&p.rect));
    let text = ordered
        .iter()
        .map(|p| p.text.trim())
        .collect::<Vec<_>>()
        .join(if as_word { "" } else { " " });
    let mut parts = Vec::new();
    let mut rules = Vec::new();
    for found in &ordered {
        let piece = pieces.iter().find(|p| p.found == *found)?;
        if let Some(join) = &piece.join {
            parts.extend(join.parts.clone());
            rules.extend(join.rules.clone());
        } else {
            parts.push(ScanPart {
                text: found.text.clone(),
                rect: found.rect,
                observed: None,
            });
        }
    }
    Some(JoinedFound::new(
        Found { rect, text },
        Some(ScanJoin {
            parts,
            rules,
            as_word,
        }),
    ))
}

/// Legacy saved text remains in the database. Rebuild visible words from the detected pieces;
/// aliases let a nested legacy rule still match its formerly collapsed intermediate text.
pub(crate) fn apply_saved(
    found: Vec<Found>,
    rules: &[JoinRule],
    labels: Option<&LabelCache>,
    scope: &str,
) -> Vec<JoinedFound> {
    let mut found: Vec<_> = found
        .into_iter()
        .map(|f| JoinedFound::new(f, None))
        .collect();
    for rule in rules {
        let Some((first, rest)) = rule.parts.split_first() else {
            continue;
        };
        let mut start = 0;
        while start < found.len() {
            if !found[start].aliases.contains(first) {
                start += 1;
                continue;
            }
            let mut chain = vec![start];
            for want in rest {
                let last = found[*chain.last().unwrap_or(&start)].found.rect;
                let next = found
                    .iter()
                    .enumerate()
                    .filter(|(i, f)| {
                        !chain.contains(i)
                            && f.aliases.contains(want)
                            && follows(&last, &f.found.rect)
                    })
                    .min_by(|a, b| {
                        (a.1.found.rect.x - last.right())
                            .abs()
                            .total_cmp(&(b.1.found.rect.x - last.right()).abs())
                            .then(
                                (a.1.found.rect.y - last.bottom())
                                    .abs()
                                    .total_cmp(&(b.1.found.rect.y - last.bottom()).abs()),
                            )
                    })
                    .map(|(i, _)| i);
                if let Some(next) = next {
                    chain.push(next);
                } else {
                    break;
                }
            }
            if chain.len() != rule.parts.len() {
                start += 1;
                continue;
            }
            let pieces: Vec<_> = chain.iter().map(|i| found[*i].clone()).collect();
            let as_word = labels.is_some_and(|c| c.join_as_word(scope, &rule.parts));
            let Some(mut joined) = combine(&pieces, as_word) else {
                start += 1;
                continue;
            };
            if let Some(join) = &mut joined.join {
                join.rules.push(rule.parts.clone());
            }
            joined.aliases.push(key(&rule.joined));
            joined.observed = Some(rule.joined.clone());
            found[start] = joined;
            let moved = chain[1..].iter().filter(|i| **i < start).count();
            let mut gone = chain[1..].to_vec();
            gone.sort_unstable_by(|a, b| b.cmp(a));
            for i in gone {
                found.remove(i);
            }
            start = start.saturating_sub(moved) + 1;
        }
    }
    found
}

fn pending_item(item: u32, found: &JoinedFound) -> ScanItem {
    ScanItem {
        item,
        rect: found.found.rect,
        text: found.found.text.clone(),
        label: String::new(),
        source: LabelSource::Pending,
        known: false,
        join: found.join.clone(),
    }
}

fn message(error: impl std::fmt::Display) -> EngineError {
    EngineError::Other(error.to_string())
}

impl Engine {
    /// Join current item IDs atomically with correction checks, preserving spaces unless explicitly joining a word.
    pub async fn join_scan_items(
        &self,
        id: u64,
        item: u32,
        selected: Vec<u32>,
        as_word: bool,
    ) -> Result<EventRx> {
        if selected.len() < 2 || selected.iter().collect::<HashSet<_>>().len() != selected.len() {
            return Err(message("Select at least two different labels to join."));
        }
        let inner = self.inner.clone();
        let (joined, scope) = tokio::task::spawn_blocking(move || with_scan(&inner, id, |scan| {
            if item < scan.next_item { return Err(message("The label numbers changed. Wait for the current labels and try joining again.")); }
            let next = item.checked_add(1).ok_or_else(|| message("Too many labels in this scan; scan again."))?;
            let pieces = selected.iter().map(|id| scan.items.get(id).cloned()
                .ok_or_else(|| message("One selected label changed. Select the current labels again."))).collect::<Result<Vec<_>>>()?;
            let inputs: Vec<_> = pieces.iter().map(|p| JoinedFound::new(Found { rect:p.rect, text:p.text.clone() }, p.join.clone())).collect();
            let mut joined = combine(&inputs, as_word).ok_or_else(|| message("There is no text to join."))?;
            if joined.found.text.chars().count() > 4096 { return Err(message("Join fewer labels; a sentence can contain at most 4,096 characters.")); }
            let ordered = reading_order(inputs.iter().map(|p| p.found.clone()).collect());
            let rule: Vec<_> = ordered.iter().map(|p| key(&p.text)).collect();
            if let Some(cache) = &inner.labels { cache.remember_join(&scan.scope, &rule, &joined.found.text, as_word).map_err(message)?; }
            if let Some(join) = &mut joined.join {
                for part in &mut join.parts {
                    if let Some(piece) = pieces.iter().find(|p| p.rect==part.rect && p.text==part.text)
                        && let Some(raw) = scan.originals.get(&piece.item).filter(|raw| **raw != part.text) {
                        part.observed = Some(raw.clone());
                    }
                }
                join.rules.push(rule);
            }
            for old in selected { scan.items.remove(&old); scan.originals.remove(&old); }
            scan.next_item = next;
            scan.items.insert(item, pending_item(item,&joined));
            scan.originals.insert(item, joined.found.text.clone());
            Ok((joined,scan.scope.clone()))
        })).await.map_err(message)??;
        Ok(self.label_joined(id, item, vec![joined], scope, false, None))
    }

    /// Remove only this join's rules and restore its original boxes with fresh IDs, without recapturing the game.
    pub async fn separate_scan_item(&self, id: u64, item: u32) -> Result<EventRx> {
        let inner = self.inner.clone();
        let (found, first, scope) = tokio::task::spawn_blocking(move || {
            with_scan(&inner, id, |scan| {
                let joined = scan
                    .items
                    .get(&item)
                    .and_then(|i| i.join.clone())
                    .filter(|j| j.parts.len() >= 2)
                    .ok_or_else(|| {
                        message(
                            "That joined label is no longer current. Use the current label's menu.",
                        )
                    })?;
                let first = scan.next_item;
                let count = u32::try_from(joined.parts.len()).map_err(message)?;
                let next = first
                    .checked_add(count)
                    .ok_or_else(|| message("Too many labels in this scan; scan again."))?;
                if let Some(cache) = &inner.labels {
                    cache
                        .forget_rules(&scan.scope, &joined.rules)
                        .map_err(message)?;
                }
                scan.items.remove(&item);
                scan.originals.remove(&item);
                scan.next_item = next;
                let mut found = Vec::new();
                for (offset, part) in joined.parts.into_iter().enumerate() {
                    let number = first + offset as u32;
                    let piece = JoinedFound::new(
                        Found {
                            rect: part.rect,
                            text: part.text.clone(),
                        },
                        None,
                    );
                    scan.originals
                        .insert(number, part.observed.unwrap_or(part.text));
                    scan.items.insert(number, pending_item(number, &piece));
                    found.push(piece);
                }
                Ok((found, first, scan.scope.clone()))
            })
        })
        .await
        .map_err(message)??;
        Ok(self.label_joined(id, first, found, scope, false, Some(item)))
    }

    fn label_joined(
        &self,
        id: u64,
        first: u32,
        items: Vec<JoinedFound>,
        scope: String,
        correct_ocr: bool,
        separated: Option<u32>,
    ) -> EventRx {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let inner = self.inner.clone();
        self.inner.runtime.spawn(async move {
            let em = Emitter::new(tx, id);
            if let Some(item) = separated {
                em.send(Event::ScanSeparated {
                    id,
                    item,
                    next_item: first + items.len() as u32,
                });
            }
            crate::scan::label(
                &inner,
                &em,
                LabelInput::Joined { items, correct_ocr },
                first,
                &scope,
                Instant::now(),
            )
            .await;
            em.done();
        });
        rx
    }

    /// A clicked label retains the game's capture scope and uses sentence analysis for a sentence join.
    pub fn capture_scan_item(&self, id: u64, item: u32) -> Result<(Capture, bool)> {
        let (data, app) = with_scan(&self.inner, id, |scan| {
            let data = scan
                .items
                .get(&item)
                .cloned()
                .ok_or_else(|| message("That label changed. Click its current version."))?;
            Ok((data, scan.app.clone()))
        })?;
        let sentence_join = data.join.as_ref().is_some_and(|j| !j.as_word);
        let single = !sentence_join
            && data.text.split_whitespace().count() == 1
            && self
                .analyze(&data.text)
                .iter()
                .filter(|e| e.content)
                .count()
                == 1;
        let capture = Capture {
            text: data.text,
            source: CaptureSource::Ocr,
            rect: Some(data.rect),
            words: Vec::new(),
            cursor: self.inner.platform.screen.cursor(),
            app,
            confidence: None,
        };
        Ok((capture, single))
    }
}
