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
//!
//! The dock is a column on the right edge of the window, so its bottom-right corner is the window's
//! own rounded corner. On the backend that owns a window shape, the surface cuts that corner out of
//! the window itself, which is what keeps a native view from painting over the rounded frame the
//! window draws.

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

/// Largest corner radius a docked view is cut at, so a bad measurement cannot eat into the view.
///
/// The radius reaches the native side from the frontend's own measurement of the frame the window
/// draws, so it bounds how much of a native dock view may be cut away at the window's bottom-right
/// corner.
pub const MAX_PAGE_CORNER_RADIUS: f64 = 30.0;

/// Clamps a requested corner radius to a finite length a window can draw.
///
/// The radius is measured by the frontend, so a value caught mid-frame (or a nonsense one) is
/// refused instead of being handed to a window shape.
pub fn clamp_corner_radius(radius: f64) -> f64 {
    if radius.is_finite() {
        radius.clamp(0.0, MAX_PAGE_CORNER_RADIUS)
    } else {
        0.0
    }
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
pub use linux::ViewShape;
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
        /// Radius the window cuts its bottom-right corner at, shared with the reshape callback.
        ///
        /// A window shape is in window coordinates, so it has to be applied again whenever the
        /// window is allocated rather than only when a dock view is placed.
        corner: Rc<Cell<f64>>,
    }

    // SAFETY: the handles are GTK objects that may only be touched on the main thread. Every entry
    // point of the dock surface runs there, posted by the external commands.
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

    /// Whether the GDK backend can clip the window to a shape.
    ///
    /// Only the X11 backend owns the X window a shape is applied to. A Wayland surface has no
    /// window shape, so the rounded frame is whatever the window paints there.
    pub fn supports_native_rounding() -> bool {
        gtk::gdk::Display::default()
            .map(|display| &*display.type_().name() == "GdkX11Display")
            .unwrap_or(false)
    }

    /// Main-thread state one docked native view keeps for its frame band and its corner.
    ///
    /// The GTK callbacks that reshape a view are connected once, when the view is built, and read
    /// this state on every allocation. A later show that only changes the corner radius or the band
    /// updates the state and re-applies it, so a reused ChatHub tab or plugin panel follows the
    /// frontend instead of keeping the geometry it was first built with, and no reshape callback is
    /// ever connected twice. A radius that drops to zero re-applies the same state, and the input
    /// region of the view then keeps the square corner without rebuilding the view.
    pub struct ViewShape {
        widget: gtk::Widget,
        radius: Cell<f64>,
        band: Cell<crate::frame_edges::Band>,
    }

    impl ViewShape {
        /// Connects the reshape callbacks once and returns the shared state.
        ///
        /// The widget must be the native view itself, not the holder it is placed in: the input
        /// region is measured from the view's allocation, so a widget that does not own the region
        /// it was given is left alone.
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

        /// Reshapes the view from the state it currently holds.
        ///
        /// Only the input region is reshaped here, so a press on the rounded window corner reaches
        /// the window instead of the view. The corner itself is cut out of the window by
        /// [`DockSurface::set_corner_radius`], which is what makes it come from the window rather
        /// than from the view.
        fn apply(&self) {
            crate::frame_edges::apply_frame_band_and_corner(
                &self.widget,
                self.band.get(),
                self.radius.get(),
            );
        }
    }

    /// Cuts the rounded bottom-right corner of the window out of a dock view.
    ///
    /// A dock view is a rectangle, so it would paint over the rounded frame the window draws at its
    /// bottom-right corner. The shape is applied to the window the view is in rather than to the
    /// view itself, which is what makes the corner come from the window: the frame keeps its own
    /// radius, and every native view inside the window is cut by the same shape, whatever widget
    /// owns which `GdkWindow`.
    ///
    /// Only the X11 backend owns a window shape; a Wayland surface has none, and the corner there
    /// stays whatever the window paints. The shape of the previous size is cleared first, which is
    /// also how a radius that drops to zero gives the square corner back.
    fn apply_window_corner<W: gtk::glib::IsA<gtk::Widget>>(widget: &W, radius: f64) {
        let Some(toplevel) = widget.toplevel() else {
            return;
        };
        let Some(window) = toplevel.window() else {
            return;
        };

        // A shape is combined with the one already set, so the shape of the previous size has to go
        // first: a window that grew would otherwise keep the narrower corner it had, and a window
        // whose radius dropped to zero would keep a corner it no longer draws.
        window.shape_combine_region(None, 0, 0);

        if !supports_native_rounding() || !(radius > 0.0) {
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

        /// Cuts the window's bottom-right corner at `radius`, or gives the square corner back for zero.
        ///
        /// The dock is a column on the right edge of the window, so the corner a dock view would
        /// paint over is the window's own: `radius` is the radius the frontend measured on the frame
        /// the window draws, and the shape is what keeps that frame visible under a view. A backend
        /// without a window shape keeps the window as it paints it.
        pub fn set_corner_radius(&self, app: &AppHandle<Wry>, radius: f64) -> Result<()> {
            let mut registry = self.lock()?;
            // The shape targets the window of the overlay, so the overlay is installed here when a
            // caller sets a radius before the first view opened.
            registry.install(app)?;

            let Some(surface) = registry.overlay.as_ref() else {
                return Ok(());
            };
            surface.corner.set(radius);
            apply_window_corner(&surface.overlay, radius);

            Ok(())
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

            // The window shape belongs to the window, so it is applied from the overlay that spans
            // it: a resize moves the rounded corner with the window, and the callback follows.
            let corner = Rc::new(Cell::new(0.0));
            let on_allocate = Rc::downgrade(&corner);
            overlay.connect_size_allocate(move |overlay, _allocation| {
                if let Some(corner) = on_allocate.upgrade() {
                    apply_window_corner(overlay, corner.get());
                }
            });

            self.overlay = Some(Surface {
                overlay,
                content,
                corner,
            });
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

            // No native view is left to paint over the window frame, so the window keeps its own
            // rounded corner instead of the shape a dock view was cut with.
            surface.corner.set(0.0);
            apply_window_corner(&surface.overlay, 0.0);

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

    /// Guard for the round window corner: the shape is cut out of the window itself, so a dock view
    /// cannot paint over the rounded frame whatever widget owns which `GdkWindow`, and the corner
    /// follows the window rather than a view.
    #[test]
    fn the_window_corner_is_cut_out_of_the_window_itself() {
        let source = production_source();

        assert!(source.contains("GdkX11Display"));
        // The shape targets the window the view is in, never the view's own window: a rectangular
        // dock view would otherwise paint over the frame the window draws.
        assert!(source.contains("let Some(toplevel) = widget.toplevel()"));
        assert!(source.contains("let Some(window) = toplevel.window()"));
        assert!(!source.contains("widget.has_window()"));
        assert!(!source.contains("apply_bottom_right"));
        // The shape belongs to the window, so it is applied from the overlay that spans the window
        // and again whenever that window is allocated.
        assert!(source
            .contains("pub fn set_corner_radius(&self, app: &AppHandle<Wry>, radius: f64)"));
        assert!(source.contains("overlay.connect_size_allocate(move |overlay, _allocation|"));
        assert!(source.contains("apply_window_corner(overlay, corner.get())"));
        // No native view is left to paint over the frame, so the window keeps its own corner.
        assert!(source.contains("surface.corner.set(0.0)"));
        assert!(source.contains("apply_window_corner(&surface.overlay, 0.0)"));
    }

    /// The corner is cut in the logical window coordinates a shape region uses: GDK scales the
    /// region to device pixels itself, so the scale factor must not be applied again here.
    #[test]
    fn the_window_corner_keeps_the_radius_in_logical_coordinates() {
        let source = production_source();
        let shape = block(
            source,
            "fn apply_window_corner",
            "Owns every bounded native dock view",
        );

        assert!(!shape.contains("scale_factor"));
        assert!(shape.contains("rounded_rectangles(width, height, radius)"));
    }

    /// Guard for the input path: only the input region stays with the view, so a press on the
    /// rounded window corner reaches the window while the corner itself comes from the window.
    #[test]
    fn the_view_reshapes_only_its_input_region() {
        let source = production_source();
        let apply = block(source, "fn apply(&self)", "Cuts the rounded bottom-right corner");

        assert!(apply.contains("apply_frame_band_and_corner"));
        assert!(!apply.contains("shape_combine_region"));
        // The reshape callbacks are connected once, when the view shape is installed.
        assert!(source.contains("shape.widget.connect_realize"));
        assert!(source.contains("let on_realize = Rc::downgrade(&shape)"));
        assert!(source.contains("let on_allocate = Rc::downgrade(&shape)"));
        assert!(!source.contains("let on_realize = shape.clone()"));
        assert!(!source.contains("let on_allocate = shape.clone()"));
        // The state is one cell per view, so a later show updates the geometry the view was built
        // with instead of keeping it.
        assert!(source.contains("radius: Cell<f64>"));
        assert!(source.contains("band: Cell<crate::frame_edges::Band>"));
        assert!(source.contains("pub fn set(&self, radius: f64, band: crate::frame_edges::Band)"));
    }
}
