//! Learning loop persistence: vocabulary, FSRS-6 scheduling, Anki export.

pub mod anki;
pub mod fsrs;
pub mod store;

use thiserror::Error;

pub use anki::{AnkiClient, build_card, export_tsv};
pub use fsrs::Rating;
pub use store::{Context, LookupInfo, ReviewOutcome, SrsConfig, Vocab, VocabStats, WordRecord, now_secs};

#[derive(Debug, Error)]
pub enum SrsError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("anki: {0}")]
    Anki(String),
    #[error("{0}")]
    Other(String),
}

pub type Result<T, E = SrsError> = std::result::Result<T, E>;
