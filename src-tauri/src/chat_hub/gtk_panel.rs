//! Linux carrier of the ChatHub tabs.
//!
//! Every ChatHub entry has its own tab, each tab keeps its own `wry` webview (with its own
//! browsing session) while another tab is shown, and every webview is placed at the rectangle
//! the frontend measured for the right dock. The rectangle is the only geometry the carrier
//! knows: it never widens, narrows or splits the window, so the workflow UI keeps exactly the
//! layout the frontend gave it.
//!
//! The native views are bounded overlay children of the shared dock surface
//! ([`crate::native_dock`]): a holder aligned to the start of the overlay with the rectangle as
//! its margins and size request. A full-area container would swallow every pointer event of the
//! window instead, which is what kept the workflow UI from receiving clicks while a native view
//! was open, so the carrier only ever creates bounded holders.

use std::sync::{Mutex, MutexGuard};

use tauri::{AppHandle, PhysicalSize, Wry};
use wry::{WebContext, WebView, WebViewBuilderExtUnix, WebViewExtUnix};

use super::{clamp_width, host_window, page_builder, page_data_directory, page_proxy};
use crate::db::chat_hub::parse_chat_hub_url;
use crate::error::{AppError, Result};
use crate::frame_edges::{give_frame_band_to_window, Band};
use crate::native_dock::{self, DockBounds, DockHolder, DockOwner};

/// Tab key a caller names when it does not send a tab id.
///
/// The frontend names every tab, so this only keeps an older caller working: such a call keeps
/// addressing the same single page it always did.
const DEFAULT_TAB_ID: &str = "default";

/// Distance from a window edge at which a dock rectangle still counts as covering it, in logical
/// pixels. A measurement is rounded, so an exact hit is not guaranteed.
const WINDOW_EDGE_REACH: f64 = 1.0;

/// State of the docked ChatHub tabs.
#[derive(Debug, Default)]
pub struct ChatHubPageState {
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    /// One entry per open tab, keyed by the tab id the frontend generated.
    tabs: std::collections::HashMap<String, Tab>,
    /// The tab currently shown, if any.
    active: Option<String>,
}

/// One open tab: its own webview, the holder that bounds it and the state it last showed.
struct Tab {
    webview: WebView,
    holder: DockHolder,
    /// Url the tab is currently showing.
    url: String,
    /// Rectangle the tab is currently placed at, in logical pixels.
    bounds: DockBounds,
}

impl std::fmt::Debug for Tab {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Tab").finish_non_exhaustive()
    }
}

// SAFETY: the webview and the GTK holder may only be touched on the main thread. The state keeps
// them solely so a tab can be navigated, resized and destroyed later, and every access happens on
// that thread: `run_on_page_thread` posts each operation there.
unsafe impl Send for Tab {}

