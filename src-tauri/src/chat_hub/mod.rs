//! ChatHub page carrier.
//!
//! The ChatHub page is a second webview that lives inside the Workflow window,
//! docked to its right edge next to the workflow UI. Keeping the page inside the
//! window is what makes it follow the window by construction: the platform moves a
//! child view together with its parent, and no separate top level window exists for
//! the desktop compositor to decorate with a shadow.
//!
//! The page is created with `wry` directly instead of the Tauri webview APIs. That
//! is a deliberate isolation boundary: a Tauri webview inside the Workflow window
//! would inherit that window's capabilities (`fs`, `core:window`, ...), because a
//! command is authorized when its capability matches the webview label *or* the
//! window label. A plain `wry` webview has no Tauri IPC at all, so the embedded site
//! can never reach a ChatSpeed command, whatever the capability set says.
//!
//! The carrier itself is platform specific:
//!
//! - Linux ([`gtk_panel`]): GTK lays both webviews out side by side, so the workflow
//!   UI simply becomes narrower and no geometry has to be tracked at all.
//! - Windows and macOS ([`child_view`]): the page is a child view placed at an
//!   explicit rectangle and stacked over the workflow UI, so the frontend keeps the
//!   matching space free on its own side.

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
    clamp_width, host_window, narrow_host_window, page_builder, page_data_directory,
    report_predates_layout, room_for_page, run_on_page_thread, view_mode, widen_host_window,
};
pub use proxy::page_proxy;
pub use types::{
    ChatHubPageLimits, CHAT_HUB_DEFAULT_WIDTH, CHAT_HUB_HOST_WINDOW_LABEL, CHAT_HUB_MIN_HOST_WIDTH,
    CHAT_HUB_MIN_WIDTH,
};
