//! Room the Workflow window makes for the docked column.
//!
//! The dock is a column on the right edge of the Workflow window, and every native dock view (a
//! ChatHub page, a plugin UI panel) is painted inside it by its carrier. The frontend reserves
//! that column in its own layout, which on its own narrows the workflow UI by the width of the
//! dock; the window therefore grows by the same width, so the workflow UI keeps exactly the size
//! it had. Closing the dock, or narrowing it with the splitter, hands that width back.
//!
//! The window never grows past the work area of the screen it is on. A window that would stick
//! out is capped at the work area and moved left until its right edge lands on the right edge of
//! the work area, so making room can never push a part of the window out of view. A window that
//! already fills its work area adds nothing, and the dock then takes its space from the workflow
//! UI instead.
//!
//! The width the window currently holds for the dock is kept in [`DockWindowState`]. It is what
//! the window hands back when the dock goes away, and what a remembered window size leaves out
//! (see `width_without_docked_page`), so reopening the application never restores a window that
//! is wider than the workflow UI ever was.

use std::sync::Mutex;

use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, WebviewWindow, Wry};

use crate::chat_hub::host_window;
use crate::error::{AppError, Result};

/// Smallest window change worth a resize, in logical pixels.
///
/// A window that already fills its work area has nothing to add, and a rounding residue must not
/// send a resize the platform would animate.
const MIN_WINDOW_CHANGE: f64 = 0.5;

/// Room the screen gives the window, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenArea {
    /// Left edge of the work area, in logical pixels.
    pub left: f64,
    /// Width of the work area, in logical pixels.
    pub width: f64,
}

/// Rectangle the window takes so the workflow UI keeps its width next to the dock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HostWindowGeometry {
    /// Left edge, in logical pixels.
    pub left: f64,
    /// Width, in logical pixels.
    pub width: f64,
}

/// Rectangle the window needs to hold a dock column `added` wider than it currently is.
///
/// The dock sits on the right edge of the window, so the width it needs is added there: the
/// window keeps its left edge while it still fits, and is only moved when adding the width would
/// push its right edge past the work area.
///
/// `None` means the window cannot take that width without giving up workflow width, which is the
/// case when it already fills its work area.
pub fn room_for_dock(
    window_left: f64,
    window_width: f64,
    screen: ScreenArea,
    added: f64,
) -> Option<HostWindowGeometry> {
    let measured = window_left.is_finite() && window_width.is_finite() && added.is_finite();
    if !measured || added <= 0.0 || !screen.width.is_finite() || screen.width <= 0.0 {
        return None;
    }

    let width = (window_width + added).min(screen.width);
    if width - window_width < MIN_WINDOW_CHANGE {
        return None;
    }

    let left = window_left
        .min(screen.left + screen.width - width)
        .max(screen.left);

    Some(HostWindowGeometry { left, width })
}

/// Change the window has to make for a dock that asks for `requested` while holding `held`.
///
/// `None` means the window has nothing to do: the dock already holds that width, so a repeated
/// show cannot widen the window a second time, and a residue smaller than a pixel is not worth a
/// resize the platform would animate.
fn dock_change(held: f64, requested: f64) -> Option<f64> {
    let delta = requested - held;
    if !delta.is_finite() || delta.abs() < MIN_WINDOW_CHANGE {
        return None;
    }

    Some(delta)
}

/// State of the room the Workflow window currently makes for the dock.
///
/// The width is held per window rather than per dock view, because the dock is one column shared
/// by every provider: which tab is docked changes what is painted inside it, never how wide it
/// is. Every entry point runs on the main thread, where the window is touched.
#[derive(Debug, Default)]
pub struct DockWindowState {
    grown: Mutex<f64>,
}

impl DockWindowState {
    /// Creates the state of a window that holds no dock.
    pub fn new() -> Self {
        Self::default()
    }

    /// Width the dock currently holds in the window, in logical pixels.
    ///
    /// A poisoned lock reports no width at all, which only leaves a remembered window size as it
    /// was measured.
    pub fn grown(&self) -> f64 {
        self.grown.lock().map(|grown| *grown).unwrap_or(0.0)
    }

