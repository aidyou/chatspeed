//! Platform carrier of the docked Agent Skills surface.
//!
//! The trusted surface is a Tauri child webview of the Workflow window, placed inside the right
//! dock. Every entry point runs on the platform main thread.

/// Linux carrier: the child webview is moved into a bounded holder of the shared dock surface.
///
/// The same-window child API builds the webview into the window's default `GtkBox`, which ignores
/// a rectangle, so the inner `webkit2gtk::WebView` is taken out of that box and packed into a
/// holder aligned to the start of the shared overlay. That holder is what bounds the surface to
/// the measured rectangle and keeps the workflow UI receiving every pointer event outside it.
#[cfg(target_os = "linux")]
mod imp {
    use std::rc::Rc;
    use std::sync::mpsc;

    use gtk::prelude::*;
    use tauri::{AppHandle, LogicalPosition, LogicalSize, Webview, Wry};

    use crate::frame_edges::Band;
    use crate::native_dock::{self, DockBounds, DockHolder, DockOwner, ViewShape};

    use super::super::view::{agent_skills_builder, host_tauri_window};

    /// The docked Agent Skills webview: a Tauri webview inside a bounded holder of the dock.
    pub(crate) struct Page {
        webview: Webview<Wry>,
        holder: DockHolder,
        /// Main-thread state that reshapes the webview for the band and radius it is shown at.
        shape: Rc<ViewShape>,
    }

    // SAFETY: the webview and the GTK holder may only be touched on the main thread, and every
    // entry point of the Agent Skills view runs there.
    unsafe impl Send for Page {}

    impl Page {
        /// Builds the trusted surface into its own bounded holder at the measured rectangle.
        pub(crate) fn create(
            app: &AppHandle<Wry>,
            bounds: DockBounds,
            radius: f64,
        ) -> Result<Self, String> {
            let band = band_for(app, bounds)?;
            // The dock is a column on the right edge of the window, so its bottom-right corner is
            // the window's own; the shape keeps the rounded frame visible under the surface.
            native_dock::with_surface(app, |surface| surface.set_corner_radius(app, radius))
                .map_err(|error| error.to_string())?;

            let holder = native_dock::with_surface(app, |surface| {
                surface.holder(app, DockOwner::AgentSkills, bounds)
            })
            .map_err(|error| error.to_string())?;
            // The holder is kept out of the window until its webview is attached, so an empty
            // input region never takes focus while the child is being built; `move_into_holder`
            // shows it again once the webview is packed inside.
            holder.set_visible(false);

            // The surface is a Tauri webview, so it receives the desktop IPC; the child API builds
            // it into the window box below, where it is moved into the bounded holder.
            let window = host_tauri_window(app)?;
            let webview = match window.add_child(
                agent_skills_builder(app),
                LogicalPosition::new(bounds.x, bounds.y),
                LogicalSize::new(bounds.width, bounds.height),
            ) {
                Ok(webview) => webview,
                Err(error) => {
                    remove_owner(app);
                    return Err(format!("failed to create the Agent Skills webview: {error}"));
                }
            };

            // The child webview now exists, so every later failure has to close it as well as
            // detach its holder: a live webview whose handle was dropped would keep its label taken
            // and leak its process. `with_webview` runs on the main thread, and this call is
            // already on it, so the callback settles before it returns and the shape can be
            // received right after; the holder travels back through the same channel.
            let (sender, receiver) = mpsc::channel();
            if let Err(error) = webview.with_webview(move |platform| {
                let shape = move_into_holder(platform.inner(), &holder, radius, band);
                let _ = sender.send((holder, shape));
            }) {
                return Err(rollback(
                    app,
                    &webview,
                    format!("failed to attach the Agent Skills webview: {error}"),
                ));
            }

            let (holder, shape) = match receiver.recv() {
                Ok(received) => received,
                Err(_) => {
                    return Err(rollback(
                        app,
                        &webview,
                        "the Agent Skills webview did not report its attachment".to_string(),
                    ));
                }
            };
            let shape = match shape {
                Ok(SendShape(shape)) => shape,
                Err(error) => {
                    return Err(rollback(app, &webview, error));
                }
            };

            Ok(Self {
                webview,
                holder,
                shape,
            })
        }

        /// Places the surface at `bounds` and re-applies the corner radius and the frame band.
        pub(crate) fn show(
            &self,
            app: &AppHandle<Wry>,
            bounds: DockBounds,
            radius: f64,
        ) -> Result<(), String> {
            let band = band_for(app, bounds)?;
            native_dock::with_surface(app, |surface| surface.set_corner_radius(app, radius))
                .map_err(|error| error.to_string())?;

            self.holder.set_bounds(bounds);
            self.shape.set(radius, band);
            self.holder.set_visible(true);

            Ok(())
        }

        pub(crate) fn set_visible(&self, visible: bool) -> Result<(), String> {
            self.holder.set_visible(visible);

            Ok(())
        }

        /// Closes the child webview, then detaches its holder.
        ///
        /// The child is closed first, so releasing the holder never disturbs the window content
        /// the shared overlay wraps; the Workflow window itself is never touched.
        pub(crate) fn destroy(&self, app: &AppHandle<Wry>) -> Result<(), String> {
            self.webview
                .close()
                .map_err(|error| format!("failed to close the Agent Skills view: {error}"))?;
            native_dock::with_surface(app, |surface| {
                surface.remove(app, &self.holder);
                Ok(())
            })
            .map_err(|error| error.to_string())
        }
    }

