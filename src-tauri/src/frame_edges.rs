//! Frame bands a webview gives back to the window it fills.
//!
//! Every ChatSpeed window is a webview on the whole window area, and on Linux WebKitGTK
//! claims the pointer events that reach it: that is what makes the page interactive at all,
//! but it also means the band along the window frame never arrives at a GTK event handler.
//! Neither the resize handler tao installs on the window (`button-press-event` ->
//! `begin_resize_drag`) nor the one tauri installs on the webview can see the press, so
//! dragging the frame of a frameless window did nothing on Linux.
//!
//! The band is therefore taken out of the input region of the webview instead of being
//! handled in an event handler. The pointer stays over the window there, the window keeps
//! the press, and its own resize path runs again.
//!
//! Only the input region shrinks: the webview keeps its size and still paints the band. The
//! band is measured from the allocation the webview currently has, so it is applied again on
//! every allocation.

use gtk::prelude::WidgetExt;

/// Width of one frame band, in window coordinates.
///
/// A press in it is inside the band the window level resize handler of tao accepts, which
/// measures `scale_factor * 5` from the rectangle of the window.
const FRAME_BAND: i32 = 5;

/// Sides of a webview whose frame band goes back to the window.
#[derive(Clone, Copy)]
pub struct Band {
    /// Left side of the webview.
    pub left: bool,
    /// Right side of the webview.
    pub right: bool,
    /// Top side of the webview.
    pub top: bool,
    /// Bottom side of the webview.
    pub bottom: bool,
}

impl Band {
    /// Every side, for a webview that covers the whole window.
    pub const EVERY_SIDE: Self = Self {
        left: true,
        right: true,
        top: true,
        bottom: true,
    };

    /// Top, right and bottom side of a column that owns the right edge of the window.
    ///
    /// Its left side borders the workflow UI instead of the window frame, where the splitter
    /// between the two webviews lives, so that side keeps its input.
    pub const RIGHT_COLUMN: Self = Self {
        left: false,
        right: true,
        top: true,
        bottom: true,
    };
}

/// The part of a webview that keeps receiving input.
///
/// `None` for a webview too small to hold a band on both sides of an axis, which leaves its
/// input region untouched instead of cutting the whole webview away.
fn inner_rect(width: i32, height: i32, band: Band) -> Option<(i32, i32, i32, i32)> {
    let left = if band.left { FRAME_BAND } else { 0 };
    let right = if band.right { FRAME_BAND } else { 0 };
    let top = if band.top { FRAME_BAND } else { 0 };
    let bottom = if band.bottom { FRAME_BAND } else { 0 };

    let kept_width = width - left - right;
    let kept_height = height - top - bottom;

    (kept_width > 0 && kept_height > 0).then_some((left, top, kept_width, kept_height))
}

/// Gives the frame band of a webview back to the window that hosts it.
pub fn give_frame_band_to_window<W: gtk::glib::IsA<gtk::Widget>>(widget: &W, band: Band) {
    apply_frame_band(widget, band);
    widget.connect_size_allocate(move |widget, _allocation| apply_frame_band(widget, band));
}

/// Removes the frame band from the input region of a webview.
fn apply_frame_band<W: gtk::glib::IsA<gtk::Widget>>(widget: &W, band: Band) {
    // A shape is combined with the one that is already set, so the band of the previous
    // allocation has to go first: a window that grew would otherwise keep the narrower shape
    // it had before.
    widget.input_shape_combine_region(None);

    let Some(window) = widget.window() else {
        return;
    };

    // A window that cannot be resized keeps its band with the page instead of swallowing the
    // press for a drag that never starts.
    let state = window.state();
    if state.contains(gtk::gdk::WindowState::MAXIMIZED)
        || state.contains(gtk::gdk::WindowState::FULLSCREEN)
    {
        return;
    }

    let Some((x, y, width, height)) = inner_rect(window.width(), window.height(), band) else {
        return;
    };

    let region =
        gtk::cairo::Region::create_rectangle(&gtk::cairo::RectangleInt::new(x, y, width, height));
    widget.input_shape_combine_region(Some(&region));
}

#[cfg(test)]
mod tests {
    use super::{inner_rect, Band, FRAME_BAND};

    #[test]
    fn every_side_of_a_full_window_webview_loses_its_frame_band() {
        assert_eq!(
            inner_rect(600, 400, Band::EVERY_SIDE),
            Some((FRAME_BAND, FRAME_BAND, 590, 390))
        );
    }

    #[test]
    fn a_right_column_keeps_its_left_edge_for_the_splitter() {
        assert_eq!(
            inner_rect(300, 400, Band::RIGHT_COLUMN),
            Some((0, FRAME_BAND, 295, 390))
        );
    }

    #[test]
    fn a_webview_that_cannot_hold_a_band_keeps_its_whole_input_region() {
        assert_eq!(inner_rect(FRAME_BAND * 2, 400, Band::EVERY_SIDE), None);
        assert_eq!(inner_rect(600, FRAME_BAND * 2, Band::EVERY_SIDE), None);
        // A side that is not given back does not count against the size of the other axis.
        assert_eq!(inner_rect(FRAME_BAND, 400, Band::RIGHT_COLUMN), None);
    }

    #[test]
    fn the_smallest_webview_that_holds_a_band_keeps_one_point() {
        assert_eq!(
            inner_rect(FRAME_BAND * 2 + 1, FRAME_BAND * 2 + 1, Band::EVERY_SIDE),
            Some((FRAME_BAND, FRAME_BAND, 1, 1))
        );
    }
}