//! Rules the ChatHub page follows on every platform.
//!
//! The page is created with `wry` directly instead of the Tauri webview APIs, which is
//! what keeps it out of reach of the Tauri IPC, and the platform carriers in this module
//! only decide where that page is placed.

use std::path::PathBuf;

use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, WebviewWindow, Wry};
use tauri_plugin_opener::OpenerExt;
use wry::{NewWindowResponse, ProxyConfig, WebContext, WebViewBuilder};

use super::types::{
    CHAT_HUB_DEFAULT_WIDTH, CHAT_HUB_HOST_WINDOW_LABEL, CHAT_HUB_MIN_HOST_WIDTH, CHAT_HUB_MIN_WIDTH,
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

/// Smallest window change worth a resize, in logical pixels.
///
/// A window that already fills the screen has nothing to add, and a rounding residue must
/// not send a resize the platform would animate.
const MIN_WINDOW_CHANGE: f64 = 0.5;

/// Rounding residue a reported window width may carry, in logical pixels.
///
/// A window size is reported in physical pixels and scaled back to logical ones, so the width a
/// layout started from can come back rounded.
const REPORTED_WIDTH_TOLERANCE: f64 = 2.0;

/// Whether a window report describes the geometry a layout this side asked for replaced.
///
/// A resize is reported before the window has applied the change it describes (the same
/// behaviour window geometry is saved with, see `WINDOW_GEOMETRY_SAVE_DELAY`), so the report
/// that follows a page docking carries the width the window had before the page made room for
/// itself: the width the page was laid out for minus the width the page took, which is
/// `layout_width - taken_width`. Laying the page out for that width would put it back over the
/// workflow UI, so the layout is kept for such a report. Every other report describes a window
/// the page has to follow, including one the user shrank.
pub fn report_predates_layout(reported_width: f64, layout_width: f64, taken_width: f64) -> bool {
    if !reported_width.is_finite() || !layout_width.is_finite() || !taken_width.is_finite() {
        return false;
    }

    let previous_window_width = layout_width - taken_width;
    (reported_width - previous_window_width).abs() <= REPORTED_WIDTH_TOLERANCE
}

/// Rectangle the host window takes so the workflow UI keeps its width next to the page.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HostWindowGeometry {
    /// Left edge, in logical pixels.
    pub left: f64,
    /// Width, in logical pixels.
    pub width: f64,
}

/// Room the screen gives a window, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenArea {
    /// Left edge of the work area, in logical pixels.
    pub left: f64,
    /// Width of the work area, in logical pixels.
    pub width: f64,
}

/// Rectangle the window needs to make room for the docked page.
///
/// The page is a second webview inside the workflow window, so the workflow UI can only
/// keep its width if the window itself grows: the page takes the added space at the right
/// edge while the workflow UI stays where it was.
///
/// The window never grows past the screen. A window that would stick out is capped at the
/// width of the work area and moved left until its right edge lands on the right edge of
/// the screen, so an expansion can never push a part of the window out of view.
///
/// `None` means the window cannot take the page without giving up workflow width, which is
/// the case when it already fills the screen: the page then takes its space from the
/// workflow UI, exactly as it did before.
pub fn room_for_page(
    window_left: f64,
    window_width: f64,
    screen: ScreenArea,
    page_width: f64,
) -> Option<HostWindowGeometry> {
    let measured = window_left.is_finite() && window_width.is_finite() && page_width.is_finite();
    if !measured || page_width <= 0.0 || !screen.width.is_finite() || screen.width <= 0.0 {
        return None;
    }

    let width = (window_width + page_width).min(screen.width);
    if width - window_width < MIN_WINDOW_CHANGE {
        return None;
    }

    let left = window_left
        .min(screen.left + screen.width - width)
        .max(screen.left);

    Some(HostWindowGeometry { left, width })
}

