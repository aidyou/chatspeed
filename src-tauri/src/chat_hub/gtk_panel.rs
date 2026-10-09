//! Linux carrier of the ChatHub tabs.
//!
//! Every ChatHub entry has its own tab, each tab keeps its own `wry` webview (with its own
//! browsing session) while another tab is shown, and every webview is placed at the rectangle
//! the frontend measured for the right dock. The rectangle is the only geometry the carrier
//! knows: how much room the window makes for that dock is decided once, for every platform, by
//! [`crate::dock_window`], and never by a carrier.
//!
//! The native views are bounded overlay children of the shared dock surface
//! ([`crate::native_dock`]): a holder aligned to the start of the overlay with the rectangle as
//! its margins and size request. A full-area container would swallow every pointer event of the
//! window instead, which is what kept the workflow UI from receiving clicks while a native view
//! was open, so the carrier only ever creates bounded holders.

use std::rc::Rc;
use std::sync::{Mutex, MutexGuard};

use tauri::{AppHandle, PhysicalSize, Wry};
use wry::{WebContext, WebView, WebViewBuilderExtUnix, WebViewExtUnix};

use super::{clamp_width, host_window, page_builder, page_data_directory, page_proxy};
use crate::db::chat_hub::parse_chat_hub_url;
use crate::error::{AppError, Result};
use crate::native_dock::{self, DockBounds, DockHolder, DockOwner, ViewShape};

/// Tab key a caller names when it does not send a tab id.
///
/// The frontend names every tab, so this only keeps an older caller working: such a call keeps
/// addressing the same single page it always did.
const DEFAULT_TAB_ID: &str = "default";

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
    /// Main-thread state that reshapes the webview for the band and radius it is shown at.
    shape: Rc<ViewShape>,
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
        let radius = native_dock::clamp_corner_radius(corner_radius);
        // The band follows the rectangle the frontend measured, and both dock owners compute it
        // through the same rule.
        let band = native_dock::band_for(bounds, window_width, window_height);
        // The dock is a column on the right edge of the window, so its bottom-right corner is the
        // window's own: the shape is cut into the window, which keeps the rounded frame the window
        // draws visible under a view that would otherwise paint over it.
        native_dock::with_surface(app, |surface| surface.set_corner_radius(app, radius))?;

        let mut inner = self.lock()?;
        if !inner.tabs.contains_key(&key) {
            let holder = native_dock::with_surface(app, |surface| {
                surface.holder(app, DockOwner::ChatHub, bounds)
            })?;

            // The page is reused per tab, so building it is the only moment a proxy can be
            // applied: the settings are read here. The browsing profile is the same directory on
            // every tab, so the session survives a tab close and a restart.
            let mut web_context = WebContext::new(Some(page_data_directory(app)));
            let webview = page_builder(&mut web_context, &url, page_proxy(app), app)
                .build_gtk(holder.container())?;

            // The native view is reshaped from this state on every allocation, so a later show
            // that only moves the rectangle or changes the radius updates the state instead of
            // connecting a second callback or rebuilding the webview.
            let shape = ViewShape::install(&webview.webview());
            shape.set(radius, band);

            inner.tabs.insert(
                key.clone(),
                Tab {
                    webview,
                    holder,
                    shape,
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

        // The rectangle and the radius both feed the same reshape state, so a reused tab follows
        // whatever the frontend resent instead of the geometry it was first built with.
        tab.shape.set(radius, band);

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

    /// Locks the state, reporting a poisoned lock as a plain error instead of a panic.
    fn lock(&self) -> Result<MutexGuard<'_, Inner>> {
        self.inner.lock().map_err(|_| AppError::General {
            message: "the ChatHub page state is poisoned".to_string(),
        })
    }
}

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
            .split("pub fn reload")
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
        // Making room for the dock belongs to the shared window state, so this carrier never widens,
        // narrows or splits the window itself.
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
        assert!(source.contains(".build_gtk(holder.container())"));
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
    /// window, so a drag on the frame still resizes the window. The band comes from the rule the two
    /// dock owners share, not from a copy local to this carrier.
    #[test]
    fn a_dock_that_reaches_a_window_edge_gives_it_back() {
        let source = include_str!("gtk_panel.rs");

        assert!(source.contains("native_dock::band_for(bounds, window_width, window_height)"));
        assert!(source.contains("shape.set(radius, band)"));

        // The common dock spans the right edge from top to bottom, which is the right column band.
        let dock = native_dock::band_for(
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

    /// A reused tab follows the radius and the rectangle the frontend resent instead of the geometry
    /// it was first built with, and neither change rebuilds the webview or its session.
    #[test]
    fn a_reused_tab_follows_the_resent_radius_and_rectangle() {
        let show = implementation(include_str!("gtk_panel.rs"), "pub fn show", "pub fn hide");

        // The webview and its page are built only when the tab does not exist yet.
        let build = format!("{}{}", "build_", "gtk");
        assert_eq!(show.matches(&build).count(), 1);
        // Every show re-applies the current radius and the current band to the shared state.
        assert!(show.contains("tab.shape.set(radius, band)"));
        // A radius change never reaches the page: the corner is a property of the window, so a
        // reused tab changes no document and drops no session.
        assert!(!show.contains("evaluate_script"));
        assert!(!show.contains("reload"));
    }

    /// Guard for the corner: the carrier cuts it into the window, because the dock is a column on
    /// the window's right edge and a rectangular view would otherwise paint over the rounded frame.
    #[test]
    fn the_carrier_cuts_the_window_corner_out_of_the_dock() {
        let source = production_source();

        assert!(source.contains("native_dock::clamp_corner_radius(corner_radius)"));
        // The shape belongs to the window, so it is applied through the shared surface and never
        // through a script or a see-through page.
        assert!(source.contains("surface.set_corner_radius(app, radius)"));
        assert!(source.contains("page_builder(&mut web_context, &url, page_proxy(app), app)"));
        assert!(!source.contains("dock_page_builder"));
        assert!(!source.contains("bottom_right_corner"));
        assert!(!source.contains("supports_native_rounding"));
        // The native view is still reshaped through the shared state, which carries the band as
        // well.
        assert!(source.contains("ViewShape::install(&webview.webview())"));
        // The obsolete four-corner script is gone: only the shared bottom-right corner is rounded.
        let four_corner = format!("{}{}", "page_corners_", "script");
        assert!(!source.contains(&four_corner));
        assert!(!source.contains("border-top-left-radius"));
    }
}