    /// The frame band of the rectangle the frontend measured, through the rule both other dock
    /// owners use.
    fn band_for(app: &AppHandle<Wry>, bounds: DockBounds) -> Result<Band, String> {
        let window = host_tauri_window(app)?;
        let (width, height) = native_dock::window_size(&window).map_err(|error| error.to_string())?;

        Ok(native_dock::band_for(bounds, width, height))
    }

    /// Detaches every holder of this owner, so a failed creation leaves no overlay child behind.
    fn remove_owner(app: &AppHandle<Wry>) {
        let _ = native_dock::with_surface(app, |surface| {
            surface.remove_owner(app, DockOwner::AgentSkills);
            Ok(())
        });
    }

    /// Closes a webview whose creation did not finish, then detaches the holders of its owner.
    ///
    /// The original failure is returned even when the rollback itself fails, so the caller learns
    /// why the surface never opened; a failed close is logged instead of masking it. Like a normal
    /// close, this closes only the child and never the parent Workflow window.
    fn rollback(app: &AppHandle<Wry>, webview: &Webview<Wry>, failure: String) -> String {
        if let Err(error) = webview.close() {
            log::warn!(
                "Agent Skills failed to close a half-created webview during rollback: {}",
                error
            );
        }
        remove_owner(app);

        failure
    }

    /// Sends a main-thread-only dock shape from the `with_webview` callback back to `create`.
    struct SendShape(Rc<ViewShape>);

    // SAFETY: the shape wraps a GTK widget that may only be touched on the main thread; both the
    // sending callback and the receiving `create` run on that thread.
    unsafe impl Send for SendShape {}

    /// Moves the inner GTK webview into the bounded holder and installs its reshape state.
    fn move_into_holder<W: gtk::glib::IsA<gtk::Widget>>(
        inner: W,
        holder: &DockHolder,
        radius: f64,
        band: Band,
    ) -> Result<SendShape, String> {
        if let Some(parent) = inner.parent() {
            if let Some(container) = parent.downcast_ref::<gtk::Container>() {
                container.remove(&inner);
            }
        }
        holder.container().pack_start(&inner, true, true, 0);
        // The holder is shown only once the webview is packed inside it, so it is never visible as
        // an empty input region while the child is built.
        holder.set_visible(true);

        let shape = ViewShape::install(&inner);
        shape.set(radius, band);

        Ok(SendShape(shape))
    }
}

/// Windows and macOS carrier: the child view is placed at the rectangle the frontend measured.
#[cfg(not(target_os = "linux"))]
mod imp {
    use tauri::{AppHandle, LogicalPosition, LogicalSize, Webview, Wry};

    use crate::native_dock::DockBounds;

    use super::super::view::{agent_skills_builder, host_tauri_window};

    /// The docked Agent Skills webview, a child view of the Workflow window.
    pub(crate) struct Page {
        webview: Webview<Wry>,
    }

    // SAFETY: a Tauri webview may only be touched on the main thread, and every entry point of the
    // Agent Skills view runs there.
    unsafe impl Send for Page {}

    impl Page {
        pub(crate) fn create(
            app: &AppHandle<Wry>,
            bounds: DockBounds,
            _radius: f64,
        ) -> Result<Self, String> {
            // A child view is placed at its rectangle and the window keeps its own corners, so a
            // corner radius is not needed here; the surface stays opaque.
            let window = host_tauri_window(app)?;
            let webview = window
                .add_child(
                    agent_skills_builder(app),
                    LogicalPosition::new(bounds.x, bounds.y),
                    LogicalSize::new(bounds.width, bounds.height),
                )
                .map_err(|error| format!("failed to create the Agent Skills webview: {error}"))?;

            Ok(Self { webview })
        }

        pub(crate) fn show(
            &self,
            _app: &AppHandle<Wry>,
            bounds: DockBounds,
            _radius: f64,
        ) -> Result<(), String> {
            self.webview
                .set_position(LogicalPosition::new(bounds.x, bounds.y))
                .map_err(|error| format!("failed to place the Agent Skills view: {error}"))?;
            self.webview
                .set_size(LogicalSize::new(bounds.width, bounds.height))
                .map_err(|error| format!("failed to size the Agent Skills view: {error}"))?;
            self.webview
                .show()
                .map_err(|error| format!("failed to show the Agent Skills view: {error}"))
        }

        pub(crate) fn set_visible(&self, visible: bool) -> Result<(), String> {
            let result = if visible {
                self.webview.show()
            } else {
                self.webview.hide()
            };
            result.map_err(|error| format!("failed to toggle the Agent Skills view: {error}"))
        }

        pub(crate) fn destroy(&self, _app: &AppHandle<Wry>) -> Result<(), String> {
            // A child view is destroyed with its handle, and closing it never touches the parent
            // Workflow window.
            self.webview
                .close()
                .map_err(|error| format!("failed to close the Agent Skills view: {error}"))
        }
    }
}

pub(crate) use imp::Page;