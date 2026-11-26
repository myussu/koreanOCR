//! Text to speech for studying: hear Korean read the way a person says it.
//!
//! * [`client`] — talks to anything that serves `POST /v1/audio/speech` (the local `tts-server`,
//!   a PC on your network, a paid service)
//! * [`local`] — starts and supervises the local `tts-server` (qwentts.cpp running Qwen3-TTS)
//! * [`split`] — cuts a long text into sentences that read well one at a time
//! * [`cache`] — keeps spoken sentences on disk so hearing one again costs nothing
//! * [`wav`] — reads just enough of a WAV file to know how long it is
//!
//! [`Tts`] ties them together: split, look in the cache, ask the provider, remember.

pub mod cache;
pub mod client;
pub mod local;
pub mod split;
pub mod wav;

use std::sync::Arc;

use maru_core::config::{TtsConfig, TtsProvider};
use maru_core::Paths;
use thiserror::Error;

pub use client::{Audio, TtsClient};

#[derive(Debug, Error)]
pub enum TtsError {
    #[error("cannot reach the speech server at {url}: {detail}")]
    Unreachable { url: String, detail: String },
    #[error("the speech server answered HTTP {status}: {body}")]
    Http { status: u16, body: String },
    #[error("the speech server sent {0}")]
    BadAudio(String),
    #[error("speech is not set up: {0}")]
    NotConfigured(String),
    #[error("could not start the speech server: {0}")]
    Spawn(String),
}

pub type Result<T> = std::result::Result<T, TtsError>;

/// Takes a speech model may make before one of a plausible length is kept. Models like Qwen3-TTS now and then keep
/// talking, or babble, long after a short text (a word of 0.6 s came back as 22 s); asking again usually comes out right.
const TAKES: usize = 3;

/// The longest a take of `text` may plausibly last: a second, plus 0.45 s for every Hangul syllable or other word,
/// slower at a lower speed. Natural Korean speech measured 0.3–0.4 s a syllable.
pub fn longest_plausible(text: &str, speed: f32) -> f32 {
    let syllables = text.chars().filter(|c| ('\u{AC00}'..='\u{D7A3}').contains(c)).count();
    let words = text.split(|c: char| !c.is_alphanumeric() || ('\u{AC00}'..='\u{D7A3}').contains(&c)).filter(|w| !w.is_empty()).count();
    (1.0 + 0.45 * (syllables + words) as f32) / speed.max(0.25)
}

/// Is this take short enough for its text? Audio that is not WAV cannot be measured and is taken as it is.
fn plausible(audio: &Audio, limit: f32) -> bool {
    wav::info(&audio.bytes).is_none_or(|w| w.seconds <= limit)
}

/// Speech for the app: one provider, one cache.
#[derive(Clone)]
pub struct Tts {
    client: Arc<TtsClient>,
    cache: Arc<cache::AudioCache>,
}

impl Tts {
    /// `local_port` is where the local server listens; for the `Endpoint` provider the configured
    /// address is used instead.
    pub fn new(cfg: &TtsConfig, paths: &Paths) -> Result<Self> {
        let base = match cfg.provider {
            TtsProvider::Local => format!("http://127.0.0.1:{}", cfg.port),
            TtsProvider::Endpoint => cfg
                .endpoint
                .clone()
                .filter(|e| !e.trim().is_empty())
                .ok_or_else(|| TtsError::NotConfigured("no server address: set [tts] endpoint".into()))?,
        };
        let mut cfg = cfg.clone();
        // The local server knows which language the model should speak only if told.
        if cfg.provider == TtsProvider::Local && cfg.language.trim().is_empty() {
            cfg.language = "Korean".into();
        }
        Ok(Self { client: Arc::new(TtsClient::new(&base, &cfg)), cache: Arc::new(cache::AudioCache::new(paths.tts_cache_dir(), u64::from(cfg.cache_mb) * 1_048_576)) })
    }

    pub fn client(&self) -> &TtsClient {
        &self.client
    }

    /// The sentences of `text`, as they will be spoken.
    pub fn sentences(&self, text: &str) -> Vec<String> {
        split::sentences(text)
    }

