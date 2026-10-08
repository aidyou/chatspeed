//! Rules the ChatHub page follows on every platform.
//!
//! The page is created with `wry` directly instead of the Tauri webview APIs, which is
//! what keeps it out of reach of the Tauri IPC, and the platform carriers in this module
//! only decide where that page is placed.

use std::path::PathBuf;

use tauri::{AppHandle, Manager, WebviewWindow, Wry};
use tauri_plugin_opener::OpenerExt;
use wry::{NewWindowResponse, ProxyConfig, WebContext, WebViewBuilder};

use super::types::{
    CHAT_HUB_DEFAULT_WIDTH, CHAT_HUB_HOST_WINDOW_LABEL, CHAT_HUB_MIN_HOST_WIDTH, CHAT_HUB_MIN_WIDTH,
};
use super::ChatHubPageState;
use crate::error::{AppError, Result};

/// Tells the frontend how it has to make room for the docked page.
///
/// Every platform reserves the right dock itself: the frontend measures the dock rectangle in
/// its own layout and hands it to the carrier, which places a native view inside it. The
/// carrier therefore never changes the window layout on any platform, so `reserve` is the mode
/// everywhere.
pub fn view_mode() -> &'static str {
    "reserve"
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

/// Largest corner radius the page accepts, so a bad measurement cannot eat into the page.
const MAX_PAGE_CORNER_RADIUS: f64 = 30.0;

/// Builds the page webview with the rules that hold on every platform.
///
/// The page may only navigate to web content, a new window request goes to the browser of
/// the platform instead of spawning an unmanaged window, the clipboard is enabled because
/// the embedded chat needs it to paste messages, and the proxy the network settings ask
/// for is applied here: a webview can only be given one while it is built.
///
/// `corner_radius` is the radius the window actually draws at its bottom right, which a
/// rectangle stacked over the workflow UI paints over. A platform whose window keeps
/// square corners reports none, and the page keeps its rectangular edge there.
pub fn page_builder<'a>(
    web_context: &'a mut WebContext,
    url: &str,
    proxy: Option<ProxyConfig>,
    corner_radius: f64,
    app: &AppHandle<Wry>,
) -> WebViewBuilder<'a> {
    build_page(web_context, url, proxy, corner_radius, false, app)
}

/// Builds a page that always paints over the window corner, for the Linux dock.
///
/// A backend that cannot shape the native view (Wayland) lets the page give the corner back itself,
/// and the radius can change while the page is alive because the frontend resends it with every
/// dock snapshot. The transparency of a webview cannot be toggled without rebuilding it, and a
/// rebuild would drop the browsing session, so such a page is created see-through and carries the
/// shared corner script even at a zero radius. The script's runtime setter then follows every later
/// radius, including zero, without a reload.
#[cfg(target_os = "linux")]
pub fn dock_page_builder<'a>(
    web_context: &'a mut WebContext,
    url: &str,
    proxy: Option<ProxyConfig>,
    corner_radius: f64,
    app: &AppHandle<Wry>,
) -> WebViewBuilder<'a> {
    build_page(web_context, url, proxy, corner_radius, true, app)
}

/// Builds the page webview with the rules that hold on every platform.
///
/// `always_see_through` forces the see-through page and the shared corner script even at a zero
/// radius, which the Linux dock needs so a radius that grows later can still be given back.
fn build_page<'a>(
    web_context: &'a mut WebContext,
    url: &str,
    proxy: Option<ProxyConfig>,
    corner_radius: f64,
    always_see_through: bool,
    app: &AppHandle<Wry>,
) -> WebViewBuilder<'a> {
    // The handlers below outlive this call, so the page keeps its own handle to open a link
    // even after the entry that created it was closed.
    let opener = app.clone();
    let builder = WebViewBuilder::new_with_web_context(web_context)
        .with_url(url)
        .with_clipboard(true)
        .with_navigation_handler(|url| is_web_url(&url))
        .with_new_window_req_handler(move |url, _features| {
            open_in_browser(&opener, &url);
            NewWindowResponse::Deny
        });

    // A page that hands its corner back to the window also has to be see-through: an
    // opaque page would show its own background where the window border belongs.
    let radius = corner_radius.clamp(0.0, MAX_PAGE_CORNER_RADIUS);
    let builder = if always_see_through || radius > 0.0 {
        builder
            .with_transparent(true)
            .with_initialization_script(corner_script(radius))
    } else {
        builder
    };

    match proxy {
        Some(proxy) => builder.with_proxy_config(proxy),
        None => builder,
    }
}

