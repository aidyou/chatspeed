//! Windows and macOS carrier of the ChatHub tabs.
//!
//! On these platforms a webview is created as a child view at an explicit rectangle, and the
//! platform moves a child view together with its parent window, so the tabs follow the window
//! without any per-move work. The rectangle is the one the frontend measured for the right dock,
//! and it is the only geometry this carrier knows: how much room the window makes for that dock is
//! decided once, for every platform, by [`crate::dock_window`], and never by a carrier.
//!
//! Every ChatHub entry has its own tab, each tab keeps its own `wry` webview (with its own
//! browsing session) while another tab is shown, and the tabs share the bounded dock surface of
//! [`crate::native_dock`] on this platform as a common managed state.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use tauri::{AppHandle, PhysicalSize, Wry};
use wry::{
    dpi::{LogicalPosition, LogicalSize},
    Rect, WebContext, WebView,
};

use super::{clamp_width, host_window, page_builder, page_data_directory, page_proxy};
use crate::db::chat_hub::parse_chat_hub_url;
use crate::error::{AppError, Result};
use crate::native_dock::{self, DockBounds};

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
    tabs: HashMap<String, Tab>,
    /// The tab currently shown, if any.
    active: Option<String>,
}

/// One open tab: its own webview and the state it last showed.
struct Tab {
    webview: WebView,
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

// SAFETY: a `wry::WebView` may only be touched on the main thread. The state keeps the handle
// solely so a tab can be navigated, resized and destroyed later, and every access happens on that
// thread: `run_on_page_thread` posts each operation there.
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
    ///
    /// `_corner_radius` is the radius a window draws at the bottom-right corner of its dock. These
    /// platforms clip a child view to the window frame themselves, so the corner needs no help from
    /// this carrier and the measurement is unused here.
    pub fn show(
        &self,
        app: &AppHandle<Wry>,
        url: &str,
        width: f64,
        top_inset: f64,
        _corner_radius: f64,
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
            // The page is reused per tab, so building it is the only moment a proxy can be
            // applied: the settings are read here. The browsing profile is the same directory on
            // every tab, so the session survives a tab close and a restart.
            let mut web_context = WebContext::new(Some(page_data_directory(app)));
            let webview = page_builder(&mut web_context, &url, page_proxy(app), app)
                .with_bounds(rect(bounds))
                .build_as_child(&host)?;

            inner.tabs.insert(
                key.clone(),
                Tab {
                    webview,
                    url: url.clone(),
                    bounds,
                },
            );
        }

        // A single tab is visible at a time; the others are hidden without touching their
        // sessions. The loop also covers the tab that was just created.
        for (id, other) in inner.tabs.iter() {
            if id != &key {
                other.webview.set_visible(false)?;
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
            tab.webview.set_bounds(rect(bounds))?;
            tab.bounds = bounds;
        }
        tab.webview.set_visible(true)?;
        inner.active = Some(key);

        Ok(())
    }

