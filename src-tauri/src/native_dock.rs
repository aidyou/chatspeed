//! Shared native dock surface of the Workflow window.
//!
//! ChatHub tabs and plugin UI panels are both native `wry` webviews that are layered over the
//! workflow UI at the rectangle the frontend measured for the right dock. They are placed
//! through this surface, so one window holds exactly one overlay and every native view is
//! bounded to the rectangle it was given, whatever opened it.
//!
//! On Linux the surface installs a single `gtk::Overlay` over the window content and adds each
//! native view as a *bounded* overlay child: a plain `gtk::Box` aligned to the start of the
//! overlay, with the rectangle as its margins and its size request. The overlay child is the
//! only place the GTK input handling routes events to; a full-area overlay child would swallow
//! every pointer event of the window instead, which is what kept the workflow UI from receiving
//! clicks while a plugin panel was open.
//!
//! On Windows and macOS a native view is a child view placed at an explicit rectangle, so there
//! is no container to install: [`DockSurface`] exists there so the same managed state is used on
//! every platform.

use tauri::{AppHandle, Manager, WebviewWindow, Wry};

use crate::error::{AppError, Result};

/// Smallest dock edge that is worth a native view, in logical pixels.
///
/// A rectangle can be measured mid-frame, so the value only has to keep a nonsense or empty
/// rectangle away from a native view.
const MIN_DOCK_SIDE: f64 = 1.0;

/// Rectangle of one native dock view inside the Workflow window, in logical pixels.
///
/// The frontend measures this rectangle on its own dock placeholder, so the native side only
/// places a view: how much room the workflow layout keeps free stays the frontend's business.
/// The same rectangle is used for a ChatHub tab and for a plugin panel.
#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize)]
pub struct DockBounds {
    /// Left edge, in logical pixels.
    pub x: f64,
    /// Top edge, in logical pixels.
    pub y: f64,
    /// Width, in logical pixels.
    pub width: f64,
    /// Height, in logical pixels.
    pub height: f64,
}

impl DockBounds {
    /// Fits a measured rectangle into the window, or refuses one that cannot describe a view.
    ///
    /// The rectangle comes from the frontend's own measurement, so a resize caught mid-frame can
    /// carry a non-finite or empty value. An offset outside the window is pulled back to its
    /// edge; a rectangle that no longer fits the window at all is refused instead of being handed
    /// to a native view as a nonsense geometry.
    pub fn sanitize(self, window_width: f64, window_height: f64) -> Result<Self> {
        let measured = self.x.is_finite()
            && self.y.is_finite()
            && self.width.is_finite()
            && self.height.is_finite()
            && window_width.is_finite()
            && window_height.is_finite();
        if !measured {
            return Err(AppError::General {
                message: "the dock bounds are not finite".to_string(),
            });
        }
        if self.width < MIN_DOCK_SIDE || self.height < MIN_DOCK_SIDE {
            return Err(AppError::General {
                message: format!(
                    "the dock bounds are empty ({:.0}x{:.0})",
                    self.width, self.height
                ),
            });
        }

        let x = self.x.clamp(0.0, window_width);
        let y = self.y.clamp(0.0, window_height);
        let width = self.width.min(window_width - x);
        let height = self.height.min(window_height - y);
        if width < MIN_DOCK_SIDE || height < MIN_DOCK_SIDE {
            return Err(AppError::General {
                message: "the dock bounds lie outside the window".to_string(),
            });
        }

        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }
}

/// Logical size of the host window client area, for clamping a measured rectangle.
pub fn window_size(host: &WebviewWindow<Wry>) -> Result<(f64, f64)> {
    let scale_factor = host.scale_factor()?;
    let size = host.inner_size()?.to_logical::<f64>(scale_factor);
    Ok((size.width, size.height))
}

/// Runs `operation` with the managed dock surface of the application.
///
/// The surface is managed once at startup and shared by every native carrier, so a missing
/// surface is reported as a plain error instead of being worked around with a second overlay.
pub fn with_surface<T>(
    app: &AppHandle<Wry>,
    operation: impl FnOnce(&DockSurface) -> Result<T>,
) -> Result<T> {
    match app.try_state::<DockSurface>() {
        Some(surface) => operation(surface.inner()),
        None => Err(AppError::General {
            message: "the native dock surface is not managed".to_string(),
        }),
    }
}