impl ChatHubPageState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reveals the tab `tab_id` (or the default tab) at the measured rectangle.
    ///
    /// Every tab keeps its own webview, so switching between tabs keeps each site's session; the
    /// other tabs are hidden, never destroyed. `bounds` is the dock rectangle the frontend
    /// measured in logical pixels. A caller that sends only the legacy `width`/`top_inset` pair is
    /// laid out at the right edge of the window instead, so an older frontend keeps working.
    pub fn show(
        &self,
        app: &AppHandle<Wry>,
        url: &str,
        width: f64,
        top_inset: f64,
        corner_radius: f64,
        tab_id: Option<String>,
        bounds: Option<DockBounds>,
    ) -> Result<()> {
        let url = parse_chat_hub_url(url)?.to_string();
        let host = host_window(app)?;
        let (window_width, window_height) = native_dock::window_size(&host)?;
        let key = tab_id.unwrap_or_else(|| DEFAULT_TAB_ID.to_string());
        if key.is_empty() {
            return Err(AppError::General {
                message: "the ChatHub tab id is empty".to_string(),
            });
        }

        let bounds = bounds
            .unwrap_or_else(|| legacy_bounds(width, top_inset, window_width, window_height))
            .sanitize(window_width, window_height)?;

        let mut inner = self.lock()?;
        if !inner.tabs.contains_key(&key) {
            let holder = native_dock::with_surface(app, |surface| {
                surface.holder(app, DockOwner::ChatHub, bounds)
            })?;

            // The page is reused per tab, so building it is the only moment a proxy can be
            // applied: the settings are read here. The browsing profile is the same directory on
            // every tab, so the session survives a tab close and a restart.
            let mut web_context = WebContext::new(Some(page_data_directory(app)));

            // A page that gives a window corner back also has to be see-through. The shared corner
            // script rounds one window corner, which a page below a shared titlebar needs, so this
            // carrier leaves it out (`0.0`) and rounds the four corners itself: one implementation
            // covers them all, so they cannot drift apart.
            let radius = corner_radius.clamp(0.0, MAX_PAGE_CORNER_RADIUS);
            let mut builder = page_builder(&mut web_context, &url, page_proxy(app), 0.0, app);
            if radius > 0.0 {
                builder = builder
                    .with_transparent(true)
                    .with_initialization_script(page_corners_script(radius));
            }

            let webview = builder.build_gtk(holder.container())?;

            // The dock rectangle can reach a window edge, so that edge keeps its resize band with
            // the window instead of the page: this webview is built by wry, so the resize handler
            // tauri installs on a Tauri webview never reaches it.
            give_frame_band_to_window(
                &webview.webview(),
                band_for(bounds, window_width, window_height),
            );

            inner.tabs.insert(
                key.clone(),
                Tab {
                    webview,
                    holder,
                    url: url.clone(),
                    bounds,
                },
            );
        }

        // A single tab is visible at a time; the others are hidden without touching their
        // sessions. The loop also covers the tab that was just created.
        for (id, other) in inner.tabs.iter() {
            if id != &key {
                other.holder.set_visible(false);
            }
        }

        let tabs = &mut inner.tabs;
        let tab = tabs.get_mut(&key).ok_or_else(|| AppError::General {
            message: "the ChatHub tab is missing".to_string(),
        })?;

        if tab.url != url {
            tab.webview.load_url(&url)?;
            tab.url = url;
        }
        if tab.bounds != bounds {
            tab.holder.set_bounds(bounds);
            tab.bounds = bounds;
        }
        tab.holder.set_visible(true);
        inner.active = Some(key);

        Ok(())
    }

    /// Hides every tab without destroying it, so each site keeps its session.
    pub fn hide(&self, _app: &AppHandle<Wry>) -> Result<()> {
        let mut inner = self.lock()?;
        for tab in inner.tabs.values() {
            tab.holder.set_visible(false);
        }
        inner.active = None;

        Ok(())
    }

    /// Applies a new width to the active tab.
    ///
    /// A drag only changes how much of the dock the tab takes; the window itself is never
    /// resized. The rectangle keeps its top and height and stays anchored to the right edge of the
    /// window, which is the dock this command predates.
    pub fn set_width(&self, app: &AppHandle<Wry>, width: f64) -> Result<()> {
        let host = host_window(app)?;
        let (window_width, window_height) = native_dock::window_size(&host)?;

        let mut inner = self.lock()?;
        let Some(active) = inner.active.clone() else {
            return Ok(());
        };
        let Some(tab) = inner.tabs.get_mut(&active) else {
            return Ok(());
        };

        let width = clamp_width(window_width, width);
        let bounds = DockBounds {
            x: (window_width - width).max(0.0),
            y: tab.bounds.y,
            width,
            height: tab.bounds.height,
        }
        .sanitize(window_width, window_height)?;

        tab.holder.set_bounds(bounds);
        tab.bounds = bounds;

        Ok(())
    }

    /// Reloads `tab_id`, keeping the tab's webview, its session and its current page.
    pub fn reload(&self, _app: &AppHandle<Wry>, tab_id: &str) -> Result<()> {
        let inner = self.lock()?;
        let tab = inner.tabs.get(tab_id).ok_or_else(|| AppError::General {
            message: format!("the ChatHub tab '{tab_id}' is not open"),
        })?;

        tab.webview.reload()?;

        Ok(())
    }

    /// Closes one tab, or every tab when no tab id is given.
    ///
    /// Only an explicit close reaches this, so switching between tabs or hiding the dock keeps
    /// the site sessions alive.
    pub fn destroy(&self, app: &AppHandle<Wry>, tab_id: Option<String>) -> Result<()> {
        match tab_id {
            Some(key) => {
                let removed = {
                    let mut inner = self.lock()?;
                    let removed = inner.tabs.remove(&key);
                    if inner.active.as_deref() == Some(key.as_str()) {
                        inner.active = None;
                    }
                    removed
                };

                if let Some(tab) = removed {
                    native_dock::with_surface(app, |surface| {
                        surface.remove(app, &tab.holder);
                        Ok(())
                    })?;
                }
            }
            None => self.close_all(app)?,
        }

        Ok(())
    }

    /// Closes every tab and removes every holder it owns.
    pub fn close_all(&self, app: &AppHandle<Wry>) -> Result<()> {
        {
            let mut inner = self.lock()?;
            inner.tabs.clear();
            inner.active = None;
        }

        native_dock::with_surface(app, |surface| {
            surface.remove_owner(app, DockOwner::ChatHub);
            Ok(())
        })
    }

    /// Nothing to do on this platform: the dock rectangle comes from the frontend's own layout and
    /// is applied when a tab is shown, so a window resize does not move a native view from here.
    pub fn sync_bounds(&self, _app: &AppHandle<Wry>, _reported: PhysicalSize<u32>) -> Result<()> {
        Ok(())
    }

    /// Drops every tab handle when the window it was docked to goes away.
    ///
    /// The window is going away with the tabs, so the webviews are destroyed with it and only the
    /// handles are released here; the shared dock surface is reset by the window-destroy cleanup.
    pub fn release(&self, _app: &AppHandle<Wry>) {
        self.forget();
    }

    /// Clears tracked state without touching handles that are already gone.
    pub fn forget(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.tabs.clear();
            inner.active = None;
        }
    }

    /// Width the dock took from the host window, in logical pixels.
    ///
    /// The dock no longer takes width from the window: the frontend reserves it in its own layout,
    /// so no independent window resize is reported here.
    pub fn grown_width(&self) -> f64 {
        0.0
    }

    /// Locks the state, reporting a poisoned lock as a plain error instead of a panic.
    fn lock(&self) -> Result<MutexGuard<'_, Inner>> {
        self.inner.lock().map_err(|_| AppError::General {
            message: "the ChatHub page state is poisoned".to_string(),
        })
    }
}

