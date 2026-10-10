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
    /// The second value says whether it came from the cache.
    pub async fn speak(&self, text: &str) -> Result<(Audio, bool)> {
        let key = self.client.cache_key(text);
        if let Some(hit) = self.cache.get(&key) {
            return Ok((hit, true));
        }
        let audio = self.client.speak(text).await?;
        self.cache.put(&key, &audio);
        Ok((audio, false))
    }
}