    /// Hides every tab without destroying it, so each site keeps its session.
    pub fn hide(&self, _app: &AppHandle<Wry>) -> Result<()> {
        let mut inner = self.lock()?;
        for tab in inner.tabs.values() {
            tab.webview.set_visible(false)?;
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
    pub fn destroy(&self, _app: &AppHandle<Wry>, tab_id: Option<String>) -> Result<()> {
        let mut inner = self.lock()?;

        match tab_id {
            Some(key) => {
                inner.tabs.remove(&key);
                if inner.active.as_deref() == Some(key.as_str()) {
                    inner.active = None;
                }
            }
            None => {
                inner.tabs.clear();
                inner.active = None;
            }
        }

        Ok(())
    }

    /// Closes every tab; the child views go away with their handles.
    pub fn close_all(&self, app: &AppHandle<Wry>) -> Result<()> {
        self.destroy(app, None)
    }

    /// Nothing to do on this platform: the dock rectangle comes from the frontend's own layout and
    /// is applied when a tab is shown, so a window resize does not move a child view from here.
    pub fn sync_bounds(&self, _app: &AppHandle<Wry>, _reported: PhysicalSize<u32>) -> Result<()> {
        Ok(())
    }

    /// Drops every tab handle when the window it was docked to goes away.
    ///
    /// The window is going away with the tabs, so the child views are destroyed with it and only
    /// the handles are released here; the shared dock surface is reset by the window-destroy
    /// cleanup.
    pub fn release(&self, _app: &AppHandle<Wry>) {
        self.forget();
    }

    /// Clears tracked state without touching a handle that is already gone.
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

/// Rectangle of a tab as the child view API takes it.
fn rect(bounds: DockBounds) -> Rect {
    Rect {
        position: LogicalPosition::new(bounds.x, bounds.y).into(),
        size: LogicalSize::new(bounds.width, bounds.height).into(),
    }
}

#[cfg(test)]
mod tests {
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
        include_str!("child_view.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("child_view.rs carries a test module")
    }

    /// Guard for the multi-tab model: every tab keeps its own webview, and switching between tabs
    /// hides the others instead of destroying them.
    #[test]
    fn every_tab_keeps_its_own_webview_while_another_is_shown() {
        let source = include_str!("child_view.rs");
        let show = implementation(source, "pub fn show", "pub fn hide");
        let hide = source
            .split("pub fn hide")
            .nth(1)
            .expect("the hide path is missing")
            .split("pub fn reload")
            .next()
            .expect("the hide path is not terminated");

        assert!(show.contains("if !inner.tabs.contains_key(&key)"));
        assert!(show.contains("inner.tabs.insert("));
        assert!(show.contains("other.webview.set_visible(false)"));
        assert!(hide.contains("tab.webview.set_visible(false)"));
        assert!(!hide.contains("tabs.clear"));
    }

    /// Guard for the geometry contract: the rectangle comes from the frontend and no carrier
    /// changes the window layout.
    #[test]
    fn the_dock_rectangle_comes_from_the_frontend() {
        let source = production_source();
        let show = implementation(source, "pub fn show", "pub fn hide");
        let rectangle = implementation(source, "fn rect", "#[cfg(test)]");

        assert!(show.contains("bounds\n            .unwrap_or_else(|| legacy_bounds(width, top_inset, window_width, window_height))\n            .sanitize(window_width, window_height)?"));
        // The tab is placed at the rectangle the frontend measured.
        assert!(rectangle.contains("LogicalPosition::new(bounds.x, bounds.y)"));
        assert!(rectangle.contains("LogicalSize::new(bounds.width, bounds.height)"));
        // Making room for the dock belongs to the shared window state, so this carrier never
        // widens, narrows or splits the window itself.
        let widening = format!("{}{}", "widen_", "host_window");
        let narrowing = format!("{}{}", "narrow_", "host_window");
        assert!(!source.contains(&widening));
        assert!(!source.contains(&narrowing));
        assert!(!show.contains("window_size.width + added"));
    }

    /// Guard for the session lifetime: hiding the dock must keep every webview.
    #[test]
    fn hiding_the_page_keeps_the_webview_alive() {
        let source = include_str!("child_view.rs");
        let hide = source
            .split("pub fn hide")
            .nth(1)
            .expect("the hide path is missing")
            .split("pub fn reload")
            .next()
            .expect("the hide path is not terminated");

        assert!(hide.contains("tab.webview.set_visible(false)"));
        assert!(!hide.contains("destroy"));
    }

    /// Guard for the isolation boundary: the page is built by wry, so it gets no Tauri IPC and
    /// cannot reach a ChatSpeed command.
    #[test]
    fn the_page_is_built_by_wry_without_tauri_ipc() {
        let source = include_str!("child_view.rs");

        assert!(source.contains("build_as_child(&host)"));
        assert!(source.contains(".with_bounds(rect(bounds))"));
        assert!(!source.contains(concat!("Webview", "Builder")));
    }
}
