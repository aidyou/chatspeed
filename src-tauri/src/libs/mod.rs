// pub mod dedup;
// pub mod similarity;
#[cfg(not(feature = "desktop"))]
pub mod ai_temp;
pub mod fs;
pub mod lang;
// The TSID generator backs runtime-only identifiers (workflow sessions, shell
// output files); no desktop path constructs one, so the desktop crate does not
// compile it.
#[cfg(not(feature = "desktop"))]
pub mod tsid;
pub mod util;
// Desktop-only: wraps Tauri webview handles, which the desktop-free runtime
// crate must not link.
#[cfg(feature = "desktop")]
pub mod webview_proxy;
// Windowless chat stream registry. Every chat turn now runs in the standalone
// runtime, so the desktop no longer compiles its Tauri-window implementation
// (`window_channels.rs`); the runtime owns no window and lets a control-plane
// route register the one stream each chat turn needs.
#[cfg(not(feature = "desktop"))]
#[path = "window_channels_runtime.rs"]
pub mod window_channels;
