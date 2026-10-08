//! ChatHub page carrier.
//!
//! A ChatHub entry is a web chat rendered in a native webview that is docked to the right of
//! the workflow UI, inside the Workflow window. Every entry has its own tab, and every tab
//! keeps its own webview (with its own browsing session) while another tab is shown.
//!
//! The page is created with `wry` directly instead of the Tauri webview APIs. That is a
//! deliberate isolation boundary: a Tauri webview inside the Workflow window would inherit that
//! window's capabilities (`fs`, `core:window`, ...), because a command is authorized when its
//! capability matches the webview label *or* the window label. A plain `wry` webview has no
//! Tauri IPC at all, so the embedded site can never reach a ChatSpeed command, whatever the
//! capability set says.
//!
//! Where the page goes is decided by the frontend on every platform: it reserves the right dock
//! in its own layout, measures that placeholder in logical pixels and hands the rectangle to the
//! carrier. No carrier widens, narrows or splits the window any more; the rectangle is the single
//! geometry every carrier places its native view in. The native views of ChatHub tabs and of
//! plugin UI panels share one overlay through [`crate::native_dock`], so a ChatHub tab can never
//! intercept a pointer event that belongs to the workflow UI or to a plugin panel.
//!
//! The carrier itself is platform specific:
//!
//! - Linux ([`gtk_panel`]): each tab is a bounded overlay child aligned to the start of the
//!   shared overlay.
//! - Windows and macOS ([`child_view`]): each tab is a child view placed at its rectangle.

mod page;
mod proxy;
mod types;

#[cfg(target_os = "linux")]
mod gtk_panel;

#[cfg(any(target_os = "windows", target_os = "macos"))]
mod child_view;

#[cfg(target_os = "linux")]
pub use gtk_panel::ChatHubPageState;

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub use child_view::ChatHubPageState;

pub use page::{
    clamp_width, host_window, page_builder, page_data_directory, run_on_page_thread, view_mode,
};
pub use proxy::page_proxy;
pub use types::{
    ChatHubPageLimits, CHAT_HUB_DEFAULT_WIDTH, CHAT_HUB_HOST_WINDOW_LABEL, CHAT_HUB_MIN_HOST_WIDTH,
    CHAT_HUB_MIN_WIDTH,
};
