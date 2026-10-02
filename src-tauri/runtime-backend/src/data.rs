//! Backend wiring for the canonical runtime data-command dispatcher.
//!
//! The dispatcher itself is the single canonical module at
//! `src-tauri/src/runtime_data.rs`, shared with the desktop crate through
//! `#[path]` so there is exactly one implementation of the data-command cores.
//! This crate exposes it at the crate root (`crate::runtime_data`) so the shared
//! `/control/v1` control-plane server can mount the handler with the same path
//! in both crates.

#[path = "../../src/runtime_data.rs"]
pub mod runtime_data;
