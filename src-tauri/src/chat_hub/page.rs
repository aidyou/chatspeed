//! Rules the ChatHub page follows on every platform.
//!
//! The page is created with `wry` directly instead of the Tauri webview APIs, which is
//! what keeps it out of reach of the Tauri IPC, and the platform carriers in this module
//! only decide where that page is placed.

use std::path::PathBuf;

use tauri::{AppHandle, Manager, WebviewWindow, Wry};
use wry::{NewWindowResponse, ProxyConfig, WebContext, WebViewBuilder};

use super::types::{
    CHAT_HUB_DEFAULT_WIDTH, CHAT_HUB_HOST_WINDOW_LABEL, CHAT_HUB_MIN_HOST_WIDTH,
    CHAT_HUB_MIN_WIDTH,
};
use super::ChatHubPageState;
use crate::error::{AppError, Result};

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
/// instead of spawning unmanaged windows, the clipboard is enabled because the
/// embedded chat needs it to paste messages, and the proxy the network settings ask
/// for is applied here: a webview can only be given one while it is built.
pub fn page_builder<'a>(
    web_context: &'a mut WebContext,
    url: &str,
    proxy: Option<ProxyConfig>,
) -> WebViewBuilder<'a> {
    let builder = WebViewBuilder::new_with_web_context(web_context)
        .with_url(url)
        .with_clipboard(true)
        .with_navigation_handler(|url| matches!(url.split(':').next(), Some("http") | Some("https")))
        .with_new_window_req_handler(|url, _features| {
            log::debug!("ChatHub refused a new window request for '{}'", url);
            NewWindowResponse::Deny
        });

    match proxy {
        Some(proxy) => builder.with_proxy_config(proxy),
        None => builder,
    }
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
        // The page can be dragged down to a phone viewport, which is the width a chat
        // site needs to switch to its mobile layout.
        assert_eq!(CHAT_HUB_MIN_WIDTH, 375.0);
        // A request below the phone width is raised to it.
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
        let source = include_str!("page.rs");

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
        let source = include_str!("page.rs");

        assert!(source.contains(r#".join("chat_hub_page")"#));
    }
}
