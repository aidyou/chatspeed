// pub mod dedup;
// pub mod similarity;
pub mod ai_temp;
pub mod fs;
pub mod lang;
// The TSID generator backs runtime-only identifiers (workflow sessions, shell
// output files).
pub mod tsid;
pub mod util;
// Windowless chat stream registry. Every chat turn runs in the standalone
// runtime, so the runtime owns this window-free implementation of the registry.
// The former Tauri-window implementation now lives in the desktop crate's
// `libs` adapter and is never linked by this crate.
#[path = "window_channels_runtime.rs"]
pub mod window_channels;
