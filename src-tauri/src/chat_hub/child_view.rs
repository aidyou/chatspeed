//! Windows and macOS carrier of the ChatHub page.
//!
//! On these platforms a webview can be created as a child view at an explicit
//! rectangle, and the platform moves a child view together with its parent window, so
//! the page follows the window without any per-move work and without a separate top
//! level window.
//!
//! A child view is stacked *over* the workflow UI instead of changing the window
//! layout, so the frontend keeps the matching space free on its own side
//! ([`super::view_mode`] reports `reserve`), and this module keeps the rectangle in
//! sync with the window size because the page is not part of the Tauri webview
//! registry that would resize a Tauri webview automatically.

use std::sync::Mutex;

use tauri::{AppHandle, WebviewWindow, Wry};
use wry::{
    dpi::{LogicalPosition, LogicalSize},
    Rect, WebContext, WebView,
};

use super::{clamp_width, host_window, page_builder, page_data_directory, page_proxy};
use crate::db::chat_hub::parse_chat_hub_url;
use crate::error::{AppError, Result};

/// State of the single ChatHub page.
#[derive(Debug, Default)]
pub struct ChatHubPageState {
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    page: Option<Page>,
    /// Url the page is currently showing.
    url: Option<String>,
    /// Width currently applied to the page, in logical pixels.
    width: Option<f64>,
    /// Space the frontend keeps free above the page, in logical pixels.
    top_inset: f64,
}

/// The native handle of the docked page.
struct Page {
    webview: WebView,
}

impl std::fmt::Debug for Page {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Page").finish_non_exhaustive()
    }
}

// SAFETY: a `wry::WebView` may only be touched on the main thread. The state keeps
// the handle solely so the page can be navigated, resized and destroyed later, and
// every access happens on that thread: `run_on_page_thread` posts each operation
// there, and the window events that resize or release the page already run there.
unsafe impl Send for Page {}

impl ChatHubPageState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reveals the page, docked to the right edge of the workflow window.
    ///
    /// The same webview is reused for every ChatHub entry, so the site keeps its
    /// cookies and session while navigating between entries. `top_inset` is the space
    /// the frontend chrome occupies, which a stacked page must not cover, and
    /// `corner_radius` is the radius of the rounded window border the page gives back
    /// at its bottom-right corner.
    pub fn show(
        &self,
        app: &AppHandle<Wry>,
        url: &str,
        width: f64,
        top_inset: f64,
        corner_radius: f64,
    ) -> Result<()> {
        let url = parse_chat_hub_url(url)?.to_string();
        let host = host_window(app)?;
        let (bounds, width) = page_bounds(&host, width, top_inset)?;

        let mut inner = self.inner.lock()?;
        inner.top_inset = top_inset;

        if inner.page.is_none() {
            // The page is reused for every entry, so building it is the only moment a
            // proxy can be applied: the settings are read here.
            let mut web_context = WebContext::new(Some(page_data_directory(app)));
            let webview = page_builder(&mut web_context, &url, page_proxy(app), corner_radius)
                .with_bounds(bounds)
                .build_as_child(&host)?;

            inner.page = Some(Page { webview });
            inner.url = Some(url.clone());

            #[cfg(debug_assertions)]
            log::info!(
                "[ChatHub] docked the page at {:.0} logical pixels in the '{}' window",
                width,
                super::CHAT_HUB_HOST_WINDOW_LABEL
            );
        }

        let page = inner.page.as_ref().ok_or_else(|| AppError::General {
            message: "the ChatHub page is missing".to_string(),
        })?;

        if inner.url.as_deref() != Some(url.as_str()) {
            page.webview.load_url(&url)?;
        }

        page.webview.set_bounds(bounds)?;
        page.webview.set_visible(true)?;

        inner.url = Some(url);
        inner.width = Some(width);

        Ok(())
    }

    /// Hides the page without destroying it, so its session survives.
    pub fn hide(&self, _app: &AppHandle<Wry>) -> Result<()> {
        let inner = self.inner.lock()?;
        if let Some(page) = inner.page.as_ref() {
            page.webview.set_visible(false)?;
        }
        Ok(())
    }

    /// Applies a new width to the docked page.
    pub fn set_width(&self, app: &AppHandle<Wry>, width: f64) -> Result<()> {
        let host = host_window(app)?;
        let top_inset = self.inner.lock()?.top_inset;
        let (bounds, width) = page_bounds(&host, width, top_inset)?;

        let mut inner = self.inner.lock()?;
        if let Some(page) = inner.page.as_ref() {
            page.webview.set_bounds(bounds)?;
        }
        inner.width = Some(width);

        Ok(())
    }

    /// Re-applies the page rectangle after the host window changed size.
    ///
    /// Moving the window needs no work here, because the platform moves a child view
    /// together with its parent; only a size change has to be forwarded.
    pub fn sync_bounds(&self, app: &AppHandle<Wry>) -> Result<()> {
        let inner = self.inner.lock()?;
        let Some(page) = inner.page.as_ref() else {
            return Ok(());
        };
        let Some(width) = inner.width else {
            return Ok(());
        };
        let top_inset = inner.top_inset;

        let host = host_window(app)?;
        let (bounds, _) = page_bounds(&host, width, top_inset)?;

        Ok(page.webview.set_bounds(bounds)?)
    }

    /// Destroys the page and clears every tracked view field.
    ///
    /// Only the explicit close action and application exit reach this, so switching
    /// entries or hiding the page keeps the site session alive.
    pub fn destroy(&self, _app: &AppHandle<Wry>) -> Result<()> {
        let mut inner = self.inner.lock()?;

        // Dropping the handle is what destroys the embedded page.
        inner.page = None;
        inner.url = None;
        inner.width = None;

        Ok(())
    }

    /// Clears tracked state without touching a handle that is already gone.
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