    /// Makes the window hold a dock column `docked_width` wide, or hands the width back for zero.
    ///
    /// The window only moves by the difference between the width the dock asks for and the width
    /// it already holds, so a repeated request is a no-op and dragging the splitter moves the
    /// window by exactly what changed. A window that cannot take the width keeps its current
    /// geometry, and the dock takes that part from the workflow UI instead.
    pub fn set_docked_width(&self, app: &AppHandle<Wry>, docked_width: f64) -> Result<()> {
        if !docked_width.is_finite() || docked_width < 0.0 {
            return Ok(());
        }

        let host = host_window(app)?;
        let mut grown = self.grown.lock().map_err(|_| AppError::General {
            message: "the dock window state is poisoned".to_string(),
        })?;

        let delta = match dock_change(*grown, docked_width) {
            Some(delta) => delta,
            None => return Ok(()),
        };

        let scale_factor = host.scale_factor()?;
        let window_size = host.inner_size()?.to_logical::<f64>(scale_factor);
        let position = host.outer_position()?.to_logical::<f64>(scale_factor);

        if delta < 0.0 {
            // Handing the width back only undoes what the window took for the dock, so the left
            // edge stays where making room put it.
            let width = window_size.width + delta;
            if width < MIN_WINDOW_CHANGE {
                return Ok(());
            }

            host.set_size(tauri::Size::Logical(LogicalSize::new(
                width,
                window_size.height,
            )))?;
            *grown = (*grown + delta).max(0.0);

            return Ok(());
        }

        let Some(screen) = screen_area(&host, scale_factor) else {
            return Ok(());
        };
        let Some(geometry) = room_for_dock(position.x, window_size.width, screen, delta) else {
            return Ok(());
        };

        host.set_size(tauri::Size::Logical(LogicalSize::new(
            geometry.width,
            window_size.height,
        )))?;

        // The window is only moved when the dock would have pushed its right edge off the screen.
        if (geometry.left - position.x).abs() >= MIN_WINDOW_CHANGE {
            host.set_position(tauri::Position::Logical(LogicalPosition::new(
                geometry.left,
                position.y,
            )))?;
        }

        // A work area that could not give the whole width leaves the rest to the workflow UI, so
        // the state records what the window actually took.
        *grown += geometry.width - window_size.width;

        Ok(())
    }
}

/// Makes the Workflow window hold a dock column `width` wide, from any thread.
///
/// The window geometry is read and changed on the thread that owns the window, so the work is
/// posted there and its result awaited: the frontend then learns about a failure immediately
/// instead of the window silently staying the size it was.
pub async fn apply_docked_width(app: &AppHandle<Wry>, width: f64) -> Result<()> {
    let (sender, receiver) = std::sync::mpsc::channel();
    let thread_app = app.clone();

    app.run_on_main_thread(move || {
        let result = thread_app
            .try_state::<DockWindowState>()
            .ok_or_else(|| AppError::General {
                message: "the dock window state is not managed".to_string(),
            })
            .and_then(|state| state.inner().set_docked_width(&thread_app, width));
        let _ = sender.send(result);
    })
    .map_err(|error| AppError::General {
        message: format!("the dock window work could not be posted: {error}"),
    })?;

    match receiver.recv() {
        Ok(result) => result,
        Err(_) => Err(AppError::General {
            message: "the dock window work did not run".to_string(),
        }),
    }
}