/// Largest corner radius the page accepts, so a bad measurement cannot eat into the page.
///
/// It is the bound the shared corner script applies, kept the same here so a radius that reaches
/// the window is treated the same way on every carrier.
const MAX_PAGE_CORNER_RADIUS: f64 = 30.0;

/// Rectangle a legacy caller's `width`/`top_inset` pair describes.
///
/// The pair names the docked page of an older frontend, which owned the right edge of the window
/// below the app chrome; it is turned into the same rectangle the frontend now measures itself.
fn legacy_bounds(width: f64, top_inset: f64, window_width: f64, window_height: f64) -> DockBounds {
    let width = clamp_width(window_width, width);
    let top = top_inset.clamp(0.0, window_height);

    DockBounds {
        x: (window_width - width).max(0.0),
        y: top,
        width,
        height: (window_height - top).max(1.0),
    }
}

/// Frame band the dock gives back for the window edges its rectangle reaches.
///
/// A dock that reaches a window edge covers the band the window level resize handler accepts, so
/// that edge has to stay with the window; an edge inside the window borders the workflow UI and
/// keeps its input. The common dock spans the right edge of the window from top to bottom, which
/// is exactly the band a right column gives back, so the named bands are reused for the shapes
/// that occur.
fn band_for(bounds: DockBounds, window_width: f64, window_height: f64) -> Band {
    let reaches_left = bounds.x <= WINDOW_EDGE_REACH;
    let reaches_top = bounds.y <= WINDOW_EDGE_REACH;
    let reaches_right = bounds.x + bounds.width >= window_width - WINDOW_EDGE_REACH;
    let reaches_bottom = bounds.y + bounds.height >= window_height - WINDOW_EDGE_REACH;

    match (reaches_left, reaches_top, reaches_right, reaches_bottom) {
        (false, true, true, true) => Band::RIGHT_COLUMN,
        (true, true, true, true) => Band::EVERY_SIDE,
        (left, top, right, bottom) => Band {
            left,
            top,
            right,
            bottom,
        },
    }
}

