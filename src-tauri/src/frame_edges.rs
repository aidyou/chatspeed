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
//!
//! A rounded bottom-right corner is a hole in the same input region, so it is composed with the
//! band instead of being applied as a second region that would reset it.
//!
//! Both the band and the corner are in the logical window coordinates a GDK region uses, the same
//! coordinates `gdk_window_get_width` and the widget allocation report. GDK scales a region to
//! device pixels on its way to X11 (see the X11 backend's `_gdk_x11_region_get_xrectangles`), so
//! neither value may be multiplied by the scale factor here: a radius scaled once more would be
//! twice as large on a high density display.

use gtk::cairo::{RectangleInt, Region};
use gtk::prelude::WidgetExt;

/// Width of one frame band, in logical window coordinates.
///
/// A press in it is inside the band the window level resize handler of tao accepts, which measures
/// `scale_factor * 5` device pixels from the rectangle of the window, i.e. five logical pixels. The
/// input region this band is cut from is in the same logical coordinates, so the two agree.
const FRAME_BAND: i32 = 5;

/// Largest radius the scanline walks, in logical pixels.
///
/// A docked page never comes close to this: the radius it reports is clamped to
/// [`crate::native_dock::MAX_PAGE_CORNER_RADIUS`] before it reaches this module. The bound exists
/// so a nonsense value cannot ask the scanline for an unbounded allocation or overflow the squared
/// term that positions the arc.
const MAX_SCANLINE_RADIUS: i32 = 8192;

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
///
/// This is the one-shot entry point, for a webview whose band never changes while it is alive (the
/// whole-window webview the resize path installs). A dock view, whose band and radius follow the
/// frontend, keeps the shared state of [`crate::native_dock`] instead and re-applies the region
/// from it whenever that state changes.
pub fn give_frame_band_to_window<W: gtk::glib::IsA<gtk::Widget>>(widget: &W, band: Band) {
    apply_frame_band_and_corner(widget, band, 0.0);
    widget.connect_size_allocate(move |widget, _allocation| {
        apply_frame_band_and_corner(widget, band, 0.0)
    });
}

/// The input region of a webview: its allocation without the frame band it gives back, and
/// without the area a rounded bottom-right corner leaves unpainted.
///
/// The band a webview gives back and the corner it rounds are holes in the same input region, so
/// they are composed into one region here. A second region for the corner alone would reset the
/// band the previous allocation set, and a press on the window frame would reach the page again.
///
/// Nothing is connected here, so a caller that keeps the band and the radius in main-thread state
/// ([`crate::native_dock`]) can re-apply the region whenever that state changes, without adding a
/// second allocation callback.
pub(crate) fn apply_frame_band_and_corner<W: gtk::glib::IsA<gtk::Widget>>(
    widget: &W,
    band: Band,
    corner_radius: f64,
) {
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

    let (window_width, window_height) = (window.width(), window.height());
    let Some((x, y, width, height)) = inner_rect(window_width, window_height, band) else {
        return;
    };

    // The corner sits at the bottom right of the whole webview rather than of the banded
    // rectangle, so the shaped area is measured from the allocation and then clipped to the banded
    // rectangle.
    let shaped = rounded_rectangles(
        window_width,
        window_height,
        logical_corner_radius(corner_radius),
    );
    let beyond_band = clip_to_band(&shaped, (x, y, width, height));
    if beyond_band.is_empty() {
        return;
    }

    widget.input_shape_combine_region(Some(&Region::create_rectangles(&beyond_band)));
}

/// Corner radius in the logical window coordinates a shape region uses, from the logical radius
/// the frontend reported.
///
/// The region is in the same coordinates `gdk_window_get_width` reports, and GDK scales it to
/// device pixels itself, so the radius is used as it arrived: multiplying it by the scale factor
/// here would give a high density display a corner twice as large as the one it draws.
///
/// A radius that is not a usable length leaves the corner square, which keeps a mid-frame or
/// nonsense measurement from clipping the page.
fn logical_corner_radius(corner_radius: f64) -> i32 {
    if !(corner_radius > 0.0) {
        return 0;
    }

    corner_radius.round() as i32
}