    /// Speech for one sentence (or any short text): from the cache if it was spoken before.
    /// The second value says whether it came from the cache. A take far longer than the text could need is asked for
    /// again (up to [`TAKES`] in all, keeping the shortest); only a plausible one is cached.
    pub async fn speak(&self, text: &str) -> Result<(Audio, bool)> {
        let key = self.client.cache_key(text);
        let limit = longest_plausible(text, self.client.speed());
        if let Some(hit) = self.cache.get(&key) {
            // An overlong take cached before this check is made again.
            if plausible(&hit, limit) {
                return Ok((hit, true));
            }
        }
        let mut shortest: Option<(Audio, f32)> = None;
        for take in 1..=TAKES {
            let audio = match self.client.speak(text).await {
                Ok(audio) => audio,
                Err(_) if shortest.is_some() => break,
                Err(e) => return Err(e),
            };
            if plausible(&audio, limit) {
                self.cache.put(&key, &audio);
                return Ok((audio, false));
            }
            let seconds = wav::info(&audio.bytes).map_or(f32::MAX, |w| w.seconds);
            tracing::warn!(take, seconds, limit, "speech much longer than its text; asking again");
            if shortest.as_ref().is_none_or(|(_, s)| seconds < *s) {
                shortest = Some((audio, seconds));
            }
        }
        // Not cached: the next request tries again.
        Ok((shortest.map(|(audio, _)| audio).expect("at least one take"), false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// A speech server on a free port that answers each request with a WAV of the next length in `seconds`.
    fn server(seconds: Vec<f32>) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let served = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = served.clone();
        std::thread::spawn(move || {
            for secs in seconds {
                let Ok((mut s, _)) = listener.accept() else { return };
                let mut buf = vec![0u8; 65536];
                let _ = s.read(&mut buf);
                // Counted before the answer goes out: the caller may look at the count as soon as it has the audio.
                count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let wav = wav::encode_pcm16(&vec![0i16; (secs * 24_000.0) as usize], 24_000);
                let head = format!("HTTP/1.1 200 OK\r\ncontent-type: audio/wav\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", wav.len());
                let _ = s.write_all(head.as_bytes());
                let _ = s.write_all(&wav);
            }
        });
        (url, served)
    }

    fn tts(url: &str, dir: &std::path::Path) -> Tts {
        let cfg = TtsConfig { provider: maru_core::config::TtsProvider::Endpoint, endpoint: Some(url.into()), ..Default::default() };
        Tts::new(&cfg, &Paths::under(dir)).unwrap()
    }

    #[test]
    fn a_word_may_last_about_a_second_and_a_sentence_a_few() {
        assert!((longest_plausible("이", 1.0) - 1.45).abs() < 1e-3, "one syllable; normal takes measured 0.5–1.0 s");
        assert!(longest_plausible("도둑", 1.0) < 3.0);
        let line = "원래 우리 왕자님은 운이 억수로 나쁘거든.";
        assert!(longest_plausible(line, 1.0) > 7.4, "a measured natural reading of a longer line fits");
        assert!(longest_plausible("이", 0.5) > longest_plausible("이", 1.0), "slower speech may take longer");
    }

    #[tokio::test]
    async fn an_overlong_take_is_asked_for_again_and_only_a_plausible_one_is_cached() {
        let dir = tempfile::tempdir().unwrap();
        let (url, served) = server(vec![22.0, 0.6]);
        let tts = tts(&url, dir.path());
        let (audio, cached) = tts.speak("도둑").await.unwrap();
        assert!(!cached);
        assert!((wav::info(&audio.bytes).unwrap().seconds - 0.6).abs() < 0.01, "the second take");
        assert_eq!(served.load(std::sync::atomic::Ordering::SeqCst), 2);
        let (again, cached) = tts.speak("도둑").await.unwrap();
        assert!(cached && again.bytes == audio.bytes, "the plausible take was cached");
    }

    #[tokio::test]
    async fn when_every_take_is_too_long_the_shortest_is_used_and_not_cached() {
        let dir = tempfile::tempdir().unwrap();
        let (url, served) = server(vec![14.0, 6.0, 9.0, 0.5]);
        let tts = tts(&url, dir.path());
        let (audio, _) = tts.speak("이").await.unwrap();
        assert!((wav::info(&audio.bytes).unwrap().seconds - 6.0).abs() < 0.01);
        assert_eq!(served.load(std::sync::atomic::Ordering::SeqCst), 3);
        let (later, cached) = tts.speak("이").await.unwrap();
        assert!(!cached && (wav::info(&later.bytes).unwrap().seconds - 0.5).abs() < 0.01, "asked again next time");
    }
}