/// Largest corner radius a docked page draws, so a bad measurement cannot eat into the page.
///
/// It bounds both the corner the X11 backend clips into the native view and the corner the shared
/// script leaves unpainted, so a radius that reaches the window is treated the same way on every
/// carrier.
pub const MAX_PAGE_CORNER_RADIUS: f64 = 30.0;

/// Clamps a requested corner radius to a finite length a page can draw.
///
/// The radius reaches the page from the frontend's own measurement, so a value caught mid-frame
/// (or a nonsense one) is refused instead of being handed to a native view or to a script.
pub fn clamp_corner_radius(radius: f64) -> f64 {
    if radius.is_finite() {
        radius.clamp(0.0, MAX_PAGE_CORNER_RADIUS)
    } else {
        0.0
    }
}

/// Script that leaves the rounded bottom-right corner of a page unpainted.
///
/// A docked page is a rectangular native view, so no border radius of the window it is stacked
/// over can cut it: only a backend that shapes the native view can clip it, and every other
/// backend has to let the page give the corner back itself. That is what this script does, and it
/// is shared by the ChatHub page and by a plugin UI panel so the two corners cannot drift apart.
///
/// The script is an installer: it keeps its state on the page, installs its listeners once, and
/// exposes `window.__csCorner.set(radius)`. A radius that changes while the page is alive is
/// applied through [`bottom_right_corner_update_script`], so it never costs a reload. Three rules
/// make the corner hold on pages that build themselves differently:
///
/// - The corner is given back by every element that paints it. A decorative layer carries
///   `pointer-events: none`, so it never shows up in a hit test and the document is inspected by
///   geometry instead: an element is rounded when it covers the bottom-right point of the viewport
///   and paints something there.
/// - A pseudo element paints a box of its own, which the radius of its host does not cut, so a
///   host that paints the corner through one is marked and the corner reaches it through a rule.
/// - The corner is applied again for a short while after the page load, and whenever the radius
///   changes, because a site can build the layer that paints the corner later than the load event.
///
/// Every radius the script set is remembered, so a radius of zero removes them all and restores
/// whatever the page painted before, which is what a window that becomes square again needs.
pub fn bottom_right_corner_script(radius: f64) -> String {
    let radius = clamp_corner_radius(radius);
    format!(
        r#"(function () {{
  var KEY = '__csCorner';
  var transparent = 'rgba(0, 0, 0, 0)';
  var state = window[KEY];
  if (!state) {{
    state = {{ radius: 0, applied: [], pending: 0, tries: 0, installed: false }};
    window[KEY] = state;
  }}
  function opaque(background) {{
    return !!background && background !== 'transparent' && background !== transparent;
  }}
  function paints(style) {{
    return style.backgroundImage !== 'none' || opaque(style.backgroundColor);
  }}
  function styleElement() {{
    var element = document.getElementById('cs-corner-pseudo');
    if (!element) {{
      element = document.createElement('style');
      element.id = 'cs-corner-pseudo';
      (document.head || document.documentElement).appendChild(element);
    }}
    return element;
  }}
  function appliedOf(element) {{
    for (var index = 0; index < state.applied.length; index += 1) {{
      if (state.applied[index].element === element) {{
        return state.applied[index];
      }}
    }}
    return null;
  }}
  function applyRadius(element, radius) {{
    var record = appliedOf(element);
    if (!record) {{
      record = {{
        element: element,
        value: element.style.getPropertyValue('border-bottom-right-radius'),
        priority: element.style.getPropertyPriority('border-bottom-right-radius')
      }};
      state.applied.push(record);
    }}
    element.style.setProperty('border-bottom-right-radius', radius, 'important');
  }}
  function restore() {{
    for (var index = 0; index < state.applied.length; index += 1) {{
      var record = state.applied[index];
      if (record.value) {{
        record.element.style.setProperty('border-bottom-right-radius', record.value, record.priority);
      }} else {{
        record.element.style.removeProperty('border-bottom-right-radius');
      }}
    }}
    state.applied = [];
    var marked = document.querySelectorAll('[data-cs-corner]');
    for (var mark = 0; mark < marked.length; mark += 1) {{
      marked[mark].removeAttribute('data-cs-corner');
    }}
    var style = document.getElementById('cs-corner-pseudo');
    if (style) {{
      style.textContent = '';
    }}
  }}
  function roundCorner() {{
    if (state.radius <= 0) {{
      restore();
      return;
    }}
    var radius = state.radius + 'px';
    styleElement().textContent = '[data-cs-corner]::before,[data-cs-corner]::after'
      + '{{border-bottom-right-radius:' + radius + ' !important}}';
    var x = window.innerWidth - 2;
    var y = window.innerHeight - 2;
    var elements = document.querySelectorAll('*');
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
        applyRadius(element, radius);
      }}
      for (var part = 0; part < 2; part += 1) {{
        var pseudo = window.getComputedStyle(element, part ? '::after' : '::before');
        if (pseudo.content && pseudo.content !== 'none' && paints(pseudo)) {{
          element.setAttribute('data-cs-corner', '');
        }}
      }}
    }}
    applyRadius(document.documentElement, radius);
  }}
  state.apply = roundCorner;
  state.set = function (value) {{
    var next = Number(value);
    state.radius = isFinite(next) && next > 0 ? next : 0;
    roundCorner();
  }};
  if (!state.installed) {{
    state.installed = true;
    document.addEventListener('DOMContentLoaded', roundCorner);
    window.addEventListener('load', roundCorner);
    window.addEventListener('resize', function () {{
      if (state.pending) {{
        return;
      }}
      state.pending = window.setTimeout(function () {{
        state.pending = 0;
        roundCorner();
      }}, 200);
    }});
    state.retry = window.setInterval(function () {{
      state.tries += 1;
      if (state.tries > 15) {{
        window.clearInterval(state.retry);
        return;
      }}
      roundCorner();
    }}, 400);
  }}
  try {{
    state.set({radius});
  }} catch (error) {{}}
}})();"#
    )
}

