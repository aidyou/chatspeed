//! Backend wiring for the canonical runtime data-command dispatcher.
//!
//! The dispatcher itself is the single canonical module owned by this crate
//! (`src/data/runtime_data.rs`). The desktop links this crate and keeps only
//! transport adapters, so there is one implementation of the data-command
//! cores. This crate exposes it at the crate root (`crate::runtime_data`) for
//! the `/control/v1` control-plane server.

pub mod runtime_data;
