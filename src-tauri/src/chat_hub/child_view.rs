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

use tauri::{AppHandle, PhysicalSize, WebviewWindow, Wry};
use wry::{
    dpi::{LogicalPosition, LogicalSize},
    Rect, WebContext, WebView,
};

use super::{
    clamp_width, host_window, narrow_host_window, page_builder, page_data_directory, page_proxy,
    report_predates_layout, widen_host_window,
};
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
    /// Width the docked page took from the host window, in logical pixels.
    ///
    /// The workflow UI keeps its width next to the page only because the window grows by
    /// the width of the page, so the added width is remembered here and handed back when
    /// the page goes away. Zero means the page holds no width of the window.
    grown: f64,
    /// Window width the rectangle of the page was computed for, in logical pixels.
    layout_window_width: f64,
    /// Whether the window still reports the geometry the page made room from.
    ///
    /// A resize is reported before the window has applied it, so the report that arrives right
    /// after this carrier grew the window still describes the window it grew from. Laying the
    /// page out for that width would put it back over the workflow UI, so the layout is kept
    /// until the window reports something else.
    layout_pending: bool,
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
    ///
    /// A stacked page takes its width from the workflow UI, so the window is widened by
    /// that width first (see [`super::room_for_page`]) and the workflow UI keeps the width
    /// it had. A window that already fills the screen cannot grow and the page then takes
    /// its space from the workflow UI, as it always did.
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
        let window_size = host_inner_size(&host)?;

        // The page makes room for itself once: re-selecting an entry, or bringing a hidden
        // page back, must not widen the window a second time. The window is resized outside
        // the state lock, because a resize is reported back to this window.
        let added = if self.grown_width() <= 0.0 {
            widen_host_window(&host, width)
        } else {
            0.0
        };

        // The page is laid out against the window it is about to live in, so it never shows
        // up at the right edge it would have had and jumps to the new one afterwards.
        let window_size = LogicalSize::new(window_size.width + added, window_size.height);
        let (bounds, width) = page_bounds(width, top_inset, window_size);

        let mut inner = self.inner.lock()?;
        inner.top_inset = top_inset;
        inner.grown += added;
        inner.layout_window_width = window_size.width;
        // The window is asked for a width it reports as the one it had before, so the report
        // that is about to arrive is told apart from a resize the user performs later.
        inner.layout_pending = added > 0.0;

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
    ///
    /// The width the page took from the window is handed back with it, so the workflow UI
    /// keeps the width it has while the page is away.
    pub fn hide(&self, app: &AppHandle<Wry>) -> Result<()> {
        let grown = self.take_grown()?;

        let hidden = {
            let mut inner = self.inner.lock()?;
            // The page is leaving the window, so no report has a layout left to keep.
            inner.layout_pending = false;

            match inner.page.as_ref() {
                Some(page) => page.webview.set_visible(false).map_err(AppError::from),
                None => Ok(()),
            }
        };

        self.hand_width_back(app, grown);
        hidden
    }

    /// Applies a new width to the docked page.
    ///
    /// A drag only changes how much of the window the page takes: the window itself is not
    /// resized while the splitter moves, so the width it holds is left as the opening set it.
    pub fn set_width(&self, app: &AppHandle<Wry>, width: f64) -> Result<()> {
        let host = host_window(app)?;
        let top_inset = self.inner.lock()?.top_inset;
        let (bounds, width) = page_bounds(width, top_inset, host_inner_size(&host)?);

        let mut inner = self.inner.lock()?;
        if let Some(page) = inner.page.as_ref() {
            page.webview.set_bounds(bounds)?;
        }
        inner.width = Some(width);

        Ok(())
    }

    /// Re-applies the page rectangle after the host window reported a size.
    ///
    /// Moving the window needs no work here, because the platform moves a child view
    /// together with its parent; only a size change has to be forwarded.
    ///
    /// `reported` is the size the window reported, and that is not always the size it has: a
    /// resize reaches this handler before the window has applied the change it describes, so
    /// the report of the resize this carrier asked for still describes the window the page grew
    /// from. Laying the page out for that width would put it back over the workflow UI, so the
    /// layout the page was given is kept for such a report.
    pub fn sync_bounds(&self, app: &AppHandle<Wry>, reported: PhysicalSize<u32>) -> Result<()> {
        let host = host_window(app)?;
        let scale_factor = host.scale_factor()?;
        let reported = reported.to_logical::<f64>(scale_factor);

        let mut inner = self.inner.lock()?;
        let Some(width) = inner.width else {
            return Ok(());
        };
        if inner.page.is_none() {
            return Ok(());
        }
        let top_inset = inner.top_inset;

        let predates_layout = inner.layout_pending
            && report_predates_layout(reported.width, inner.layout_window_width, inner.grown);

        let window_size = if predates_layout {
            #[cfg(debug_assertions)]
            log::debug!(
                "[ChatHub] kept the docked layout of {:.0} logical pixels while the '{}' window reports {:.0}",
                inner.layout_window_width,
                super::CHAT_HUB_HOST_WINDOW_LABEL,
                reported.width
            );

            // The report describes the window the page made room from, so the layout that was
            // computed for the width the page was given is kept instead.
            LogicalSize::new(inner.layout_window_width, reported.height)
        } else {
            inner.layout_pending = false;
            reported
        };
        let bounds = page_bounds(width, top_inset, window_size).0;

        let page = inner.page.as_ref().ok_or_else(|| AppError::General {
            message: "the ChatHub page is missing".to_string(),
        })?;

        Ok(page.webview.set_bounds(bounds)?)
    }

    /// Destroys the page and clears every tracked view field.
    ///
    /// Only the explicit close action and application exit reach this, so switching
    /// entries or hiding the page keeps the site session alive.
    pub fn destroy(&self, app: &AppHandle<Wry>) -> Result<()> {
        let grown = self.take_grown()?;

        {
            let mut inner = self.inner.lock()?;

            // Dropping the handle is what destroys the embedded page.
            inner.page = None;
            inner.url = None;
            inner.width = None;
            inner.layout_window_width = 0.0;
            inner.layout_pending = false;
        }

        self.hand_width_back(app, grown);
        Ok(())
    }

    /// Clears tracked state without touching a handle that is already gone.
    pub fn forget(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.page = None;
            inner.url = None;
            inner.width = None;
            inner.grown = 0.0;
            inner.layout_window_width = 0.0;
            inner.layout_pending = false;
        }
    }

    /// Releases the page together with the window it was docked to.
    ///
    /// The window is going away with the page, so the width the page holds is dropped
    /// instead of being handed back to a window that is closing anyway.
    pub fn release(&self, app: &AppHandle<Wry>) {
        if let Err(error) = self.take_grown() {
            log::warn!("Failed to read the ChatHub page width: {}", error);
        }

        if let Err(error) = self.destroy(app) {
            log::warn!("Failed to release the ChatHub page: {}", error);
        }
        self.forget();
    }

    /// Width the docked page took from the host window, in logical pixels.
    ///
    /// A window size is remembered across runs, so the width the page holds is reported
    /// here to be kept out of that record: reopening the app must not restore a window that
    /// is wider than the workflow UI ever was.
    pub fn grown_width(&self) -> f64 {
        self.inner.lock().map(|inner| inner.grown).unwrap_or(0.0)
    }

    /// Takes the width the page holds out of the state, so it is handed back exactly once.
    fn take_grown(&self) -> Result<f64> {
        let mut inner = self.inner.lock()?;
        Ok(std::mem::take(&mut inner.grown))
    }

    /// Gives the width the page took back to the window it was taken from.
    fn hand_width_back(&self, app: &AppHandle<Wry>, grown: f64) {
        if grown <= 0.0 {
            return;
        }

        match host_window(app) {
            Ok(host) => narrow_host_window(&host, grown),
            Err(error) => log::warn!("Failed to reach the ChatHub host window: {}", error),
        }
    }
}

