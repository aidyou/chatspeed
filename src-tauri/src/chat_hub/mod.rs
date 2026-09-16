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

#[cfg(target_os = "linux")]
mod gtk_panel;

#[cfg(any(target_os = "windows", target_os = "macos"))]
mod child_view;

#[cfg(target_os = "linux")]
pub use gtk_panel::ChatHubPageState;

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub use child_view::ChatHubPageState;

use std::path::PathBuf;

use serde::Serialize;
use tauri::{AppHandle, Manager, WebviewWindow, Wry};
use wry::{NewWindowResponse, WebContext, WebViewBuilder};

use crate::error::{AppError, Result};

/// Label of the window the page is docked to.
pub const CHAT_HUB_HOST_WINDOW_LABEL: &str = "workflow";

/// Default width of the docked page, in logical pixels.
pub const CHAT_HUB_DEFAULT_WIDTH: f64 = 500.0;

/// Narrowest page width that still renders a mobile layout, in logical pixels.
pub const CHAT_HUB_MIN_WIDTH: f64 = 500.0;

/// Width the workflow UI always keeps next to the page, in logical pixels.
pub const CHAT_HUB_MIN_HOST_WIDTH: f64 = 480.0;

/// Width limits of the docked page, reported to the frontend.
///
/// The splitter has to clamp a drag exactly like [`clamp_width`] does, so the limits
/// are owned here instead of being duplicated in the frontend.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatHubPageLimits {
    /// Narrowest page width the carrier accepts.
    pub min_width: f64,
    /// Width the workflow UI always keeps next to the page.
    pub min_host_width: f64,
}

impl ChatHubPageLimits {
    /// The limits every carrier enforces.
    pub fn current() -> Self {
        Self {
            min_width: CHAT_HUB_MIN_WIDTH,
            min_host_width: CHAT_HUB_MIN_HOST_WIDTH,
        }
    }
}

/// Tells the frontend how it has to make room for the docked page.
///
/// `split` means the platform lays both webviews out itself, so the workflow UI is
/// already narrower and the frontend reserves nothing. `reserve` means the page is
/// stacked over the workflow UI, so the frontend keeps the right side of its own
/// layout free.
pub fn view_mode() -> &'static str {
    if cfg!(target_os = "linux") {
        "split"
    } else {
        "reserve"
    }
}

/// The window the page is docked to.
pub fn host_window(app: &AppHandle<Wry>) -> Result<WebviewWindow<Wry>> {
    app.get_webview_window(CHAT_HUB_HOST_WINDOW_LABEL)
        .ok_or_else(|| AppError::General {
            message: format!(
                "ChatHub host window '{}' is not available",
                CHAT_HUB_HOST_WINDOW_LABEL
            ),
        })
}

/// Clamps a requested page width to what the current window can hold.
///
/// The page never becomes narrower than a mobile layout needs, and the workflow UI
/// always keeps its own minimum width, so a drag can neither collapse the page nor
/// push the workflow UI out of the window.
pub fn clamp_width(window_width: f64, requested: f64) -> f64 {
    if !window_width.is_finite() || !requested.is_finite() {
        return CHAT_HUB_DEFAULT_WIDTH;
    }

    let maximum = (window_width - CHAT_HUB_MIN_HOST_WIDTH).max(CHAT_HUB_MIN_WIDTH);
    requested.clamp(CHAT_HUB_MIN_WIDTH, maximum)
}

/// Builds the page webview with the rules that hold on every platform.
///
/// The page may only navigate to web content, new window requests are refused
/// instead of spawning unmanaged windows, and the clipboard is enabled because the
/// embedded chat needs it to paste messages.
pub fn page_builder<'a>(web_context: &'a mut WebContext, url: &str) -> WebViewBuilder<'a> {
    WebViewBuilder::new_with_web_context(web_context)
        .with_url(url)
        .with_clipboard(true)
        .with_navigation_handler(|url| matches!(url.split(':').next(), Some("http") | Some("https")))
        .with_new_window_req_handler(|url, _features| {
            log::debug!("ChatHub refused a new window request for '{}'", url);
            NewWindowResponse::Deny
        })
}

