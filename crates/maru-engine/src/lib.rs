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
mod deferred_models;
pub mod cache;
pub mod capture;
pub mod change;
pub mod continuous;
pub mod engine;
pub mod memory;
mod pipeline;
mod live_lane;
mod live_context;
pub mod live_models;
mod live_text;
mod resource_targets;
pub mod model_controls;
pub mod ocr_corrections;
#[cfg(test)]
mod model_control_tests;
#[cfg(test)]
mod remote_model_tests;
pub mod resource_monitor;
pub mod server_discovery;
mod scan;
mod scan_joining;
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
pub use maru_llm::connection::{ConnectionReply, test_connection};
