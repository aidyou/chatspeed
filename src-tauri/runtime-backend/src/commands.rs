//! Runtime view of the canonical command core.
//!
//! The canonical `src/commands/mod.rs` also declares desktop adapters (window,
//! clipboard, updater, tray, ...) that the runtime must not link. The workflow
//! command file itself is transport-neutral core plus a `desktop`-gated set of
//! `#[tauri::command]` wrappers, so the runtime includes it directly instead of
//! copying any of it.

#[path = "../../src/commands/workflow.rs"]
pub mod workflow;

/// The canonical `commands::chat` module is a desktop command surface (windows,
/// Tauri state). The runtime only needs its transport-neutral
/// `setup_chat_proxy` helper, which is built next to the ccproxy code that
/// consumes it, so the shared call sites stay unchanged in both crates.
pub mod chat {
    pub use crate::ccproxy::proxy_settings::setup_chat_proxy;
}