/// Whether a URL addresses web content, which is the only thing the page may reach.
///
/// A chat site is untrusted content, so it may neither navigate the page to a local file nor
/// hand a scheme of its own to the platform: both a navigation and a new window request are
/// judged by this rule.
fn is_web_url(url: &str) -> bool {
    matches!(url.split(':').next(), Some("http") | Some("https"))
}

/// Opens a link the page asked to open in a new window in the browser of the platform.
///
/// A chat site opens a link in a new tab, which is what `target="_blank"` and `window.open`
/// ask the webview for. The docked page has no tab strip to put a second page in, so the
/// request is refused by the webview (see [`page_builder`]) and handed to the browser
/// instead: the link then opens where the user keeps their browsing session, and the page
/// itself stays the single page it is.
///
/// Only web content is handed over. Anything else is refused exactly like a navigation of
/// the page itself, so an embedded site can never make the platform act on a scheme it
/// picked for itself.
fn open_in_browser(app: &AppHandle<Wry>, url: &str) {
    if !is_web_url(url) {
        log::debug!("ChatHub refused to open a new window request for '{}'", url);
        return;
    }

    if let Err(error) = app.opener().open_url(url, None::<&str>) {
        log::warn!(
            "ChatHub failed to open a new window request for '{}': {}",
            url,
            error
        );
    }
}

