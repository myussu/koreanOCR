//! Everything that talks to the local language model.
//!
//! * [`client`] — streaming OpenAI-compatible chat client (llama-server speaks it)
//! * [`server`] — spawns and supervises `llama-server` with the right flags
//! * [`prompts`] — the instructions, kept in one place so they can be tuned
//! * [`demux`] — splits one token stream into labelled sections as it arrives
//! * [`tasks`] — translate / gloss / deep-dive operations built from the above

pub mod client;
pub mod demux;
pub mod prompts;
pub mod server;
pub mod tasks;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("cannot reach the language model at {url}: {source}")]
    Unreachable { url: String, source: reqwest::Error },
    #[error("language model returned HTTP {status}: {body}")]
    Http { status: u16, body: String },
    #[error("language model error: {0}")]
    Api(String),
    #[error("language model stopped responding for {0}s")]
    Stalled(u64),
    #[error("could not start llama-server: {0}")]
    Spawn(String),
    #[error("llama-server did not become ready: {0}")]
    NotReady(String),
    #[error("stream error: {0}")]
    Stream(String),
}

pub type Result<T, E = LlmError> = std::result::Result<T, E>;

pub use client::{ChatOpts, LlmClient, Message};
#[cfg(any(test, feature = "testing"))]
pub use client::testing;
pub use prompts::PromptContext;
pub use server::{LlamaServer, build_args};
pub use tasks::{DeepRequest, EnKoRequest, GlossLine, GlossRequest, GlossWord, KoEnRequest, LlmChunk, parse_gloss_line};
