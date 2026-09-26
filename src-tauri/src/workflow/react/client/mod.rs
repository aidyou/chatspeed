//! Transport adapters for the workflow runtime.
//!
//! Phase 1 keeps the Tauri event transport here. Later units add the shared
//! runtime hub (`hub`) and the loopback HTTP control plane (`http`) next to it.

pub mod http;
pub mod hub;
pub mod tauri;
