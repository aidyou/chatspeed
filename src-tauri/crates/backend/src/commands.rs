//! Runtime view of the canonical command core.
//!
//! The desktop `commands` module holds the Tauri adapters (window, clipboard,
//! updater, tray, ...) that the runtime must not link, so this crate owns only
//! the transport-neutral workflow command file (`src/commands/workflow.rs`).
//! That file keeps the canonical `*_core` helpers the workflow application
//! service and the automation dispatcher call directly; the desktop keeps its
//! `#[tauri::command]` wrappers in its own `src/commands/workflow.rs`.

pub mod workflow;

/// The canonical `commands::chat` module is a desktop command surface (windows,
/// Tauri state). The runtime only needs its transport-neutral
/// `setup_chat_proxy` helper, which is built next to the ccproxy code that
/// consumes it, so the shared call sites stay unchanged in both crates.
pub mod chat {
    pub use crate::ccproxy::proxy_settings::setup_chat_proxy;
}
