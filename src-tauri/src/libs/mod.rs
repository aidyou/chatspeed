// pub mod dedup;
// pub mod similarity;
pub mod ai_temp;
pub mod fs;
pub mod lang;
pub mod tsid;
pub mod util;
// Desktop-only: wraps Tauri webview handles, which the desktop-free runtime
// crate must not link.
#[cfg(feature = "desktop")]
pub mod webview_proxy;
// Same module path in both crates: desktop streams chat output into Tauri
// windows; the runtime has no window and lets a control-plane route register the
// one stream each chat turn needs.
#[cfg(feature = "desktop")]
pub mod window_channels;
#[cfg(not(feature = "desktop"))]
#[path = "window_channels_runtime.rs"]
pub mod window_channels;
