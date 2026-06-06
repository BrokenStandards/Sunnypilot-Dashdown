//! Foreign callback traits, re-exported in one place for a tidy FFI namespace.
//! The `#[uniffi::export(rust, foreign)]` registration lives at each trait's
//! definition site (`ProgressSink` in the sync engine, `LogSink` in logging);
//! these re-exports are purely organizational.

pub use crate::logging::LogSink;
pub use crate::sync_engine::ProgressSink;
