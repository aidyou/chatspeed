//! Desktop `libs` adapter.
//!
//! Re-exports the runtime backend's transport-neutral helpers (`fs`, `lang`,
//! `util`, the TSID generator and the windowless chat stream registry) and adds
//! the desktop-only `webview_proxy` module, which resolves a webview's proxy from
//! the runtime configuration. `window_channels.rs` (the Tauri-window chat stream
//! registry) is retained next to it as an unwired desktop-only source: chat
//! streaming now runs in the runtime and reaches the frontend over SSE, so
//! nothing constructs the registry any more.

pub use chatspeed_runtime_backend::libs::*;

pub mod webview_proxy;