/// JavaScript that moves the rounded corner of an already installed page to a new radius.
///
/// [`bottom_right_corner_script`] keeps the corner handling on the page and re-applies it at the
/// moments a site may build the layer that paints the corner. This only calls its runtime setter,
/// so a radius that changes while the page is alive is applied without running the installer again
/// and without reloading the page, which would drop the browsing session. A radius of zero removes
/// the radii the installer applied and restores what the page had before, so a window that becomes
/// square again gives its corner back.
pub fn bottom_right_corner_update_script(radius: f64) -> String {
    let radius = clamp_corner_radius(radius);
    format!("window.__csCorner && window.__csCorner.set({radius});")
}

/// Who owns a native dock view, so one owner can be cleared without touching the other.
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockOwner {
    /// A docked ChatHub page tab.
    ChatHub,
    /// A plugin UI panel.
    Plugin,
}

#[cfg(target_os = "linux")]
pub use linux::{supports_native_rounding, ViewShape};
#[cfg(target_os = "linux")]
pub use linux::{DockHolder, DockSurface};

/// Distance from a window edge at which a dock rectangle still counts as covering it, in logical
/// pixels. A measurement is rounded, so an exact hit is not guaranteed.
#[cfg(target_os = "linux")]
const WINDOW_EDGE_REACH: f64 = 1.0;

