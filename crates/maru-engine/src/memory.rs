//! Short-term context: what you were just reading (so pronouns and names stay
//! consistent), and a scrollable history of recent lines.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use maru_core::Direction;
use serde::Serialize;

/// Context older than this belongs to a different scene.
const TTL: Duration = Duration::from_secs(15 * 60);

struct Entry {
    at: Instant,
    source: String,
    translation: String,
}

pub struct Memory {
    per_app: HashMap<String, VecDeque<Entry>>,
    max_per_app: usize,
}

impl Memory {
    pub fn new(max_per_app: usize) -> Self {
        Self { per_app: HashMap::new(), max_per_app }
    }

    pub fn push(&mut self, app: &str, source: &str, translation: &str, now: Instant) {
        if self.max_per_app == 0 || source.trim().is_empty() || translation.trim().is_empty() {
            return;
        }
        let q = self.per_app.entry(app.to_string()).or_default();
        if q.back().is_some_and(|e| e.source == source) {
            return; // same line again
        }
        q.push_back(Entry { at: now, source: source.to_string(), translation: translation.to_string() });
        while q.len() > self.max_per_app {
            q.pop_front();
        }
    }

    /// Recent `(source, translation)` pairs for this app, oldest first.
    pub fn context(&self, app: &str, now: Instant) -> Vec<(String, String)> {
        self.per_app
            .get(app)
            .map(|q| {
                q.iter()
                    .filter(|e| now.duration_since(e.at) <= TTL)
                    .map(|e| (e.source.clone(), e.translation.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HistoryItem {
    pub id: u64,
    /// Unix seconds.
    pub at: i64,
    pub source: String,
    pub translation: String,
    pub direction: Direction,
    pub app: Option<String>,
    /// Read from live subtitles.
    pub live: bool,
    /// Live lines: from the line appearing on screen until its translation started / finished, in ms.
    pub first_ms: Option<u32>,
    pub total_ms: Option<u32>,
    /// Live lines: approximate screen appearance time in Unix seconds.
    pub appeared_at: Option<i64>,
    /// Live lines: the row in `history.db`, once saved.
    pub saved_id: Option<i64>,
    /// Live lines: marked to study later (`live_star.rs`).
    pub starred: bool,
    /// Live lines: Anki cards were made from it (`mining.rs`).
    pub mined: bool,
}

impl HistoryItem {
    pub fn new(id: u64, at: i64, source: String, translation: String, direction: Direction, app: Option<String>) -> Self {
        Self { id, at, source, translation, direction, app, live: false, first_ms: None, total_ms: None, appeared_at: None, saved_id: None, starred: false, mined: false }
    }
}

pub struct History {
    items: VecDeque<HistoryItem>,
    cap: usize,
}

impl History {
    pub fn new(cap: usize) -> Self {
        Self { items: VecDeque::new(), cap: cap.max(1) }
    }
    pub fn push(&mut self, item: HistoryItem) {
        // A live line that comes back right after it went away (a flicker) is the same line.
        if item.live
            && self.items.back().is_some_and(|last| last.live && last.source == item.source && item.at - last.at <= 3)
        {
            return;
        }
        self.items.push_back(item);
        while self.items.len() > self.cap {
            self.items.pop_front();
        }
    }
    /// Newest last.
    pub fn items(&self) -> Vec<HistoryItem> {
        self.items.iter().cloned().collect()
    }

    /// A fuller reading of a live line takes the place of the reading it grew from.
    pub fn replace(&mut self, id: u64, item: HistoryItem) {
        match self.items.iter_mut().rev().find(|old| old.live && old.id == id) {
            Some(old) => *old = item,
            None => self.push(item),
        }
    }

    pub(crate) fn has_live(&self, id: u64) -> bool {
        self.items.iter().any(|item| item.live && item.id == id && id != 0)
    }

    /// The newest live line of this session or, after a restart, of the saved history.
    pub(crate) fn newest_live_id(&self) -> Option<u64> {
        self.items.iter().rev().find(|item| item.live).map(|item| item.id)
    }

    pub(crate) fn is_starred(&self, id: u64) -> bool {
        self.items.iter().any(|item| item.live && item.id == id && item.starred)
    }

    /// Flip a live line's star; `None` if it is not in the history.
    pub(crate) fn toggle_star(&mut self, id: u64) -> Option<bool> {
        let item = self.items.iter_mut().rev().find(|item| item.live && item.id == id)?;
        item.starred = !item.starred;
        Some(item.starred)
    }

    pub(crate) fn set_mined_saved(&mut self, saved_id: i64) {
        for item in self.items.iter_mut().filter(|item| item.saved_id == Some(saved_id)) {
            item.mined = true;
        }
    }

    pub(crate) fn set_star_saved(&mut self, saved_id: i64, starred: bool) {
        for item in self.items.iter_mut().filter(|item| item.saved_id == Some(saved_id)) {
            item.starred = starred;
        }
    }

    pub(crate) fn saved_id_of(&self, id: u64) -> Option<i64> {
        self.items.iter().rev().find(|item| item.live && item.id == id).and_then(|item| item.saved_id)
    }

    /// Lines saved in an earlier run, shown before anything new.
    pub(crate) fn restore(&mut self, lines: Vec<HistoryItem>) {
        for line in lines.into_iter().rev() {
            if self.items.len() >= self.cap {
                break;
            }
            self.items.push_front(line);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_last_n_per_app_and_isolates_apps() {
        let now = Instant::now();
        let mut m = Memory::new(2);
        for i in 0..4 {
            m.push("game.exe", &format!("s{i}"), &format!("t{i}"), now);
        }
        m.push("chat.exe", "x", "y", now);
        let ctx = m.context("game.exe", now);
        assert_eq!(ctx, [("s2".to_string(), "t2".to_string()), ("s3".to_string(), "t3".to_string())]);
        assert_eq!(m.context("chat.exe", now).len(), 1);
        assert!(m.context("other.exe", now).is_empty());
    }

    #[test]
    fn old_context_expires_and_duplicates_collapse() {
        let t0 = Instant::now();
        let mut m = Memory::new(5);
        m.push("a", "s", "t", t0);
        m.push("a", "s", "t", t0);
        assert_eq!(m.context("a", t0).len(), 1);
        assert!(m.context("a", t0 + TTL + Duration::from_secs(1)).is_empty());
    }

    #[test]
    fn zero_capacity_disables_memory() {
        let mut m = Memory::new(0);
        m.push("a", "s", "t", Instant::now());
        assert!(m.context("a", Instant::now()).is_empty());
    }

    #[test]
    fn a_live_line_repeated_at_once_is_one_entry_but_later_it_is_a_new_one() {
        let live = |id, at, src: &str| HistoryItem { live: true, ..HistoryItem::new(id, at, src.into(), "t".into(), Direction::KoToEn, None) };
        let mut h = History::new(10);
        h.push(live(1, 100, "안녕"));
        h.push(live(2, 101, "안녕")); // flicker
        h.push(live(3, 120, "안녕")); // said again later
        h.push(HistoryItem::new(4, 121, "안녕".into(), "t".into(), Direction::KoToEn, None)); // typed by hand: always kept
        assert_eq!(h.items().iter().map(|i| i.id).collect::<Vec<_>>(), [1, 3, 4]);
    }

    #[test]
    fn history_is_bounded() {
        let mut h = History::new(2);
        for id in 0..5 {
            h.push(HistoryItem::new(id, 0, String::new(), String::new(), Direction::KoToEn, None));
        }
        assert_eq!(h.items().iter().map(|i| i.id).collect::<Vec<_>>(), [3, 4]);
    }
}