/// Makes room for the docked page by widening the host window.
///
/// Returns the width that was added, in logical pixels, which is the exact amount the page
/// hands back when it goes away. A platform that cannot report the screen, or a window that
/// already fills it, adds nothing and the page then opens the way it always has.
pub fn widen_host_window(host: &WebviewWindow<Wry>, page_width: f64) -> f64 {
    match try_widen_host_window(host, page_width) {
        Ok(added) => added,
        Err(error) => {
            log::warn!(
                "Failed to make room for the ChatHub page in the '{}' window: {}",
                host.label(),
                error
            );
            0.0
        }
    }
}

/// Hands the width the docked page took back to the workflow UI.
///
/// The window keeps its left edge, so closing the page undoes exactly the widening the
/// opening performed. A narrower window than the platform allows is not this side's
/// business: the platform keeps its own minimum width.
pub fn narrow_host_window(host: &WebviewWindow<Wry>, added: f64) {
    if let Err(error) = try_narrow_host_window(host, added) {
        log::warn!(
            "Failed to hand the ChatHub page width back in the '{}' window: {}",
            host.label(),
            error
        );
    }
}

fn try_widen_host_window(host: &WebviewWindow<Wry>, page_width: f64) -> Result<f64> {
    let scale_factor = host.scale_factor()?;
    let Some(screen) = screen_area(host, scale_factor) else {
        return Ok(0.0);
    };

    let window_size = host.inner_size()?.to_logical::<f64>(scale_factor);
    let position = host.outer_position()?.to_logical::<f64>(scale_factor);

    let Some(geometry) = room_for_page(position.x, window_size.width, screen, page_width) else {
        return Ok(0.0);
    };

    host.set_size(tauri::Size::Logical(LogicalSize::new(
        geometry.width,
        window_size.height,
    )))?;

    // The window is only moved when the page would have pushed it off the screen.
    if (geometry.left - position.x).abs() >= MIN_WINDOW_CHANGE {
        host.set_position(tauri::Position::Logical(LogicalPosition::new(
            geometry.left,
            position.y,
        )))?;
    }

    Ok(geometry.width - window_size.width)
}

fn try_narrow_host_window(host: &WebviewWindow<Wry>, added: f64) -> Result<()> {
    if !added.is_finite() || added < MIN_WINDOW_CHANGE {
        return Ok(());
    }

    let scale_factor = host.scale_factor()?;
    let window_size = host.inner_size()?.to_logical::<f64>(scale_factor);
    let width = window_size.width - added;
    if width < MIN_WINDOW_CHANGE {
        return Ok(());
    }

    host.set_size(tauri::Size::Logical(LogicalSize::new(
        width,
        window_size.height,
    )))?;

    Ok(())
}