/// Logical size of the host window client area.
fn host_inner_size(host: &WebviewWindow<Wry>) -> Result<LogicalSize<f64>> {
    let scale_factor = host.scale_factor()?;
    Ok(host.inner_size()?.to_logical::<f64>(scale_factor))
}

/// Rectangle of the docked page inside the host window, plus the width it uses.
///
/// `window_size` is the window the page is laid out in. While the page is making room for
/// itself that is the widened window rather than the one still on screen, so the page never
/// shows up at the wrong edge. `top_inset` is the space the frontend chrome (the app
/// titlebar, with the window controls) occupies, which a stacked page has to leave free.
fn page_bounds(width: f64, top_inset: f64, window_size: LogicalSize<f64>) -> (Rect, f64) {
    let width = clamp_width(window_size.width, width);
    let top = top_inset.clamp(0.0, window_size.height);

    let bounds = Rect {
        position: LogicalPosition::new(window_size.width - width, top).into(),
        size: LogicalSize::new(width, (window_size.height - top).max(1.0)).into(),
    };

    (bounds, width)
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

        assert!(sync.contains("page_bounds(width, top_inset, window_size).0"));
        assert!(sync.contains("page.webview.set_bounds(bounds)"));
        // A resize is reported before the window has applied it, so the layout the page was
        // given is kept for the report that still describes the window it grew from.
        assert!(sync.contains("report_predates_layout("));
        assert!(sync.contains("inner.layout_pending = false;"));
    }

    /// Guard for the window width: the page takes its width from the workflow UI, so the
    /// window has to be widened when the page opens and to hand exactly that width back when
    /// the page goes away.
    #[test]
    fn the_page_makes_room_in_the_window_once_and_hands_it_back() {
        let source = include_str!("child_view.rs");
        let show = implementation(source, "pub fn show", "pub fn hide");
        let hide = implementation(source, "pub fn hide", "pub fn set_width");
        let destroy = implementation(source, "pub fn destroy", "pub fn forget");
        let release = implementation(source, "pub fn release", "pub fn grown_width");

        // The page is widened once, never once per entry selection.
        assert!(show.contains("if self.grown_width() <= 0.0 {"));
        assert!(show.contains("widen_host_window(&host, width)"));
        // The rectangle is measured against the widened window, not the one still on screen.
        assert!(show.contains("window_size.width + added"));
        // Hiding and closing both give the width back.
        assert!(hide.contains("take_grown()"));
        assert!(hide.contains("hand_width_back(app, grown)"));
        assert!(destroy.contains("take_grown()"));
        assert!(destroy.contains("hand_width_back(app, grown)"));
        // Closing the window releases the page without resizing a window that is going away.
        assert!(release.contains("take_grown()"));
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
