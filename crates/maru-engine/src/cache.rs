//! Repeated lines (subtitles that recur, a sentence selected twice) come back instantly.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;

use lru::LruCache;
use maru_core::event::{Side, Variant};
use maru_core::{Direction, Gloss, Register};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    text: String,
    direction: Direction,
    register: Register,
    /// Profile name + glossary: a different glossary changes the translation.
    profile: u64,
}

impl CacheKey {
    pub fn new(text: &str, direction: Direction, register: Register, profile: &impl Hash) -> Self {
        let mut h = DefaultHasher::new();
        profile.hash(&mut h);
        Self { text: text.to_string(), direction, register, profile: h.finish() }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Cached {
    pub translations: Vec<(Variant, String)>,
    pub glosses: Vec<(Side, Gloss)>,
    pub note: Option<String>,
    pub translation_complete: bool,
    pub glosses_complete: bool,
}

impl Cached {
    /// Partial text remains visible while streaming, but only a finished answer is reusable.
    pub fn is_useful(&self) -> bool {
        self.translation_complete && self.translations.iter().any(|(_, t)| !t.trim().is_empty())
    }
}

pub struct Cache(LruCache<CacheKey, Cached>);

impl Cache {
    pub fn new(capacity: usize) -> Self {
        Self(LruCache::new(NonZeroUsize::new(capacity.max(1)).expect("capacity is at least 1")))
    }
    pub fn get(&mut self, key: &CacheKey) -> Option<Cached> {
        self.0.get(key).cloned()
    }
    pub fn put(&mut self, key: CacheKey, value: Cached) {
        if value.is_useful() {
            self.0.put(key, value);
        }
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(t: &str, profile: &str) -> CacheKey {
        CacheKey::new(t, Direction::KoToEn, Register::Polite, &profile)
    }

    fn value(t: &str) -> Cached {
        Cached { translations: vec![(Variant::Primary, t.into())], translation_complete: true, ..Default::default() }
    }

    #[test]
    fn hit_miss_and_profile_separation() {
        let mut c = Cache::new(8);
        c.put(key("안녕", "game"), value("Hi"));
        assert_eq!(c.get(&key("안녕", "game")).unwrap().translations[0].1, "Hi");
        assert!(c.get(&key("안녕", "chat")).is_none(), "different profile/glossary must not share results");
        assert!(c.get(&key("안녕하세요", "game")).is_none());
    }

    #[test]
    fn empty_results_are_not_cached() {
        let mut c = Cache::new(8);
        c.put(key("a", ""), Cached::default());
        c.put(key("b", ""), value("  "));
        assert!(c.is_empty());
    }

    #[test]
    fn a_variant_closed_before_the_response_finishes_is_not_a_cache_hit() {
        let mut c = Cache::new(8);
        let mut partial = value("Partial answer");
        partial.translation_complete = false;
        c.put(key("안녕", ""), partial);
        assert!(c.is_empty());
    }

    #[test]
    fn least_recently_used_is_evicted() {
        let mut c = Cache::new(2);
        c.put(key("a", ""), value("A"));
        c.put(key("b", ""), value("B"));
        c.get(&key("a", ""));
        c.put(key("c", ""), value("C"));
        assert!(c.get(&key("b", "")).is_none());
        assert!(c.get(&key("a", "")).is_some() && c.get(&key("c", "")).is_some());
        assert_eq!(c.len(), 2);
    }
}
