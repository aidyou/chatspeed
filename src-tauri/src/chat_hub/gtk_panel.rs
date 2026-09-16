//! Linux carrier of the ChatHub page.
//!
//! GTK does the layout here, which is why this platform has no geometry to track at
//! all: the window box is turned horizontal, the page webview is packed into a fixed
//! width column at its end, and the workflow webview keeps the remaining space. GTK
//! then moves and resizes the page together with the window itself, so the page can
//! never trail the window, and there is no separate top level window for the desktop
//! compositor to decorate with a shadow.
//!
//! The container is deliberately a [`gtk::Box`] and not a `gtk::Fixed`: a fixed
//! container positions a webview by absolute coordinates, which would stack the page
//! *over* the workflow UI instead of giving it its own space.

use std::sync::Mutex;

use gtk::prelude::*;
use tauri::{AppHandle, WebviewWindow, Wry};
use wry::{WebContext, WebView, WebViewBuilderExtUnix};

use super::{clamp_width, host_window, page_builder, page_data_directory, page_proxy};
use crate::db::chat_hub::parse_chat_hub_url;
use crate::error::{AppError, Result};

/// State of the single docked ChatHub page.
#[derive(Debug, Default)]
pub struct ChatHubPageState {
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    page: Option<Page>,
    /// Url the page is currently showing.
    url: Option<String>,
    /// Width currently applied to the page column.
    width: Option<f64>,
}

/// The native handles of the docked page.
struct Page {
    webview: WebView,
    /// Column the page lives in. It is a sibling of the workflow webview, so changing
    /// its size request is what gives the page its width.
    column: gtk::Box,
}

impl std::fmt::Debug for Page {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Page").finish_non_exhaustive()
    }
}

// SAFETY: both fields are GTK handles that may only be touched on the main thread.
// The state keeps them solely so the page can be navigated, resized and destroyed
// later, and every access happens on that thread: `run_on_page_thread` posts each
// operation there, and the window event that releases the page already runs there.
unsafe impl Send for Page {}

impl ChatHubPageState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reveals the page, docked to the right edge of the workflow window.
    ///
    /// The same webview is reused for every ChatHub entry, so the site keeps its
    /// cookies and session while navigating between entries. `_top_inset` is only used
    /// by carriers that stack the page over the workflow UI: here the page is a sibling
    /// of the workflow webview, which already contains the app titlebar itself.
    pub fn show(
        &self,
        app: &AppHandle<Wry>,
        url: &str,
        width: f64,
        _top_inset: f64,
    ) -> Result<()> {
        let url = parse_chat_hub_url(url)?.to_string();
        let host = host_window(app)?;
        let width = clamp_width(host_inner_width(&host)?, width);

        let mut inner = self.inner.lock()?;

        if inner.page.is_none() {
            inner.page = Some(Page::create(app, &host, &url, width)?);
            inner.url = Some(url.clone());
            #[cfg(debug_assertions)]
            log::info!(
                "[ChatHub] docked the page at {:.0} logical pixels in the '{}' window",
                width,
                super::CHAT_HUB_HOST_WINDOW_LABEL
            );
        }

        let page = inner.page.as_ref().ok_or_else(|| AppError::General {
            message: "the ChatHub page column is missing".to_string(),
        })?;

        if inner.url.as_deref() != Some(url.as_str()) {
            page.webview.load_url(&url)?;
        }

        // The column is hidden while no entry is open; showing it again keeps the
        // current page and its session.
        page.set_width(width);
        page.column.show_all();

        inner.url = Some(url);
        inner.width = Some(width);

        Ok(())
    }

    /// Hides the page without destroying it, so its session survives.
    ///
    /// A hidden child is skipped by the window box, so the workflow UI gets the full
    /// window width back.
    pub fn hide(&self, _app: &AppHandle<Wry>) -> Result<()> {
        let inner = self.inner.lock()?;
        if let Some(page) = inner.page.as_ref() {
            page.column.hide();
        }
        Ok(())
    }

    /// Applies a new width to the docked page.
    pub fn set_width(&self, app: &AppHandle<Wry>, width: f64) -> Result<()> {
        let host = host_window(app)?;
        let width = clamp_width(host_inner_width(&host)?, width);

        let mut inner = self.inner.lock()?;
        if let Some(page) = inner.page.as_ref() {
            page.set_width(width);
        }
        inner.width = Some(width);

        Ok(())
    }

    /// Nothing to do on this platform: GTK resizes the page column with the window
    /// itself, so only a width the user asked for has to be applied explicitly.
    pub fn sync_bounds(&self, _app: &AppHandle<Wry>) -> Result<()> {
        Ok(())
    }

    /// Destroys the page and clears every tracked view field.
    ///
    /// Only the explicit close action and application exit reach this, so switching
    /// entries or hiding the page keeps the site session alive.
    pub fn destroy(&self, _app: &AppHandle<Wry>) -> Result<()> {
        let mut inner = self.inner.lock()?;

        if let Some(page) = inner.page.take() {
            // Hiding first gives the workflow UI its full width back before the widgets
            // go away; dropping the webview is what destroys the embedded page.
            page.column.hide();
            drop(page.webview);

            // The column is the last reference of its own widget, so detaching it from
            // the window box and dropping the handle releases it as well.
            if let Some(parent) = page.column.parent() {
                if let Ok(container) = parent.downcast::<gtk::Container>() {
                    container.remove(&page.column);
                }
            }
        }

        inner.url = None;
        inner.width = None;

        Ok(())
    }

    /// Clears tracked state without touching handles that are already gone.
    pub fn forget(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.page = None;
            inner.url = None;
            inner.width = None;
        }
    }

    /// Releases the page together with the window it was docked to.
    pub fn release(&self, app: &AppHandle<Wry>) {
        if let Err(error) = self.destroy(app) {
            log::warn!("Failed to release the ChatHub page: {}", error);
        }
        self.forget();
    }
}

