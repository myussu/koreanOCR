//! Shared vocabulary for every Maru crate.
//!
//! Nothing in here does I/O heavier than reading a config file, so every other
//! crate (and the UI shell) can depend on it freely.

pub mod config;
pub mod error;
pub mod event;
pub mod paths;
pub mod settings;
pub mod types;

pub use config::Config;
pub use error::{Error, Result};
pub use event::{Event, Side, Stage, Variant};
pub use paths::Paths;
pub use types::*;