/// Rectangles of the area a rounded bottom-right corner leaves painted, in logical pixels.
///
/// The rectangle is split where the corner arc starts: every row above the corner keeps its whole
/// width, and each row of the corner square keeps only the columns that lie inside the arc. The
/// other three corners have no radius, so they stay square.
pub(crate) fn rounded_rectangles(width: i32, height: i32, radius: i32) -> Vec<RectangleInt> {
    if width <= 0 || height <= 0 {
        return Vec::new();
    }

    // The radius cannot eat more than the rectangle holds, and a nonsense value is bounded so the
    // scanline below stays small: without this the squared term that positions the arc could
    // overflow and the allocation could grow without limit.
    let radius = radius.min(width).min(height).min(MAX_SCANLINE_RADIUS);
    if radius <= 0 {
        return vec![RectangleInt::new(0, 0, width, height)];
    }

    let corner_x = width - radius;
    let corner_y = height - radius;
    let radius = i64::from(radius);
    let mut rectangles = Vec::with_capacity(radius as usize + 1);
    if corner_y > 0 {
        rectangles.push(RectangleInt::new(0, 0, width, corner_y));
    }

    for row in 0..radius {
        // A pixel centre sits half a pixel in from the row edge, so every distance is doubled to
        // stay in whole numbers. The products saturate so the bounded radius above is the only
        // limit the arithmetic needs.
        let vertical = 2 * row + 1;
        let limit = radius
            .saturating_mul(radius)
            .saturating_mul(4)
            .saturating_sub(vertical.saturating_mul(vertical));
        let kept = kept_columns(limit);
        let row_width = corner_x + kept;
        if row_width > 0 {
            rectangles.push(RectangleInt::new(0, corner_y + row as i32, row_width, 1));
        }
    }

    rectangles
}

/// The part of `shaped` that lies inside a rectangle, as its own rectangles.
fn clip_to_band(shaped: &[RectangleInt], band: (i32, i32, i32, i32)) -> Vec<RectangleInt> {
    let (x, y, width, height) = band;
    let right = x + width;
    let bottom = y + height;
    let mut kept = Vec::with_capacity(shaped.len());

    for rectangle in shaped {
        let left = rectangle.x().max(x);
        let top = rectangle.y().max(y);
        let clipped_right = (rectangle.x() + rectangle.width()).min(right);
        let clipped_bottom = (rectangle.y() + rectangle.height()).min(bottom);
        if clipped_right > left && clipped_bottom > top {
            kept.push(RectangleInt::new(
                left,
                top,
                clipped_right - left,
                clipped_bottom - top,
            ));
        }
    }

    kept
}

/// Number of columns a corner row keeps inside the arc.
///
/// `limit` is four times the squared horizontal half-width the arc still covers in that row, so a
/// column is kept while its doubled, squared distance from the arc centre stays within it.
fn kept_columns(limit: i64) -> i32 {
    if limit <= 0 {
        return 0;
    }

    let mut odd = isqrt(limit) + 1;
    if odd % 2 == 0 {
        odd += 1;
    }
    ((odd - 1) / 2) as i32
}

/// Integer square root, so the scanline stays in whole numbers.
fn isqrt(value: i64) -> i64 {
    if value <= 0 {
        return 0;
    }

    let mut root = (value as f64).sqrt() as i64;
    while (root + 1) * (root + 1) <= value {
        root += 1;
    }
    while root * root > value {
        root -= 1;
    }
    root
}

#[cfg(test)]
mod tests {
    use gtk::cairo::RectangleInt;

    use super::{
        clip_to_band, inner_rect, logical_corner_radius, rounded_rectangles, Band, FRAME_BAND,
        MAX_SCANLINE_RADIUS,
    };

    /// The production half of this file, so a guard assertion can never match its own text.
    fn production_source() -> &'static str {
        include_str!("frame_edges.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("frame_edges.rs carries a test module")
    }

    /// The radius reaches a shape region in logical window coordinates, so it is used as it
    /// arrived: the region and the dimensions it is measured against are both logical, and GDK
    /// scales the region to device pixels itself. Multiplying by a scale factor here would double
    /// the corner on a high density display.
    #[test]
    fn the_corner_radius_is_kept_in_logical_window_coordinates() {
        assert_eq!(logical_corner_radius(15.0), 15);
        assert_eq!(logical_corner_radius(12.4), 12);
        // A radius that is not a usable length leaves the corner square.
        assert_eq!(logical_corner_radius(0.0), 0);
        assert_eq!(logical_corner_radius(-4.0), 0);
        assert_eq!(logical_corner_radius(f64::NAN), 0);

        // The production path applies no scale factor to the radius: GDK scales the whole region
        // once, so a second scale here would be wrong.
        let source = production_source();
        assert!(!source.contains(".scale_factor()"));
        assert!(!source.contains("f64::from("));
    }

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

    /// Whether a point is painted by any of the rectangles.
    fn covers(rectangles: &[RectangleInt], x: i32, y: i32) -> bool {
        rectangles.iter().any(|rectangle| {
            x >= rectangle.x()
                && x < rectangle.x() + rectangle.width()
                && y >= rectangle.y()
                && y < rectangle.y() + rectangle.height()
        })
    }