/// Frame band a dock view gives back for the window edges its rectangle reaches.
///
/// A dock that reaches a window edge covers the band the window level resize handler accepts, so
/// that edge has to stay with the window; an edge inside the window borders the workflow UI and
/// keeps its input. The common dock spans the right edge of the window from top to bottom, which is
/// exactly the band a right column gives back, so the named bands are reused for the shapes that
/// occur. A ChatHub tab and a plugin panel both place their native view through this one rule, so
/// the band they give back cannot drift apart.
#[cfg(target_os = "linux")]
pub fn band_for(
    bounds: DockBounds,
    window_width: f64,
    window_height: f64,
) -> crate::frame_edges::Band {
    use crate::frame_edges::Band;

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

#[cfg(target_os = "linux")]
mod linux {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;
    use gtk::prelude::*;

    use crate::chat_hub::host_window;

    /// The shared overlay of the Workflow window, installed on first use.
    struct Surface {
        overlay: gtk::Overlay,
        /// Content the overlay wraps, kept so the window layout can be handed back.
        content: gtk::Widget,
    }

    // SAFETY: both handles are GTK objects that may only be touched on the main thread. Every
    // entry point of the dock surface runs there, posted by the external commands.
    unsafe impl Send for Surface {}

    /// The installed overlay and the bounded holders it currently carries.
    #[derive(Default)]
    struct Registry {
        overlay: Option<Surface>,
        holders: Vec<(DockOwner, gtk::Box)>,
    }

    // SAFETY: the registry holds the same main-thread-only GTK handles as `Surface`.
    unsafe impl Send for Registry {}

    /// One bounded native view holder inside the shared overlay.
    pub struct DockHolder {
        holder: gtk::Box,
    }

    // SAFETY: a GTK handle may only be touched on the main thread, and every entry point that
    // reaches a holder runs there.
    unsafe impl Send for DockHolder {}

    impl DockHolder {
        /// Container the native webview is built into.
        pub fn container(&self) -> &gtk::Box {
            &self.holder
        }

        /// Places the holder at `bounds`.
        ///
        /// The holder is aligned to the start of the overlay, so its margins are the rectangle
        /// offsets and its size request is the rectangle size: the overlay then allocates the
        /// holder the rectangle instead of the whole window.
        pub fn set_bounds(&self, bounds: DockBounds) {
            self.holder
                .set_size_request(bounds.width.round() as i32, bounds.height.round() as i32);
            self.holder.set_margin_start(bounds.x.round() as i32);
            self.holder.set_margin_top(bounds.y.round() as i32);
        }

        /// Shows or hides the holder with the webview it carries.
        pub fn set_visible(&self, visible: bool) {
            if visible {
                self.holder.show_all();
            } else {
                self.holder.hide();
            }
        }
    }

    /// Whether the GDK backend can clip a native view to a shape.
    ///
    /// Only the X11 backend owns the X window a shape is applied to. A Wayland surface has no
    /// window shape, so the page has to give a corner back itself there.
    pub fn supports_native_rounding() -> bool {
        gtk::gdk::Display::default()
            .map(|display| &*display.type_().name() == "GdkX11Display")
            .unwrap_or(false)
    }

    /// Main-thread state one docked native view keeps for its rounded corner and its frame band.
    ///
    /// The GTK callbacks that reshape a view are connected once, when the view is built, and read
    /// this state on every allocation. A later show that only changes the corner radius or the band
    /// updates the state and re-applies it, so a reused ChatHub tab or plugin panel follows the
    /// frontend instead of keeping the geometry it was first built with, and no reshape callback is
    /// ever connected twice. A radius that drops to zero re-applies the same state, and both the
    /// native window shape and the page script then give the square corner back without rebuilding
    /// the view.
    pub struct ViewShape {
        widget: gtk::Widget,
        radius: Cell<f64>,
        band: Cell<crate::frame_edges::Band>,
    }

    impl ViewShape {
        /// Connects the reshape callbacks once and returns the shared state.
        ///
        /// The widget must be the native view itself, not the holder it is placed in: the shape
        /// targets the window of the view, so a widget that does not own one is left alone.
        pub fn install<W: gtk::glib::IsA<gtk::Widget>>(widget: &W) -> Rc<Self> {
            let shape = Rc::new(Self {
                widget: widget.upcast_ref::<gtk::Widget>().clone(),
                radius: Cell::new(0.0),
                band: Cell::new(crate::frame_edges::Band::EVERY_SIDE),
            });

            // Callbacks belong to the widget, so they must not keep the shape (and its widget)
            // alive after the tab releases it.
            let on_realize = Rc::downgrade(&shape);
            shape.widget.connect_realize(move |_| {
                if let Some(shape) = on_realize.upgrade() {
                    shape.apply();
                }
            });
            let on_allocate = Rc::downgrade(&shape);
            shape.widget.connect_size_allocate(move |_, _allocation| {
                if let Some(shape) = on_allocate.upgrade() {
                    shape.apply();
                }
            });

            shape
        }

        /// Sets the corner radius and the frame band, then re-applies both to the view.
        pub fn set(&self, radius: f64, band: crate::frame_edges::Band) {
            self.radius.set(radius);
            self.band.set(band);
            self.apply();
        }

        /// The corner radius the view is currently reshaped for, in logical pixels.
        ///
        /// A carrier compares this with the radius the frontend just sent to decide whether a page
        /// that gives the corner back itself has to be told about the change.
        pub fn radius(&self) -> f64 {
            self.radius.get()
        }

        /// Reshapes the view from the state it currently holds.
        fn apply(&self) {
            let radius = self.radius.get();
            crate::frame_edges::apply_frame_band_and_corner(&self.widget, self.band.get(), radius);
            apply_bottom_right(&self.widget, radius);
        }
    }

    /// Clips a native view to its rounded bottom-right corner on the backend that supports it.
    ///
    /// The shape targets the window of the view itself. A widget without its own window reports the
    /// window of the container it was added to, and shaping that shared window would clip every
    /// other native view and the window frame with it, so such a widget is left alone. The shape of
    /// the previous allocation is cleared first, which is also how a radius that dropped to zero
    /// gives the square corner back without rebuilding the view.
    fn apply_bottom_right(widget: &gtk::Widget, radius: f64) {
        if !supports_native_rounding() {
            return;
        }
        if !widget.has_window() {
            return;
        }
        let Some(window) = widget.window() else {
            return;
        };

        // A shape is combined with the one already set, so the shape of the previous allocation has
        // to go first: a view that grew would otherwise keep the narrower corner it had, and a view
        // whose radius dropped to zero would keep a corner it no longer draws.
        window.shape_combine_region(None, 0, 0);

        if !(radius > 0.0) {
            return;
        }

        // The region is in the logical window coordinates `gdk_window_get_width` reports, and GDK
        // scales it to device pixels itself, so the radius is used exactly as it arrived.
        let radius = radius.round() as i32;
        if radius <= 0 {
            return;
        }

        let (width, height) = (window.width(), window.height());
        let shaped = gtk::cairo::Region::create_rectangles(
            &crate::frame_edges::rounded_rectangles(width, height, radius),
        );
        window.shape_combine_region(Some(&shaped), 0, 0);
    }

    /// Owns every bounded native dock view of the Workflow window.
    #[derive(Default)]
    pub struct DockSurface {
        registry: std::sync::Mutex<Registry>,
    }

    impl DockSurface {
        /// Creates an empty surface; no overlay exists until the first holder is created.
        pub fn new() -> Self {
            Self::default()
        }

        /// Creates a bounded holder for one native view, installing the overlay on first use.
        pub fn holder(
            &self,
            app: &AppHandle<Wry>,
            owner: DockOwner,
            bounds: DockBounds,
        ) -> Result<DockHolder> {
            let mut registry = self.lock()?;
            registry.install(app)?;

            let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
            // The overlay positions an overlay child by its alignment and margins, so a child
            // that is aligned to the start and given a size request stays a rectangle instead of
            // filling the window.
            holder.set_halign(gtk::Align::Start);
            holder.set_valign(gtk::Align::Start);
            holder.set_hexpand(false);
            holder.set_vexpand(false);

            let dock_holder = DockHolder { holder };
            dock_holder.set_bounds(bounds);
            if let Some(surface) = registry.overlay.as_ref() {
                surface.overlay.add_overlay(dock_holder.container());
            }
            registry
                .holders
                .push((owner, dock_holder.container().clone()));
            dock_holder.set_visible(true);

            Ok(dock_holder)
        }

        /// Removes one holder, uninstalling the overlay when the last native view is gone.
        pub fn remove(&self, app: &AppHandle<Wry>, holder: &DockHolder) {
            let Ok(mut registry) = self.registry.lock() else {
                return;
            };
            registry.detach(holder.container());
            registry.settle(app);
        }

        /// Removes every holder of one owner, leaving the other owner's views in place.
        pub fn remove_owner(&self, app: &AppHandle<Wry>, owner: DockOwner) {
            let Ok(mut registry) = self.registry.lock() else {
                return;
            };
            let removed: Vec<gtk::Box> = registry
                .holders
                .iter()
                .filter(|(holder_owner, _)| *holder_owner == owner)
                .map(|(_, holder)| holder.clone())
                .collect();
            for holder in removed {
                registry.detach(&holder);
            }
            registry.settle(app);
        }

        /// Forgets the surface without touching widgets that are already gone.
        ///
        /// This is the window-destroy path: the overlay and its holders are being torn down with
        /// the window, so only the handles are released here.
        pub fn clear(&self, _app: &AppHandle<Wry>) {
            if let Ok(mut registry) = self.registry.lock() {
                *registry = Registry::default();
            }
        }

        /// Locks the registry, reporting a poisoned lock as a plain error.
        fn lock(&self) -> Result<std::sync::MutexGuard<'_, Registry>> {
            self.registry.lock().map_err(|_| AppError::General {
                message: "the native dock state is poisoned".to_string(),
            })
        }
    }

    impl Registry {
        /// Installs the shared overlay over the window content on first use.
        fn install(&mut self, app: &AppHandle<Wry>) -> Result<()> {
            if self.overlay.is_some() {
                return Ok(());
            }

            let window = host_window(app)?;
            let vbox = window.default_vbox().map_err(|error| AppError::General {
                message: format!("the Workflow window has no content box: {error}"),
            })?;

            // GTK can only layer a widget over another one, so the content the window already
            // holds is wrapped in an `Overlay` first. The overlay sizes itself to that content,
            // which keeps the window layout exactly as the workflow UI measured it.
            let overlay = gtk::Overlay::new();
            let content = vbox
                .children()
                .into_iter()
                .next()
                .ok_or_else(|| AppError::General {
                    message: "the Workflow window holds no content to overlay".to_string(),
                })?;
            vbox.remove(&content);
            overlay.add(&content);

            vbox.pack_start(&overlay, true, true, 0);
            vbox.reorder_child(&overlay, 0);
            overlay.show();

            self.overlay = Some(Surface { overlay, content });
            Ok(())
        }

        /// Detaches one holder and drops it from the registry.
        fn detach(&mut self, holder: &gtk::Box) {
            holder.hide();
            if let Some(surface) = self.overlay.as_ref() {
                surface.overlay.remove(holder);
            }
            self.holders.retain(|(_, kept)| kept != holder);
        }

        /// Hands the wrapped content back once no holder is left.
        fn settle(&mut self, app: &AppHandle<Wry>) {
            if !self.holders.is_empty() || self.overlay.is_none() {
                return;
            }
            let Some(surface) = self.overlay.take() else {
                return;
            };

            // The window keeps its layout only when the content goes back to its first slot, so
            // the box is left exactly as it was before the first native view opened.
            if let Ok(window) = host_window(app) {
                if let Ok(vbox) = window.default_vbox() {
                    surface.overlay.remove(&surface.content);
                    vbox.remove(&surface.overlay);
                    vbox.pack_start(&surface.content, true, true, 0);
                    vbox.reorder_child(&surface.content, 0);
                }
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub use other::DockSurface;

#[cfg(not(target_os = "linux"))]
mod other {
    use super::*;

    /// Native dock surface of a platform whose views are child views of the window.
    ///
    /// A child view is placed at an explicit rectangle, so there is no container to install and
    /// the surface only carries the shared managed state.
    #[derive(Default)]
    pub struct DockSurface;

    impl DockSurface {
        /// Creates an empty surface; a child view needs no container.
        pub fn new() -> Self {
            Self
        }

        /// Nothing to release: the child views go away with the window.
        pub fn clear(&self, _app: &AppHandle<Wry>) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The production half of this file, so an assertion can never match its own text.
    fn production_source() -> &'static str {
        include_str!("native_dock.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("native_dock.rs carries a test module")
    }

    /// Source of one block, so an assertion can be scoped to the code it guards.
    fn block<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
        source
            .split(start)
            .nth(1)
            .expect("the block is missing")
            .split(end)
            .next()
            .expect("the block is not terminated")
    }

    fn bounds(width: f64, height: f64) -> DockBounds {
        DockBounds {
            x: 12.0,
            y: 24.0,
            width,
            height,
        }
    }

    #[test]
    fn a_measured_rectangle_is_kept_inside_the_window() {
        let valid = bounds(320.0, 200.0);
        assert_eq!(
            valid.sanitize(1024.0, 768.0).expect("a valid rectangle"),
            valid
        );

        // An offset outside the window is pulled back onto it.
        assert_eq!(
            DockBounds {
                x: -5.0,
                y: -8.0,
                ..valid
            }
            .sanitize(1024.0, 768.0)
            .expect("a clamped rectangle"),
            DockBounds {
                x: 0.0,
                y: 0.0,
                ..valid
            }
        );

        // A rectangle that reaches past the window edge is cut to what fits.
        assert_eq!(
            DockBounds {
                x: 900.0,
                y: 700.0,
                width: 400.0,
                height: 400.0,
            }
            .sanitize(1024.0, 768.0)
            .expect("a fitted rectangle"),
            DockBounds {
                x: 900.0,
                y: 700.0,
                width: 124.0,
                height: 68.0,
            }
        );
    }

    #[test]
    fn a_rectangle_without_a_usable_geometry_is_refused() {
        // An empty edge means there is nothing to render into.
        assert!(bounds(0.0, 200.0).sanitize(1024.0, 768.0).is_err());
        assert!(bounds(320.0, 0.0).sanitize(1024.0, 768.0).is_err());
        // A nonsense measurement is refused instead of being handed to a native view.
        assert!(bounds(f64::NAN, 200.0).sanitize(1024.0, 768.0).is_err());
        assert!(bounds(320.0, f64::INFINITY)
            .sanitize(1024.0, 768.0)
            .is_err());
        // A rectangle that lies entirely outside the window is refused.
        assert!(DockBounds {
            x: 2000.0,
            y: 2000.0,
            width: 320.0,
            height: 200.0,
        }
        .sanitize(1024.0, 768.0)
        .is_err());
    }

    /// Both dock owners give their band back through the same rule: the common right column keeps
    /// its top, right and bottom edges with the window, and a floating dock keeps all of its input.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_dock_edge_on_the_window_goes_back_to_the_window() {
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

        let floating = band_for(
            DockBounds {
                x: 400.0,
                y: 40.0,
                width: 300.0,
                height: 400.0,
            },
            1000.0,
            740.0,
        );
        assert!(!floating.left);
        assert!(!floating.top);
        assert!(!floating.right);
        assert!(!floating.bottom);
    }

    /// Guard for the input path: a native view is a bounded overlay child aligned to the start,
    /// never a full-area container that would swallow every pointer event of the window.
    #[test]
    fn a_native_view_is_a_bounded_overlay_child() {
        let source = production_source();

        assert!(source.contains("surface.overlay.add_overlay(dock_holder.container())"));
        assert!(source.contains("holder.set_halign(gtk::Align::Start)"));
        assert!(source.contains("holder.set_valign(gtk::Align::Start)"));
        assert!(source.contains("holder.set_margin_start(bounds.x.round() as i32)"));
        assert!(source.contains("holder.set_margin_top(bounds.y.round() as i32)"));
        assert!(source.contains("bounds.width.round() as i32"));
        // Nothing expands to the whole window.
        assert!(!source.contains("set_hexpand(true)"));
        assert!(!source.contains("set_vexpand(true)"));
        assert!(!source.contains("Fixed::new"));
    }

    /// Guard for the cleanup path: one owner is cleared without touching the other, and the
    /// overlay and the wrapped content are only released when the last view goes away.
    #[test]
    fn an_owner_is_cleared_without_touching_the_other_one() {
        let source = production_source();

        assert!(
            source.contains("pub fn remove_owner(&self, app: &AppHandle<Wry>, owner: DockOwner)")
        );
        assert!(source.contains("filter(|(holder_owner, _)| *holder_owner == owner)"));
        assert!(source.contains("if !self.holders.is_empty() || self.overlay.is_none()"));
        assert!(source.contains("vbox.reorder_child(&surface.content, 0)"));
    }

    /// A radius the frontend caught mid-frame is refused instead of reaching a native view or the
    /// shared script.
    #[test]
    fn a_corner_radius_is_clamped_to_a_finite_length() {
        assert_eq!(clamp_corner_radius(f64::NAN), 0.0);
        assert_eq!(clamp_corner_radius(f64::INFINITY), 0.0);
        assert_eq!(clamp_corner_radius(-5.0), 0.0);
        assert_eq!(clamp_corner_radius(12.0), 12.0);
        assert_eq!(clamp_corner_radius(90.0), MAX_PAGE_CORNER_RADIUS);
    }

    /// The shared script finds the corner by geometry and carries its radius through the runtime
    /// setter, so a decorative layer that never shows up in a hit test is still rounded and a later
    /// radius can move the corner without reinstalling the listeners.
    #[test]
    fn the_bottom_right_corner_script_carries_the_radius_and_the_layer_rules() {
        let script = bottom_right_corner_script(15.0);

        // The initial radius reaches the page through the same setter the update path uses.
        assert!(script.contains("state.set(15);"));
        assert!(script.contains("state.set = function (value)"));
        // The state and the listeners are installed once, so a second evaluation cannot stack
        // another set of them.
        assert!(script.contains("window[KEY] = state;"));
        assert!(script.contains("if (!state.installed)"));
        assert!(script.contains("document.addEventListener('DOMContentLoaded', roundCorner)"));
        assert!(script.contains("window.addEventListener('load', roundCorner)"));
        assert!(script.contains("border-bottom-right-radius"));
        assert!(script.contains("document.querySelectorAll('*')"));
        assert!(script.contains("getBoundingClientRect"));
        assert!(script.contains("'data-cs-corner'"));
        assert!(script.contains("'::after' : '::before'"));
        // The script reaches the page as JavaScript, so no formatting brace may survive in it.
        assert!(!script.contains("{{"));
        assert!(!script.contains("}}"));
    }

    /// A radius of zero removes every radius the script applied and restores what the page had
    /// before, so a window that becomes square again gives its corner back.
    #[test]
    fn a_zero_radius_removes_the_applied_corner() {
        let script = bottom_right_corner_script(0.0);

        assert!(script.contains("function restore()"));
        assert!(script.contains("removeProperty('border-bottom-right-radius')"));
        assert!(script.contains("removeAttribute('data-cs-corner')"));
        // A nonsense radius is refused before it reaches the page.
        assert!(bottom_right_corner_script(f64::NAN).contains("state.set(0);"));
        assert!(bottom_right_corner_script(90.0).contains("state.set(30);"));
    }

    /// The update script only drives the setter the installer exposed, so a radius change never
    /// reloads the page and never connects a second set of listeners.
    #[test]
    fn the_update_script_only_calls_the_runtime_setter() {
        assert_eq!(
            bottom_right_corner_update_script(15.0),
            "window.__csCorner && window.__csCorner.set(15);"
        );
        assert_eq!(
            bottom_right_corner_update_script(f64::NAN),
            "window.__csCorner && window.__csCorner.set(0);"
        );
        assert!(bottom_right_corner_update_script(90.0).contains("set(30);"));
    }

    /// Guard for the native shaping path: the shape targets the window of the view itself, which
    /// must own it, and is reapplied when the view is realized or reallocated, never the shared
    /// toplevel window.
    #[test]
    fn native_rounding_targets_the_view_window_and_is_reapplied() {
        let source = production_source();

        assert!(source.contains("GdkX11Display"));
        // The view must own its window: a windowless widget reports the window of a container it
        // was added to, and shaping that shared window would clip the whole window.
        assert!(source.contains("if !widget.has_window()"));
        assert!(!source.contains("toplevel.window()"));
        // The reshape callbacks are connected once, when the view shape is installed.
        assert!(source.contains("shape.widget.connect_realize"));
        assert!(source.contains("connect_size_allocate"));
        assert!(source.contains("let on_realize = Rc::downgrade(&shape)"));
        assert!(source.contains("let on_allocate = Rc::downgrade(&shape)"));
        assert!(!source.contains("let on_realize = shape.clone()"));
        assert!(!source.contains("let on_allocate = shape.clone()"));
        assert!(source
            .contains("pub fn install<W: gtk::glib::IsA<gtk::Widget>>(widget: &W) -> Rc<Self>"));
        // The state is one cell per view, so a later show updates the geometry the view was built
        // with instead of keeping it.
        assert!(source.contains("radius: Cell<f64>"));
        assert!(source.contains("band: Cell<crate::frame_edges::Band>"));
        assert!(source.contains("pub fn set(&self, radius: f64, band: crate::frame_edges::Band)"));
        // The shape of the previous allocation is cleared before the new one is applied, which is
        // also how a radius that drops to zero gives the square corner back.
        assert!(source.contains("window.shape_combine_region(None, 0, 0)"));
        assert!(source.contains("window.shape_combine_region(Some(&shaped), 0, 0)"));
    }

    /// The native shape uses the radius in logical window coordinates: GDK scales the region to
    /// device pixels itself, so the scale factor must not be applied again here.
    #[test]
    fn the_native_shape_keeps_the_radius_in_logical_coordinates() {
        let source = production_source();
        let shape = block(
            source,
            "fn apply_bottom_right",
            "Owns every bounded native dock view",
        );

        assert!(!shape.contains("scale_factor"));
        assert!(shape.contains("rounded_rectangles(width, height, radius)"));
    }
}