/// Work area of the screen the window is on, in logical pixels.
///
/// The work area is used instead of the full resolution so the window never grows under a panel
/// or over a dock the user keeps at the side of the screen.
fn screen_area(host: &WebviewWindow<Wry>, scale_factor: f64) -> Option<ScreenArea> {
    let monitor = host.current_monitor().ok().flatten()?;
    let work_area = monitor.work_area();
    let left = work_area.position.x as f64 / scale_factor;
    let width = work_area.size.width as f64 / scale_factor;

    if !left.is_finite() || !width.is_finite() || width <= 0.0 {
        return None;
    }

    Some(ScreenArea { left, width })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> ScreenArea {
        ScreenArea {
            left: 0.0,
            width: 1920.0,
        }
    }

    /// The dock takes the added width at the right edge of the window, so the workflow UI keeps
    /// the width it had and a window that still fits keeps its place.
    #[test]
    fn the_window_grows_to_the_right_for_the_dock() {
        assert_eq!(
            room_for_dock(100.0, 1215.0, screen(), 600.0),
            Some(HostWindowGeometry {
                left: 100.0,
                width: 1815.0,
            })
        );
        // A window whose right edge lands on the edge of the work area still fits exactly.
        assert_eq!(
            room_for_dock(120.0, 1200.0, screen(), 600.0),
            Some(HostWindowGeometry {
                left: 120.0,
                width: 1800.0,
            })
        );
    }

    /// A window that would stick out is capped at the work area and moved left until its right edge
    /// lands on the edge of the work area, so making room can never push a part of it out of view.
    #[test]
    fn a_window_that_would_stick_out_is_moved_back_onto_the_screen() {
        assert_eq!(
            room_for_dock(1000.0, 1200.0, screen(), 600.0),
            Some(HostWindowGeometry {
                left: 120.0,
                width: 1800.0,
            })
        );
        // A narrow window that sits at the edge of the work area moves left by exactly what the
        // dock adds, and no further.
        assert_eq!(
            room_for_dock(1320.0, 600.0, screen(), 600.0),
            Some(HostWindowGeometry {
                left: 720.0,
                width: 1200.0,
            })
        );
    }

    /// A window that already fills its work area makes no room, so the dock takes its space from
    /// the workflow UI exactly like it did before.
    #[test]
    fn a_window_that_fills_the_screen_makes_no_room() {
        assert_eq!(room_for_dock(0.0, 1920.0, screen(), 600.0), None);
        // A window that is already wider than the work area is never narrowed by this rule.
        assert_eq!(room_for_dock(-100.0, 2020.0, screen(), 600.0), None);
    }

    /// A measurement that cannot describe a window is refused instead of being applied.
    #[test]
    fn unusable_measurements_leave_the_window_alone() {
        assert_eq!(room_for_dock(f64::NAN, 1200.0, screen(), 600.0), None);
        assert_eq!(room_for_dock(0.0, f64::NAN, screen(), 600.0), None);
        assert_eq!(room_for_dock(0.0, 1200.0, screen(), f64::NAN), None);
        assert_eq!(room_for_dock(0.0, 1200.0, screen(), 0.0), None);
        assert_eq!(room_for_dock(0.0, 1200.0, screen(), -600.0), None);
        // A screen that reports no width cannot give the dock room.
        assert_eq!(
            room_for_dock(
                0.0,
                1200.0,
                ScreenArea {
                    left: 0.0,
                    width: 0.0
                },
                600.0
            ),
            None
        );
    }

    /// A work area that is narrower than the window is still respected: the window is capped at
    /// it, never grown past it.
    #[test]
    fn the_window_is_never_wider_than_the_work_area() {
        let geometry = room_for_dock(
            100.0,
            1200.0,
            ScreenArea {
                left: 100.0,
                width: 1300.0,
            },
            600.0,
        )
        .expect("a work area that gives the window room");

        assert_eq!(geometry.width, 1300.0);
        assert_eq!(geometry.left, 100.0);
        assert!(geometry.left + geometry.width <= 1400.0);
    }

    /// The state holds no width before a dock opens, so a remembered window size is measured as it is
    /// until the frontend makes room for the dock.
    #[test]
    fn the_state_holds_no_dock_width_before_a_dock_opens() {
        assert_eq!(DockWindowState::new().grown(), 0.0);
    }

    /// A dock only moves the window by what changed, so a repeated show cannot widen it twice and a
    /// narrower dock hands back exactly the width it no longer needs.
    #[test]
    fn a_dock_moves_the_window_only_by_what_changed() {
        assert_eq!(dock_change(0.0, 600.0), Some(600.0));
        assert_eq!(dock_change(600.0, 600.0), None);
        assert_eq!(dock_change(600.0, 380.0), Some(-220.0));
        assert_eq!(dock_change(380.0, 0.0), Some(-380.0));
        // A residue smaller than a pixel is not worth a resize the platform would animate.
        assert_eq!(dock_change(600.0, 600.2), None);
        assert_eq!(dock_change(600.0, f64::NAN), None);
    }

    /// Guard for the window geometry the state applies: making room and handing it back are the two
    /// halves of one rule, and the state records what the window actually took.
    #[test]
    fn the_window_is_only_moved_by_the_shared_dock_rule() {
        let source = include_str!("dock_window.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("dock_window.rs carries a test module");

        assert!(source.contains("let delta = match dock_change(*grown, docked_width) {"));
        assert!(source.contains("room_for_dock(position.x, window_size.width, screen, delta)"));
        // A work area that could not give the whole width leaves the rest to the workflow UI.
        assert!(source.contains("*grown += geometry.width - window_size.width;"));
        // Handing the width back is bounded by what the dock took.
        assert!(source.contains("*grown = (*grown + delta).max(0.0);"));
    }
}