impl Page {
    /// Creates the page column inside the workflow window and builds the page in it.
    fn create(app: &AppHandle<Wry>, host: &WebviewWindow<Wry>, url: &str, width: f64) -> Result<Self> {
        let window_box = host.default_vbox()?;

        // The workflow webview and the page are siblings in the window box, so the box
        // has to lay them out horizontally: the workflow webview then keeps the
        // remaining space and the page owns its column on the right edge.
        window_box.set_orientation(gtk::Orientation::Horizontal);

        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        window_box.pack_start(&column, false, false, 0);
        column.set_size_request(width.round() as i32, -1);

        // The page is reused for every entry, so building it is the only moment a proxy
        // can be applied: the settings are read here.
        let mut web_context = WebContext::new(Some(page_data_directory(app)));
        let webview = page_builder(&mut web_context, url, page_proxy(app)).build_gtk(&column)?;

        Ok(Self { webview, column })
    }

    /// Applies a width to the page column.
    fn set_width(&self, width: f64) {
        self.column.set_size_request(width.round() as i32, -1);
    }
}

/// Logical width of the host window client area.
fn host_inner_width(host: &WebviewWindow<Wry>) -> Result<f64> {
    let scale_factor = host.scale_factor()?;
    Ok(host.inner_size()?.to_logical::<f64>(scale_factor).width)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guard for the docking layout: the page has to be a fixed width sibling column
    /// of the workflow webview, which only works while the window box is horizontal.
    #[test]
    fn the_page_is_a_fixed_width_column_next_to_the_workflow_ui() {
        let source = include_str!("gtk_panel.rs");

        assert!(source.contains("window_box.set_orientation(gtk::Orientation::Horizontal)"));
        assert!(source.contains("window_box.pack_start(&column, false, false, 0)"));
        assert!(source.contains("column.set_size_request(width.round() as i32, -1)"));

        // A fixed container would stack the page over the workflow UI instead of
        // giving it its own space.
        let fixed_container_use = format!("{}{}", "Fixed::", "new");
        assert!(!source.contains(&fixed_container_use));
    }

    /// Guard for the session lifetime: hiding the page must keep its webview.
    #[test]
    fn hiding_the_page_keeps_the_webview_alive() {
        let source = include_str!("gtk_panel.rs");
        let hide = source
            .split("pub fn hide")
            .nth(1)
            .expect("the hide path is missing")
            .split("pub fn set_width")
            .next()
            .expect("the hide path is not terminated");

        assert!(hide.contains("page.column.hide()"));
        assert!(!hide.contains("destroy"));
    }

    /// Guard for the isolation boundary: the page is built by wry, so it gets no
    /// Tauri IPC and cannot reach a ChatSpeed command.
    #[test]
    fn the_page_is_built_by_wry_without_tauri_ipc() {
        let source = include_str!("gtk_panel.rs");

        assert!(source.contains("build_gtk(&column)"));
        assert!(!source.contains(concat!("Webview", "Builder")));
    }
}
