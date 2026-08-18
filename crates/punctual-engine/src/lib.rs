//! Background browser control and task scheduling for Punctual.
//!
//! GPUI remains single-threaded and only exchanges typed commands/events with
//! this crate. Chromium, timing and SQLite state transitions run on a dedicated
//! Tokio runtime owned by [`EngineHandle`].

mod browser_hub;
mod engine;
mod handle;
mod worker;

pub use handle::{EngineConfig, EngineHandle};