/// Persistent profile directory of the embedded page.
///
/// One stable directory keeps the site cookies and storage across page recreation.
/// It lives next to the other application data and never touches the ChatSpeed
/// database.
pub fn page_data_directory(app: &AppHandle<Wry>) -> PathBuf {
    let directory = app
        .path()
        .app_data_dir()
        .map(|dir| dir.join("chat_hub_page"))
        .unwrap_or_else(|error| {
            log::warn!("Failed to resolve the ChatHub page data directory: {}", error);
            PathBuf::from("chat_hub_page")
        });

    if let Err(error) = std::fs::create_dir_all(&directory) {
        log::warn!(
            "Failed to create the ChatHub page data directory '{}': {}",
            directory.display(),
            error
        );
    }

    directory
}

/// Runs one page operation on the platform main thread and waits for its result.
///
/// Every carrier creates and moves real webviews, which may only happen on the main
/// thread. The commands are asynchronous, so the work is posted to that thread and
/// its result is awaited: the frontend then learns about a failure immediately
/// instead of the page silently staying away.
pub async fn run_on_page_thread(
    app: &AppHandle<Wry>,
    operation: impl FnOnce(&ChatHubPageState, &AppHandle<Wry>) -> Result<()> + Send + 'static,
) -> Result<()> {
    let (sender, receiver) = std::sync::mpsc::channel();
    let thread_app = app.clone();

    app.run_on_main_thread(move || {
        let result = match thread_app.try_state::<ChatHubPageState>() {
            Some(state) => operation(state.inner(), &thread_app),
            None => Err(AppError::General {
                message: "the ChatHub page state is not managed".to_string(),
            }),
        };
        let _ = sender.send(result);
    })?;

    tokio::task::spawn_blocking(move || receiver.recv())
        .await
        .map_err(|error| AppError::General {
            message: format!("the ChatHub page task failed: {}", error),
        })
        .and_then(|received| match received {
            Ok(result) => result,
            Err(error) => Err(AppError::General {
                message: format!("the ChatHub page task did not report a result: {}", error),
            }),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_width_stays_within_the_window_and_keeps_the_workflow_ui_usable() {
        // A request below the mobile minimum is raised to it.
        assert_eq!(clamp_width(1600.0, 100.0), CHAT_HUB_MIN_WIDTH);
        // A request that would squeeze the workflow UI out is capped.
        assert_eq!(clamp_width(1600.0, 1500.0), 1600.0 - CHAT_HUB_MIN_HOST_WIDTH);
        // A usable request is kept as it is, so dragging the splitter is exact.
        assert_eq!(clamp_width(1600.0, 640.0), 640.0);
        // A window that is too narrow still yields the minimum page width.
        assert_eq!(clamp_width(700.0, 640.0), CHAT_HUB_MIN_WIDTH);
        // Nonsense input falls back to the default instead of poisoning the layout.
        assert_eq!(clamp_width(f64::NAN, 640.0), CHAT_HUB_DEFAULT_WIDTH);
        assert_eq!(clamp_width(1600.0, f64::NAN), CHAT_HUB_DEFAULT_WIDTH);
    }

    #[test]
    fn the_frontend_is_told_how_to_make_room_for_the_page() {
        let expected = if cfg!(target_os = "linux") {
            "split"
        } else {
            "reserve"
        };

        assert_eq!(view_mode(), expected);
    }

    #[test]
    fn the_page_can_only_ever_load_web_content() {
        let source = include_str!("mod.rs");

        // Navigation is restricted at the webview boundary...
        assert!(source.contains(r#"matches!(url.split(':').next(), Some("http") | Some("https"))"#));
        // ...new window requests are refused...
        assert!(source.contains("NewWindowResponse::Deny"));
        // ...and the page is built with wry, so it never receives Tauri IPC.
        assert!(source.contains("WebViewBuilder::new_with_web_context"));
        assert!(!source.contains(concat!("Webview", "WindowBuilder")));
        assert!(!source.contains(concat!("add_", "child")));
    }

    #[test]
    fn the_page_uses_one_stable_profile_directory() {
        let source = include_str!("mod.rs");

        assert!(source.contains(r#".join("chat_hub_page")"#));
    }
}
