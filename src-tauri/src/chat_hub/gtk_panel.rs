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
use tauri::{AppHandle, PhysicalSize, WebviewWindow, Wry};
use wry::{WebContext, WebView, WebViewBuilderExtUnix};

use super::{
    clamp_width, host_window, narrow_host_window, page_builder, page_data_directory, page_proxy,
    widen_host_window,
};
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
    /// Width the docked page took from the host window, in logical pixels.
    ///
    /// The workflow webview keeps its width next to the page only because the window grows
    /// by the width of the page, so the added width is remembered here and handed back when
    /// the page goes away. Zero means the page holds no width of the window.
    grown: f64,
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
    /// `corner_radius` is the radius the window draws, which the page gives back at both
    /// right corners: the page owns the window's right edge from top to bottom, so the
    /// top-right corner belongs to it as well.
    ///
    /// The page takes its width from the workflow webview, so the window is widened by that
    /// width first (see [`super::room_for_page`]) and the workflow UI keeps the width it had.
    /// A window that already fills the screen cannot grow and the page then takes its space
    /// from the workflow UI, as it always did.
    pub fn show(
        &self,
        app: &AppHandle<Wry>,
        url: &str,
        width: f64,
        _top_inset: f64,
        corner_radius: f64,
    ) -> Result<()> {
        let url = parse_chat_hub_url(url)?.to_string();
        let host = host_window(app)?;
        let window_width = host_inner_width(&host)?;

        // The page makes room for itself once: re-selecting an entry, or bringing a hidden
        // page back, must not widen the window a second time. The window is resized outside
        // the state lock, because a resize is reported back to this window.
        let added = if self.grown_width() <= 0.0 {
            widen_host_window(&host, width)
        } else {
            0.0
        };

        // The width limits are checked against the window the page will have, so the page
        // never has to be laid out inside the window it is leaving behind.
        let width = clamp_width(window_width + added, width);

        let mut inner = self.inner.lock()?;
        inner.grown += added;

        if inner.page.is_none() {
            inner.page = Some(Page::create(app, &host, &url, width, corner_radius)?);
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
    ///
    /// The width the page took from the window is handed back with it, so the workflow UI
    /// keeps the width it has while the page is away.
    pub fn hide(&self, app: &AppHandle<Wry>) -> Result<()> {
        let grown = self.take_grown()?;

        {
            let inner = self.inner.lock()?;
            if let Some(page) = inner.page.as_ref() {
                page.column.hide();
            }
        }

        self.hand_width_back(app, grown);
        Ok(())
    }

    /// Applies a new width to the docked page.
    ///
    /// A drag only changes how much of the window the page takes: the window itself is not
    /// resized while the splitter moves, so the width it holds is left as the opening set it.
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
    ///
    /// The reported size is therefore not needed either: the column follows the window
    /// without a rectangle of its own being recomputed here.
    pub fn sync_bounds(&self, _app: &AppHandle<Wry>, _reported: PhysicalSize<u32>) -> Result<()> {
        Ok(())
    }

    /// Destroys the page and clears every tracked view field.
    ///
    /// Only the explicit close action and application exit reach this, so switching
    /// entries or hiding the page keeps the site session alive.
    pub fn destroy(&self, app: &AppHandle<Wry>) -> Result<()> {
        let grown = self.take_grown()?;

        {
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
        }

        self.hand_width_back(app, grown);
        Ok(())
    }

    /// Clears tracked state without touching handles that are already gone.
    pub fn forget(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.page = None;
            inner.url = None;
            inner.width = None;
            inner.grown = 0.0;
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

impl Page {
    /// Creates the page column inside the workflow window and builds the page in it.
    ///
    /// `corner_radius` is the radius the window draws at its right edge, which the page covers
    /// at both of its right corners.
    fn create(
        app: &AppHandle<Wry>,
        host: &WebviewWindow<Wry>,
        url: &str,
        width: f64,
        corner_radius: f64,
    ) -> Result<Self> {
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

        // A page that gives a window corner back also has to be see-through, so the radius only
        // reaches a window that draws one.
        let radius = corner_radius.clamp(0.0, MAX_PAGE_CORNER_RADIUS);
        let mut builder = page_builder(&mut web_context, url, page_proxy(app), radius);
        if radius > 0.0 {
            // The page is packed next to the workflow webview instead of being stacked over it,
            // which is what makes it own the window's right edge from top to bottom: the shared
            // corner script gives the bottom-right corner back, and the top-right one, which a
            // stacked page never covers, is rounded on top of it.
            builder = builder.with_initialization_script(top_corner_script(radius));
        }

        let webview = builder.build_gtk(&column)?;

        Ok(Self { webview, column })
    }

    /// Applies a width to the page column.
    fn set_width(&self, width: f64) {
        self.column.set_size_request(width.round() as i32, -1);
    }
}

/// Largest corner radius the page accepts, so a bad measurement cannot eat into the page.
///
/// It repeats the bound the shared corner script applies, which only has to cover the
/// bottom-right corner: the top-right corner is rounded by [`top_corner_script`].
const MAX_PAGE_CORNER_RADIUS: f64 = 30.0;

/// Script that leaves the top-right corner of the window unpainted.
///
/// The shared corner script ([`super::page`]) rounds the bottom-right corner, which is the one
/// every carrier covers. The workflow UI keeps its own titlebar *beside* the page on this
/// platform instead of above it, so the page owns the window's right edge from top to bottom and
/// covers the top-right corner as well. That corner therefore has to come back from the page
/// here, and the rules are the ones that hold at the other corner:
///
/// - The corner is given back by every element that paints it. A decorative layer carries
///   `pointer-events: none`, so it never shows up in a hit test and the document is inspected by
///   geometry instead: an element is rounded when it covers the top-right point of the viewport
///   and paints something there.
/// - A pseudo element paints a box of its own, which the radius of its host does not cut, so a
///   host that paints the corner through one is marked and the corner reaches it through a rule.
/// - The corner is applied again for a short while after the page load, because a site can build
///   the layer that paints it later than the load event.
fn top_corner_script(radius: f64) -> String {
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
    var rule = '[data-cs-top-corner]::before,[data-cs-top-corner]::after'
      + '{{border-top-right-radius:' + radius + ' !important}}';
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
    var y = 2;
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
          element.setAttribute('data-cs-top-corner', '');
          addPseudoRule();
        }}
      }}
    }}
    for (var target = 0; target < targets.length; target += 1) {{
      targets[target].style.setProperty('border-top-right-radius', radius, 'important');
    }}
    document.documentElement.style.setProperty('border-top-right-radius', radius, 'important');
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

    /// Guard for the window width: the page takes its width from the workflow webview, so
    /// the window has to be widened when the page opens and to hand exactly that width back
    /// when the page goes away.
    #[test]
    fn the_page_makes_room_in_the_window_once_and_hands_it_back() {
        let source = include_str!("gtk_panel.rs");

        assert!(source.contains("if self.grown_width() <= 0.0 {"));
        assert!(source.contains("widen_host_window(&host, width)"));
        assert!(source.contains("narrow_host_window(&host, grown)"));
        // The width limits are checked against the window the page will have.
        assert!(source.contains("clamp_width(window_width + added, width)"));
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
