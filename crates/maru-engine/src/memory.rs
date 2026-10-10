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
}

impl HistoryItem {
    pub fn new(id: u64, at: i64, source: String, translation: String, direction: Direction, app: Option<String>) -> Self {
        Self { id, at, source, translation, direction, app, live: false, first_ms: None, total_ms: None }
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
