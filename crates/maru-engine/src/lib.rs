//! Orchestration: capture → analyse → translate → learn, streamed as events.
//!
//! * [`engine`] — the public object and its operations
//! * [`pipeline`] — the request pipelines (translation, hover lookup, deep dive)
//! * [`capture`] — selection / region / hover capture with fallbacks
//! * [`continuous`] — watch a region and report when its text changes
//! * [`annotate`], [`cache`], [`memory`], [`change`] — the supporting pieces

pub mod anki_flow;
pub mod annotate;
pub mod bootstrap;
pub mod cache;
pub mod capture;
pub mod change;
pub mod continuous;
pub mod engine;
pub mod memory;
mod pipeline;
mod live_lane;
mod live_text;
mod scan;
pub mod respace;
pub mod vision;
pub mod hardware;
pub mod hybrid_ocr;
pub mod joins;
pub mod plan;
pub mod labels;
pub mod vlm_ocr;

pub use bootstrap::{BootOptions, Services, boot, load_config};
pub use capture::{CaptureError, Capturer};
pub use continuous::{ContinuousHandle, LiveEvent, LiveLine};
pub use engine::{DeepDiveRequest, Engine, EngineError, EngineParts, EventRx, ReviewResult};
pub use memory::HistoryItem;