/// Logical size of the host window client area.
fn host_inner_size(host: &WebviewWindow<Wry>) -> Result<LogicalSize<f64>> {
    let scale_factor = host.scale_factor()?;
    Ok(host.inner_size()?.to_logical::<f64>(scale_factor))
}

/// Rectangle of the docked page inside the host window, plus the width it uses.
///
/// `top_inset` is the space the frontend chrome (the app titlebar, with the window
/// controls) occupies, which a stacked page has to leave free.
fn page_bounds(host: &WebviewWindow<Wry>, width: f64, top_inset: f64) -> Result<(Rect, f64)> {
    let window_size = host_inner_size(host)?;
    let width = clamp_width(window_size.width, width);
    let top = top_inset.clamp(0.0, window_size.height);

    let bounds = Rect {
        position: LogicalPosition::new(window_size.width - width, top).into(),
        size: LogicalSize::new(width, (window_size.height - top).max(1.0)).into(),
    };

    Ok((bounds, width))
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

    /// Guard for the docking layout: the page has to be a child view of the workflow
    /// window placed at an explicit rectangle on its right edge, which is what makes
    /// the platform move it together with the window.
    #[test]
    fn the_page_is_a_child_view_at_the_right_edge_of_the_workflow_window() {
        let source = include_str!("child_view.rs");
        let creation = implementation(source, "pub fn show", "pub fn hide");
        let rectangle = implementation(source, "fn page_bounds", "#[cfg(test)]");

        assert!(creation.contains("build_as_child(&host)"));
        assert!(creation.contains(".with_bounds(bounds)"));
        // The rectangle starts at the right edge and leaves the app chrome space free.
        assert!(rectangle.contains("LogicalPosition::new(window_size.width - width, top)"));
        assert!(rectangle.contains("LogicalSize::new(width, (window_size.height - top).max(1.0))"));
    }

    /// Guard for the resize path: the page is not part of the Tauri webview registry,
    /// so nothing but this module can keep its rectangle correct after a resize.
    #[test]
    fn the_page_rectangle_follows_a_window_resize() {
        let source = include_str!("child_view.rs");
        let sync = implementation(source, "pub fn sync_bounds", "pub fn destroy");

        assert!(sync.contains("page_bounds(&host, width, top_inset)"));
        assert!(sync.contains("page.webview.set_bounds(bounds)"));
    }

    /// Guard for the session lifetime: hiding the page must keep its webview.
    #[test]
    fn hiding_the_page_keeps_the_webview_alive() {
        let source = include_str!("child_view.rs");
        let hide = source
            .split("pub fn hide")
            .nth(1)
            .expect("the hide path is missing")
            .split("pub fn set_width")
            .next()
            .expect("the hide path is not terminated");

        assert!(hide.contains("page.webview.set_visible(false)"));
        assert!(!hide.contains("destroy"));
    }

    /// Guard for the isolation boundary: the page is built by wry, so it gets no Tauri
    /// IPC and cannot reach a ChatSpeed command.
    #[test]
    fn the_page_is_built_by_wry_without_tauri_ipc() {
        let source = include_str!("child_view.rs");

        assert!(source.contains("build_as_child(&host)"));
        assert!(!source.contains(concat!("Webview", "Builder")));
    }
}
