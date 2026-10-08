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
    let builder = if radius > 0.0 {
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
/// The docked page is a rectangular native view, so no border radius of the workflow UI
/// can cut it: the page has to give that corner back itself. It is stacked over the
/// workflow UI, which draws the rounded window border, so the page only has to leave the
/// corner transparent ([`page_builder`] makes it so).
///
/// Three rules make that hold on pages that build themselves differently:
///
/// - The corner is given back by every element that paints it. A decorative layer carries
///   `pointer-events: none`, so it never shows up in a hit test and the document is
///   inspected by geometry instead: an element is rounded when it covers the bottom-right
///   point of the viewport and paints something there.
/// - A pseudo element paints a box of its own, which the radius of its host does not cut, so
///   a host that paints the corner through one is marked and the corner reaches it through a
///   rule.
/// - The corner is applied again for a short while after the page load, because a site can
///   build the layer that paints it later than the load event.
fn corner_script(radius: f64) -> String {
    format!(
        r#"(function () {{
  var radius = '{radius}px';
  var transparent = 'rgba(0, 0, 0, 0)';
  var pending = 0;
  var ruled = false;
  function opaque(background) {{
    return !!background && background !== 'transparent' && background !== transparent;
  }}
  function paints(style) {{
    return style.backgroundImage !== 'none' || opaque(style.backgroundColor);
  }}
  function addPseudoRule() {{
    if (ruled) {{
      return;
    }}
    ruled = true;
    var rule = '[data-cs-corner]::before,[data-cs-corner]::after'
      + '{{border-bottom-right-radius:' + radius + ' !important}}';
    try {{
      if (typeof CSSStyleSheet === 'function' && 'adoptedStyleSheets' in document) {{
        var sheet = new CSSStyleSheet();
        sheet.insertRule(rule, 0);
        document.adoptedStyleSheets = document.adoptedStyleSheets.concat([sheet]);
        return;
      }}
    }} catch (error) {{}}
    var sheets = document.styleSheets;
    for (var sheet = 0; sheet < sheets.length; sheet += 1) {{
      try {{
        sheets[sheet].insertRule(rule, sheets[sheet].cssRules.length);
        return;
      }} catch (error) {{}}
    }}
    try {{
      var element = document.createElement('style');
      element.textContent = rule;
      (document.head || document.documentElement).appendChild(element);
    }} catch (error) {{}}
  }}
  function roundCorner() {{
    var x = window.innerWidth - 2;
    var y = window.innerHeight - 2;
    var elements = document.querySelectorAll('*');
    var targets = [];
    for (var index = 0; index < elements.length; index += 1) {{
      var element = elements[index];
      var rect = element.getBoundingClientRect();
      if (rect.width < 1 || rect.height < 1) {{
        continue;
      }}
      if (rect.left > x || rect.top > y || rect.right < x || rect.bottom < y) {{
        continue;
      }}
      var style = window.getComputedStyle(element);
      if (style.display === 'none' || style.visibility === 'hidden'
        || Number(style.opacity) === 0) {{
        continue;
      }}
      if (paints(style)) {{
        targets.push(element);
      }}
      for (var part = 0; part < 2; part += 1) {{
        var pseudo = window.getComputedStyle(element, part ? '::after' : '::before');
        if (pseudo.content && pseudo.content !== 'none' && paints(pseudo)) {{
          element.setAttribute('data-cs-corner', '');
          addPseudoRule();
        }}
      }}
    }}
    for (var target = 0; target < targets.length; target += 1) {{
      targets[target].style.setProperty('border-bottom-right-radius', radius, 'important');
    }}
    document.documentElement.style.setProperty('border-bottom-right-radius', radius, 'important');
  }}
  roundCorner();
  document.addEventListener('DOMContentLoaded', roundCorner);
  window.addEventListener('load', roundCorner);
  window.addEventListener('resize', function () {{
    if (pending) {{
      return;
    }}
    pending = window.setTimeout(function () {{
      pending = 0;
      roundCorner();
    }}, 200);
  }});
  var attempts = 0;
  var retry = window.setInterval(function () {{
    attempts += 1;
    if (attempts > 15) {{
      window.clearInterval(retry);
      return;
    }}
    roundCorner();
  }}, 400);
}})();"#
    )
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
        assert!(branch.contains("if radius > 0.0"));
        // ...and an opaque page would show its own background where the border belongs.
        assert!(branch.contains(".with_transparent(true)"));
        assert!(branch.contains(".with_initialization_script(corner_script(radius))"));

        // The script rounds the corner of every layer that paints it.
        let script = source
            .split("fn corner_script")
            .nth(1)
            .expect("the corner script is missing")
            .split("/// Persistent profile directory")
            .next()
            .expect("the corner script is not terminated");

        assert!(script.contains("border-bottom-right-radius"));
        assert!(script.contains("document.documentElement"));
        // A decorative layer carries pointer-events: none and never shows up in a hit test,
        // so the corner is found by geometry instead of by hit testing.
        assert!(script.contains("document.querySelectorAll('*')"));
        assert!(script.contains("getBoundingClientRect"));
        // A pseudo element paints a box of its own, which the radius of its host cannot cut.
        assert!(script.contains("'data-cs-corner'"));
        assert!(script.contains("'::after' : '::before'"));
    }

    /// Guard for the injected JavaScript itself: the radius has to reach the page as a
    /// ready to use length, and the braces of the script have to survive the format
    /// string that carries it.
    #[test]
    fn the_corner_script_carries_the_radius_as_a_css_length() {
        let script = corner_script(15.0);

        assert!(script.contains("var radius = '15px';"));
        assert!(script.contains("function roundCorner() {"));
        assert!(!script.contains("{{"));
        assert!(!script.contains("}}"));
    }
}
