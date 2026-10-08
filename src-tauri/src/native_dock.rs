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
pub use linux::{DockHolder, DockSurface};

#[cfg(target_os = "linux")]
mod linux {
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
}