/// JavaScript descriptor of one corner of the page, as [`page_corners_script`] reads it.
///
/// `property` is the border radius the script applies to the elements that paint the corner,
/// `attribute` marks a host whose pseudo element paints it, and `x` and `y` select the viewport
/// point the corner sits at.
fn corner_descriptor(property: &str, attribute: &str, x: &str, y: &str, radius: f64) -> String {
    let declarations = format!("{property}:{radius}px !important");

    format!(
        "{{property: '{property}', attribute: '{attribute}', x: '{x}', y: '{y}', \
         rule: '[{attribute}]::before,[{attribute}]::after{{{declarations}}}'}}"
    )
}

/// Script that leaves the four corners of the page unpainted.
///
/// The page is a rectangle placed over the workflow UI, so it paints over whatever corner the
/// window rounds where its rectangle reaches one. Every one of its four corners therefore has to
/// come back from the page itself. The shared corner script ([`super::page`]) rounds one window
/// corner, which is what a page below a shared titlebar needs, so this carrier leaves it out and
/// rounds the four corners here: one implementation covers them all, so the corners cannot drift
/// apart.
///
/// The rules are the ones that hold in the shared script:
///
/// - A corner is given back by every element that paints it. A decorative layer carries
///   `pointer-events: none`, so it never shows up in a hit test and the document is inspected by
///   geometry instead: an element is rounded when it covers the corner point of the viewport and
///   paints something there. A box shadow is painting there as well, and it follows the radius of
///   its host, so a host that only throws a shadow over the corner is rounded too.
/// - A pseudo element paints a box of its own, which the radius of its host does not cut, so a
///   host that paints a corner through one is marked and the corner reaches it through a rule.
/// - The corners are applied again for a short while after the page load, because a site can
///   build the layer that paints them later than the load event.
///
/// All four corners are inspected in one pass over the document, so they cost a single walk of
/// the tree rather than four.
fn page_corners_script(radius: f64) -> String {
    let page_corners = [
        ("border-top-left-radius", "data-cs-top-left", "left", "top"),
        (
            "border-top-right-radius",
            "data-cs-top-right",
            "right",
            "top",
        ),
        (
            "border-bottom-left-radius",
            "data-cs-bottom-left",
            "left",
            "bottom",
        ),
        (
            "border-bottom-right-radius",
            "data-cs-bottom-right",
            "right",
            "bottom",
        ),
    ];
    let mut descriptors = Vec::new();

    for (property, attribute, x, y) in page_corners {
        descriptors.push(corner_descriptor(property, attribute, x, y, radius));
    }

    let corners_js = descriptors.join(",\n    ");

    format!(
        r#"(function () {{
  var radius = '{radius}px';
  var corners = [{corners_js}];
  var transparent = 'rgba(0, 0, 0, 0)';
  var pending = 0;
  var ruled = false;
  function opaque(background) {{
    return !!background && background !== 'transparent' && background !== transparent;
  }}
  function paints(style) {{
    return style.backgroundImage !== 'none' || style.boxShadow !== 'none'
      || opaque(style.backgroundColor);
  }}
  function replaced(element) {{
    var tag = element.tagName;
    return tag === 'IMG' || tag === 'CANVAS' || tag === 'VIDEO' || tag === 'IFRAME'
      || tag === 'SVG' || tag === 'OBJECT' || tag === 'EMBED';
  }}
  function paintsCorner(element, style) {{
    return paints(style) || replaced(element);
  }}
  function paintsPseudo(element) {{
    for (var part = 0; part < 2; part += 1) {{
      var pseudo = window.getComputedStyle(element, part ? '::after' : '::before');
      if (pseudo.content && pseudo.content !== 'none' && paints(pseudo)) {{
        return true;
      }}
    }}
    return false;
  }}
  function insertRule(rule) {{
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
  function addCornerRules() {{
    if (ruled) {{
      return;
    }}
    ruled = true;
    for (var index = 0; index < corners.length; index += 1) {{
      insertRule(corners[index].rule);
    }}
  }}
  function coversCorner(rect, point) {{
    return rect.left <= point.x && rect.top <= point.y
      && rect.right >= point.x && rect.bottom >= point.y;
  }}
  function roundCorners() {{
    var points = [];
    var targets = [];
    var index;
    for (index = 0; index < corners.length; index += 1) {{
      points.push({{
        x: corners[index].x === 'right' ? window.innerWidth - 2 : 2,
        y: corners[index].y === 'bottom' ? window.innerHeight - 2 : 2
      }});
      targets.push([]);
    }}
    var elements = document.querySelectorAll('*');
    for (var elementIndex = 0; elementIndex < elements.length; elementIndex += 1) {{
      var element = elements[elementIndex];
      var rect = element.getBoundingClientRect();
      if (rect.width < 1 || rect.height < 1) {{
        continue;
      }}
      var covered = false;
      for (index = 0; index < corners.length; index += 1) {{
        if (coversCorner(rect, points[index])) {{
          covered = true;
          break;
        }}
      }}
      if (!covered) {{
        continue;
      }}
      var style = window.getComputedStyle(element);
      if (style.display === 'none' || style.visibility === 'hidden'
        || Number(style.opacity) === 0) {{
        continue;
      }}
      var painted = paintsCorner(element, style);
      var pseudoPainted = false;
      for (index = 0; index < corners.length; index += 1) {{
        if (!coversCorner(rect, points[index])) {{
          continue;
        }}
        if (painted) {{
          targets[index].push(element);
        }}
        if (!pseudoPainted) {{
          pseudoPainted = paintsPseudo(element);
        }}
        if (pseudoPainted) {{
          element.setAttribute(corners[index].attribute, '');
          addCornerRules();
        }}
      }}
    }}
    for (index = 0; index < corners.length; index += 1) {{
      for (var target = 0; target < targets[index].length; target += 1) {{
        targets[index][target].style.setProperty(corners[index].property, radius, 'important');
      }}
      document.documentElement.style.setProperty(corners[index].property, radius, 'important');
    }}
  }}
  roundCorners();
  document.addEventListener('DOMContentLoaded', roundCorners);
  window.addEventListener('load', roundCorners);
  window.addEventListener('resize', function () {{
    if (pending) {{
      return;
    }}
    pending = window.setTimeout(function () {{
      pending = 0;
      roundCorners();
    }}, 200);
  }});
  var attempts = 0;
  var retry = window.setInterval(function () {{
    attempts += 1;
    if (attempts > 15) {{
      window.clearInterval(retry);
      return;
    }}
    roundCorners();
  }}, 400);
}})();"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Source of one implementation block, so an assertion can never match its own text
    /// instead of the code it guards.
    fn implementation<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
        source
            .split(start)
            .nth(1)
            .expect("the implementation block is missing")
            .split(end)
            .next()
            .expect("the implementation block is not terminated")
    }

    /// The production half of this file, so an assertion can never match its own text.
    fn production_source() -> &'static str {
        include_str!("gtk_panel.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("gtk_panel.rs carries a test module")
    }

    /// Guard for the multi-tab model: every tab keeps its own webview, and switching between tabs
    /// hides the others instead of destroying them.
    #[test]
    fn every_tab_keeps_its_own_webview_while_another_is_shown() {
        let source = include_str!("gtk_panel.rs");
        let show = implementation(source, "pub fn show", "pub fn hide");
        let hide = source
            .split("pub fn hide")
            .nth(1)
            .expect("the hide path is missing")
            .split("pub fn set_width")
            .next()
            .expect("the hide path is not terminated");

        // One tab is created per id and kept in the map.
        assert!(show.contains("if !inner.tabs.contains_key(&key)"));
        assert!(show.contains("inner.tabs.insert("));
        // Every other tab is only hidden.
        assert!(show.contains("other.holder.set_visible(false)"));
        // Hiding the dock keeps every webview.
        assert!(hide.contains("tab.holder.set_visible(false)"));
        assert!(!hide.contains("tabs.clear"));
    }

    /// Guard for the geometry contract: the rectangle comes from the frontend and no carrier
    /// changes the window layout.
    #[test]
    fn the_dock_rectangle_comes_from_the_frontend() {
        let source = production_source();

        assert!(source.contains("native_dock::window_size(&host)"));
        assert!(source.contains(".sanitize(window_width, window_height)?"));
        // An older caller that sends only a width still gets a rectangle.
        assert!(source.contains("legacy_bounds(width, top_inset, window_width, window_height)"));
        // Nothing widens, narrows or splits the window any more.
        let widening = format!("{}{}", "widen_", "host_window");
        let narrowing = format!("{}{}", "narrow_", "host_window");
        assert!(!source.contains(&widening));
        assert!(!source.contains(&narrowing));
        assert!(!source.contains("set_orientation"));
        assert!(!source.contains("notebook"));
    }

    /// Guard for the input path: a tab is a bounded holder of the shared dock surface, never a
    /// full-area container that would swallow the workflow UI's pointer events.
    #[test]
    fn a_tab_is_a_bounded_holder_of_the_dock_surface() {
        let source = production_source();

        assert!(source.contains("surface.holder(app, DockOwner::ChatHub, bounds)"));
        assert!(source.contains("builder.build_gtk(holder.container())"));
        let fixed_container_use = format!("{}{}", "Fixed::", "new");
        assert!(!source.contains(&fixed_container_use));
        assert!(!source.contains("add_overlay"));
    }

    /// Guard for the isolation boundary: the page is built by wry, so it gets no Tauri IPC and
    /// cannot reach a ChatSpeed command.
    #[test]
    fn the_page_is_built_by_wry_without_tauri_ipc() {
        let source = include_str!("gtk_panel.rs");

        assert!(source.contains("build_gtk(holder.container())"));
        assert!(!source.contains(concat!("Webview", "Builder")));
    }

    /// Guard for the frame band: a dock that reaches a window edge gives that edge back to the
    /// window, so a drag on the frame still resizes the window.
    #[test]
    fn a_dock_that_reaches_a_window_edge_gives_it_back() {
        let source = include_str!("gtk_panel.rs");

        assert!(source.contains("give_frame_band_to_window("));
        assert!(source.contains("band_for(bounds, window_width, window_height)"));

        let band = band_for(
            DockBounds {
                x: 400.0,
                y: 40.0,
                width: 600.0,
                height: 700.0,
            },
            1000.0,
            740.0,
        );
        assert!(!band.left);
        assert!(!band.top);
        assert!(band.right);
        assert!(band.bottom);

        // The common dock spans the right edge from top to bottom, which is the right column band.
        let dock = band_for(
            DockBounds {
                x: 400.0,
                y: 0.0,
                width: 600.0,
                height: 740.0,
            },
            1000.0,
            740.0,
        );
        assert!(!dock.left);
        assert!(dock.top);
        assert!(dock.right);
        assert!(dock.bottom);
    }

    /// Guard for the legacy rectangle: a caller that only sends a width still lands on the right
    /// edge of the window, below the app chrome.
    #[test]
    fn a_width_only_call_lands_on_the_right_edge_of_the_window() {
        assert_eq!(
            legacy_bounds(600.0, 40.0, 1600.0, 900.0),
            DockBounds {
                x: 1000.0,
                y: 40.0,
                width: 600.0,
                height: 860.0,
            }
        );
    }

    /// Guard for the corners the page gives back: the rectangle paints over the window corners it
    /// reaches, so all four of its corners are rounded here, by this carrier alone.
    #[test]
    fn the_page_gives_back_all_four_of_its_corners() {
        let script = page_corners_script(15.0);

        for property in [
            "border-top-left-radius",
            "border-top-right-radius",
            "border-bottom-left-radius",
            "border-bottom-right-radius",
        ] {
            assert!(script.contains(&format!("property: '{property}'")));
            assert!(script.contains(&format!("{property}:15px !important")));
        }

        // A corner painted through a box shadow or through replaced content is found as well.
        assert!(script.contains("style.boxShadow !== 'none'"));
        assert!(script.contains("tag === 'IMG'"));

        // The script reaches the page as JavaScript, so no formatting brace may survive in it.
        assert!(!script.contains("{{"));
        assert!(!script.contains("}}"));

        // The shared corner script rounds one window corner, which is not what the page needs here,
        // so this carrier leaves it out and rounds the four corners itself.
        let source = include_str!("gtk_panel.rs");
        assert!(source.contains("page_builder(&mut web_context, &url, page_proxy(app), 0.0, app)"));
        assert!(source.contains(".with_initialization_script(page_corners_script(radius))"));
    }
}