    /// The arc only ever gives back the bottom-right square, so the page keeps three square
    /// corners whatever the radius is.
    #[test]
    fn the_other_three_corners_stay_square() {
        let shaped = rounded_rectangles(40, 30, 10);

        assert!(covers(&shaped, 0, 0));
        assert!(covers(&shaped, 39, 0));
        assert!(covers(&shaped, 0, 29));
        // Only the outermost bottom-right pixels are given back.
        assert!(!covers(&shaped, 39, 29));
        assert!(covers(&shaped, 32, 29));
    }

    /// The rows above the corner keep the whole width, while the last corner row keeps only the
    /// columns inside the arc, which is what makes the edge follow the arc.
    #[test]
    fn the_painted_edge_follows_the_arc() {
        let shaped = rounded_rectangles(40, 30, 10);

        assert_eq!(
            shaped.first().copied(),
            Some(RectangleInt::new(0, 0, 40, 20))
        );
        assert_eq!(
            shaped.last().copied(),
            Some(RectangleInt::new(0, 29, 33, 1))
        );
    }

    /// A radius without a usable length leaves every corner square.
    #[test]
    fn a_corner_without_a_usable_radius_stays_square() {
        assert_eq!(
            rounded_rectangles(40, 30, 0),
            vec![RectangleInt::new(0, 0, 40, 30)]
        );
        assert_eq!(
            rounded_rectangles(40, 30, -4),
            vec![RectangleInt::new(0, 0, 40, 30)]
        );
        // A rectangle without an area has no corner to round.
        assert!(rounded_rectangles(0, 30, 8).is_empty());
        assert!(rounded_rectangles(40, 0, 8).is_empty());
    }

    /// A radius larger than the rectangle is clamped to the shorter side instead of rounding the
    /// rectangle away entirely.
    #[test]
    fn a_radius_larger_than_the_rectangle_is_clamped() {
        assert_eq!(
            rounded_rectangles(40, 30, 100),
            rounded_rectangles(40, 30, 30)
        );
    }

    /// A radius of one pixel fits inside the very corner pixel, while two pixels already give the
    /// single pixel beyond the arc back.
    #[test]
    fn a_small_radius_still_gives_back_the_outermost_corner_pixel() {
        assert!(covers(&rounded_rectangles(20, 20, 1), 19, 19));
        assert!(!covers(&rounded_rectangles(20, 20, 2), 19, 19));
        assert!(covers(&rounded_rectangles(20, 20, 2), 0, 19));
    }

    /// A nonsense radius is bounded before the scanline walks it, so a huge value cannot overflow
    /// the squared term or ask for an unbounded allocation.
    #[test]
    fn a_nonsense_radius_is_bounded_before_the_scanline_walks_it() {
        let bounded = rounded_rectangles(40, 30, MAX_SCANLINE_RADIUS);
        // The radius is clamped to the rectangle, so it still rounds the whole corner square.
        assert_eq!(bounded, rounded_rectangles(40, 30, 30));
        // A radius that is far beyond the bound is capped instead of walking it row by row: the
        // result is the bounded radius applied to the huge rectangle, not the unbounded one.
        let huge = rounded_rectangles(i32::MAX, i32::MAX, i32::MAX);
        assert_eq!(huge.len(), MAX_SCANLINE_RADIUS as usize + 1);
        assert_eq!(
            huge.first().copied(),
            Some(RectangleInt::new(
                0,
                0,
                i32::MAX,
                i32::MAX - MAX_SCANLINE_RADIUS
            ))
        );
    }

    /// The input region is the banded rectangle with the corner clipped out of it, so the band is
    /// composed with the corner instead of replacing it.
    #[test]
    fn the_input_region_composes_the_band_and_the_corner() {
        assert_eq!(
            clip_to_band(&[RectangleInt::new(0, 0, 100, 100)], (10, 10, 50, 50)),
            vec![RectangleInt::new(10, 10, 50, 50)]
        );
        assert!(clip_to_band(&[RectangleInt::new(0, 0, 5, 5)], (10, 10, 50, 50)).is_empty());

        // A band that gives the bottom edge back keeps it, even where the corner is rounded.
        let shaped = rounded_rectangles(600, 400, 15);
        let banded = clip_to_band(&shaped, (0, 5, 595, 390));
        assert!(covers(&banded, 0, 10));
        assert!(!covers(&banded, 10, 398));
        assert!(!covers(&banded, 599, 399));
    }
}