/// Work area of the screen the window is on, in logical pixels.
///
/// The work area is used instead of the full resolution so the window never grows under
/// the menu bar or over a dock the user keeps at the side of the screen.
fn screen_area(host: &WebviewWindow<Wry>, scale_factor: f64) -> Option<ScreenArea> {
    let monitor = host.current_monitor().ok().flatten()?;
    let work_area = monitor.work_area();
    let left = work_area.position.x as f64 / scale_factor;
    let width = work_area.size.width as f64 / scale_factor;

    if !left.is_finite() || !width.is_finite() || width <= 0.0 {
        return None;
    }

    Some(ScreenArea { left, width })
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

    /// The window grows to the right, so the workflow UI keeps the width it had and the page
    /// takes the added space at the right edge.
    #[test]
    fn the_window_grows_to_the_right_for_the_page() {
        let screen = ScreenArea {
            left: 0.0,
            width: 1440.0,
        };

        assert_eq!(
            room_for_page(100.0, 600.0, screen, 600.0),
            Some(HostWindowGeometry {
                left: 100.0,
                width: 1200.0
            })
        );
        // A screen that starts further right is measured in the same coordinate space.
        assert_eq!(
            room_for_page(
                0.0,
                1024.0,
                ScreenArea {
                    left: 1280.0,
                    width: 1920.0
                },
                600.0
            ),
            Some(HostWindowGeometry {
                left: 1280.0,
                width: 1624.0
            })
        );
    }

    /// A window that would stick out of the screen is capped at the screen width and moved
    /// left instead, so an expansion never leaves a part of the window out of view.
    #[test]
    fn a_window_that_would_stick_out_is_moved_back_onto_the_screen() {
        let screen = ScreenArea {
            left: 0.0,
            width: 1440.0,
        };

        // 900 + 1024 + 600 reaches past the right edge, so the window takes the whole screen
        // width and slides left until its right edge lands on the right edge of the screen.
        assert_eq!(
            room_for_page(900.0, 1024.0, screen, 600.0),
            Some(HostWindowGeometry {
                left: 0.0,
                width: 1440.0
            })
        );
        // A window that only reaches past the edge by a little moves by that little.
        assert_eq!(
            room_for_page(400.0, 800.0, screen, 600.0),
            Some(HostWindowGeometry {
                left: 40.0,
                width: 1400.0
            })
        );
        // A window that started off the left edge is pulled back onto the screen.
        assert_eq!(
            room_for_page(-300.0, 800.0, screen, 600.0),
            Some(HostWindowGeometry {
                left: 0.0,
                width: 1400.0
            })
        );
    }

    /// A window that already fills the screen makes no room: the page then takes its space
    /// from the workflow UI, which is what happened before the window could grow.
    #[test]
    fn a_window_that_fills_the_screen_makes_no_room() {
        let screen = ScreenArea {
            left: 0.0,
            width: 1440.0,
        };

        assert_eq!(room_for_page(0.0, 1440.0, screen, 600.0), None);
        assert_eq!(room_for_page(-200.0, 1440.0, screen, 600.0), None);
        // A window that is already wider than the screen is never narrowed by this rule.
        assert_eq!(room_for_page(0.0, 1600.0, screen, 600.0), None);
    }

    /// Nonsense measurements never move a window.
    #[test]
    fn unusable_measurements_leave_the_window_alone() {
        let screen = ScreenArea {
            left: 0.0,
            width: 1440.0,
        };

        assert_eq!(room_for_page(f64::NAN, 1024.0, screen, 600.0), None);
        assert_eq!(room_for_page(0.0, 1024.0, screen, f64::NAN), None);
        assert_eq!(room_for_page(0.0, 1024.0, screen, 0.0), None);
        assert_eq!(
            room_for_page(
                0.0,
                1024.0,
                ScreenArea {
                    left: 0.0,
                    width: 0.0
                },
                600.0
            ),
            None
        );
    }

    /// A window that grew for the page keeps reporting the width it had before it grew, so the
    /// layout the page was given is kept instead of being computed from that report.
    #[test]
    fn a_report_of_the_geometry_a_layout_replaced_is_recognized() {
        // The window the page made room from is the width the page was laid out for minus the
        // width the page took: exactly what the report after a docking carries.
        assert!(report_predates_layout(871.0, 1471.0, 600.0));
        // The report is rounded when a physical size is scaled back to logical pixels.
        assert!(report_predates_layout(872.0, 1471.0, 600.0));
        // The window caught up, so the report describes the window the page lives in.
        assert!(!report_predates_layout(1471.0, 1471.0, 600.0));
        assert!(!report_predates_layout(1512.0, 1471.0, 600.0));
        // A window the user shrank is followed instead of ignored.
        assert!(!report_predates_layout(900.0, 1471.0, 600.0));
        assert!(!report_predates_layout(800.0, 1471.0, 600.0));
        // Nonsense measurements never keep a layout alive.
        assert!(!report_predates_layout(f64::NAN, 1471.0, 600.0));
        assert!(!report_predates_layout(871.0, f64::NAN, 600.0));
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
