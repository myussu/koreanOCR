//! Spoken sentences kept on disk: one file per sentence, named by a hash of everything that
//! changes how it sounds (voice, language, style, server, text). The oldest files go when the
//! folder grows past its limit. Hearing the same line twice, or replaying a paragraph, is free.

use std::path::PathBuf;

use crate::client::Audio;

pub struct AudioCache {
    dir: PathBuf,
    limit: u64,
}

/// A 128-bit hash for file names: two independent FNV-1a passes. Not cryptographic, and a
/// collision would only replay the wrong sentence.
pub fn key_of(parts: &[&str]) -> String {
    let (mut a, mut b) = (0xcbf29ce484222325u64, 0x84222325cbf29ce4u64);
    for part in parts {
        for byte in part.bytes().chain(std::iter::once(0x1f)) {
            a = (a ^ u64::from(byte)).wrapping_mul(0x100000001b3);
            b = (b ^ u64::from(byte)).wrapping_mul(0x100000001b3).rotate_left(5);
        }
    }
    format!("{a:016x}{b:016x}")
}

impl AudioCache {
    pub fn new(dir: PathBuf, limit: u64) -> Self {
        Self { dir, limit }
    }

    fn path(&self, key: &str, mime: &str) -> PathBuf {
        self.dir.join(format!("{key}.{}", extension(mime)))
    }

    pub fn get(&self, key: &str) -> Option<Audio> {
        if self.limit == 0 {
            return None;
        }
        for mime in ["audio/wav", "audio/mpeg", "audio/ogg", "audio/flac"] {
            let path = self.path(key, mime);
            if let Ok(bytes) = std::fs::read(&path)
                && !bytes.is_empty()
            {
                // touch it so it counts as recently used
                let _ = std::fs::File::options().append(true).open(&path).and_then(|f| f.set_modified(std::time::SystemTime::now()));
                return Some(Audio { bytes, mime: mime.to_string() });
            }
        }
        None
    }

    pub fn put(&self, key: &str, audio: &Audio) {
        if self.limit == 0 || audio.bytes.is_empty() || std::fs::create_dir_all(&self.dir).is_err() {
            return;
        }
        let path = self.path(key, &audio.mime);
        let tmp = path.with_extension("part");
        if std::fs::write(&tmp, &audio.bytes).and_then(|()| std::fs::rename(&tmp, &path)).is_err() {
            let _ = std::fs::remove_file(&tmp);
            return;
        }
        self.trim();
    }

    /// Delete the least recently used files until the folder fits.
    fn trim(&self) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return };
        let mut files: Vec<_> = entries
            .filter_map(Result::ok)
            .filter_map(|e| {
                let m = e.metadata().ok()?;
                m.is_file().then(|| (m.modified().unwrap_or(std::time::UNIX_EPOCH), m.len(), e.path()))
            })
            .collect();
        let mut total: u64 = files.iter().map(|f| f.1).sum();
        if total <= self.limit {
            return;
        }
        files.sort();
        for (_, size, path) in files {
            if total <= self.limit {
                break;
            }
            if std::fs::remove_file(path).is_ok() {
                total -= size;
            }
        }
    }
}

fn extension(mime: &str) -> &'static str {
    match mime {
        "audio/mpeg" => "mp3",
        "audio/ogg" => "ogg",
        "audio/flac" => "flac",
        _ => "wav",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audio(n: usize) -> Audio {
        Audio { bytes: vec![1; n], mime: "audio/wav".into() }
    }

    #[test]
    fn keys_depend_on_every_part_and_their_boundaries() {
        assert_eq!(key_of(&["a", "b"]), key_of(&["a", "b"]));
        assert_ne!(key_of(&["a", "b"]), key_of(&["ab", ""]));
        assert_ne!(key_of(&["voice", "안녕"]), key_of(&["voice2", "안녕"]));
        assert_eq!(key_of(&["x"]).len(), 32);
    }

    #[test]
    fn stores_and_returns_what_it_was_given() {
        let dir = tempfile::tempdir().unwrap();
        let cache = AudioCache::new(dir.path().join("c"), 1_000_000);
        assert!(cache.get("k").is_none());
        cache.put("k", &audio(100));
        assert_eq!(cache.get("k").unwrap().bytes.len(), 100);
        let mp3 = Audio { bytes: vec![9; 10], mime: "audio/mpeg".into() };
        cache.put("m", &mp3);
        assert_eq!(cache.get("m").unwrap().mime, "audio/mpeg");
    }

    #[test]
    fn the_least_recently_used_files_go_first() {
        let dir = tempfile::tempdir().unwrap();
        let cache = AudioCache::new(dir.path().to_path_buf(), 250);
        cache.put("a", &audio(100));
        std::thread::sleep(std::time::Duration::from_millis(30));
        cache.put("b", &audio(100));
        std::thread::sleep(std::time::Duration::from_millis(30));
        assert!(cache.get("a").is_some(), "reading a refreshes it");
        std::thread::sleep(std::time::Duration::from_millis(30));
        cache.put("c", &audio(100)); // 300 > 250: the oldest untouched file (b) goes
        assert!(cache.get("b").is_none());
        assert!(cache.get("a").is_some() && cache.get("c").is_some());
    }

    #[test]
    fn a_zero_limit_turns_the_cache_off() {
        let dir = tempfile::tempdir().unwrap();
        let cache = AudioCache::new(dir.path().to_path_buf(), 0);
        cache.put("a", &audio(10));
        assert!(cache.get("a").is_none());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