/// Script that leaves the rounded bottom-right corner of the window unpainted.
///
/// The docked page is a rectangular native view, so no border radius of the workflow UI can cut
/// it: the page has to give that corner back itself, and only a backend that cannot shape the
/// native view lets the page do it. The script is shared with a plugin UI panel and with the
/// native dock surface ([`crate::native_dock::bottom_right_corner_script`]), so the corner the two
/// give back cannot drift apart.
fn corner_script(radius: f64) -> String {
    crate::native_dock::bottom_right_corner_script(radius)
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
            log::warn!(
                "Failed to resolve the ChatHub page data directory: {}",
                error
            );
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
        assert_eq!(
            clamp_width(1600.0, 1500.0),
            1600.0 - CHAT_HUB_MIN_HOST_WIDTH
        );
        // A usable request is kept as it is, so dragging the splitter is exact.
        assert_eq!(clamp_width(1600.0, 640.0), 640.0);
        // A window that is too narrow still yields the minimum page width.
        assert_eq!(clamp_width(700.0, 640.0), CHAT_HUB_MIN_WIDTH);
        // Nonsense input falls back to the default instead of poisoning the layout.
        assert_eq!(clamp_width(f64::NAN, 640.0), CHAT_HUB_DEFAULT_WIDTH);
        assert_eq!(clamp_width(1600.0, f64::NAN), CHAT_HUB_DEFAULT_WIDTH);
    }

    #[test]
    fn the_frontend_is_told_to_reserve_the_dock_on_every_platform() {
        // No carrier changes the window layout any more, so the frontend reserves the right
        // dock itself on every platform.
        assert_eq!(view_mode(), "reserve");
    }

    #[test]
    fn the_page_can_only_ever_load_web_content() {
        let source = include_str!("page.rs");

        // Navigation is restricted at the webview boundary...
        assert!(source.contains(r#"matches!(url.split(':').next(), Some("http") | Some("https"))"#));
        // ...a new window request never becomes a window inside the docked page...
        assert!(source.contains("NewWindowResponse::Deny"));
        // ...and the page is built with wry, so it never receives Tauri IPC.
        assert!(source.contains("WebViewBuilder::new_with_web_context"));
        assert!(!source.contains(concat!("Webview", "WindowBuilder")));
        assert!(!source.contains(concat!("add_", "child")));
    }

    /// A chat site opens a link in a new tab, which the docked page has nowhere to put, so the
    /// request has to reach the browser of the platform instead of disappearing.
    #[test]
    fn a_new_window_request_goes_to_the_browser_of_the_platform() {
        let source = include_str!("page.rs");

        let handler = source
            .split(".with_new_window_req_handler")
            .nth(1)
            .expect("the new window handler is missing")
            .split("});")
            .next()
            .expect("the new window handler is not terminated");

        // Every request is handed to the browser of the platform...
        assert!(handler.contains("open_in_browser(&opener, &url)"));
        // ...and the webview still opens no window of its own for it.
        assert!(handler.contains("NewWindowResponse::Deny"));

        let opener = source
            .split("fn open_in_browser")
            .nth(1)
            .expect("the browser opener is missing")
            .split("\n}\n")
            .next()
            .expect("the browser opener is not terminated");

        // Only web content reaches the platform...
        assert!(opener.contains("if !is_web_url(url)"));
        // ...through the opener the application already uses for its own links.
        assert!(opener.contains(".open_url(url, None::<&str>)"));
    }

    #[test]
    fn only_web_content_is_handed_to_the_platform() {
        assert!(is_web_url("https://example.com"));
        assert!(is_web_url("http://example.com/chat?q=1#top"));
        // A scheme the embedded site picks for itself is refused, exactly like a navigation.
        assert!(!is_web_url("file:///etc/passwd"));
        assert!(!is_web_url("mailto:someone@example.com"));
        assert!(!is_web_url("chatspeed://open?url=https://example.com"));
        assert!(!is_web_url("javascript:alert(1)"));
        assert!(!is_web_url("about:blank"));
    }

    #[test]
    fn the_page_uses_one_stable_profile_directory() {
        let source = include_str!("page.rs");

        assert!(source.contains(r#".join("chat_hub_page")"#));
    }

    /// Guard for the rounded window border the page gives back: the stacked page is a
    /// rectangle, so the corner can only come back from the page itself, and only on a
    /// see-through page.
    #[test]
    fn the_page_gives_a_rounded_window_border_back() {
        let source = include_str!("page.rs");

        let branch = source
            .split("let radius = corner_radius.clamp")
            .nth(1)
            .expect("the window border branch is missing")
            .split("match proxy {")
            .next()
            .expect("the window border branch is not terminated");

        // A window without a rounded border reports no radius, so the page keeps its
        // rectangular edge instead of turning see-through for nothing...
        assert!(branch.contains("if always_see_through || radius > 0.0"));
        // ...unless a caller forces it, which the Linux dock does so a radius that grows later can
        // still be given back without rebuilding the page...
        assert!(branch.contains(".with_transparent(true)"));
        assert!(branch.contains(".with_initialization_script(corner_script(radius))"));

        // The script rounds the corner of every layer that paints it, and it is the single
        // bottom-right script the native dock surface owns, so a plugin panel and this page cannot
        // drift apart.
        let script = corner_script(15.0);

        assert!(script.contains("border-bottom-right-radius"));
        assert!(script.contains("document.documentElement"));
        // A decorative layer carries pointer-events: none and never shows up in a hit test,
        // so the corner is found by geometry instead of by hit testing.
        assert!(script.contains("document.querySelectorAll('*')"));
        assert!(script.contains("getBoundingClientRect"));
        // A pseudo element paints a box of its own, which the radius of its host cannot cut.
        assert!(script.contains("'data-cs-corner'"));
        assert!(script.contains("'::after' : '::before'"));
        // The page does not own a second script: it reuses the shared one.
        assert!(source.contains("crate::native_dock::bottom_right_corner_script(radius)"));
    }

    /// Guard for the injected JavaScript itself: the radius reaches the page through the runtime
    /// setter, and the braces of the script have to survive the format string that carries it.
    #[test]
    fn the_corner_script_carries_the_radius_through_the_runtime_setter() {
        let script = corner_script(15.0);

        assert!(script.contains("state.set(15);"));
        assert!(script.contains("function roundCorner() {"));
        assert!(!script.contains("{{"));
        assert!(!script.contains("}}"));
    }

    /// The Linux dock builds its page see-through even at a zero radius, so a radius that appears
    /// later can be given back without rebuilding the webview and losing the session.
    #[test]
    fn the_linux_dock_page_is_always_see_through() {
        let source = include_str!("page.rs");

        assert!(source.contains("pub fn dock_page_builder"));
        assert!(source.contains("build_page(web_context, url, proxy, corner_radius, true, app)"));
        // The plain builder still keys see-through on the radius it was handed.
        assert!(source.contains("build_page(web_context, url, proxy, corner_radius, false, app)"));
    }
}
