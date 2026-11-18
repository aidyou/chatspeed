//! Client-side window frame for the workflow window.
//!
//! Adapted from `gpui_component::window_border`. The rounded surface, the
//! hairline outline and the resize hit bands stay; the inner box shadows go.
//! GPUI evaluates a shadow blur with four samples per fragment, so a 10px blur
//! reads as banded rings — and the four corner cut-outs are the only place where
//! such a shadow is visible, because the opaque surface covers it everywhere
//! else. That is what made the rounded corners look soft and stepped.

use gpui_kit::component::{window_corner_radii, ActiveTheme};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

const BORDER_SIZE: Pixels = px(1.);
/// Half-width of the resize band around each visible frame edge.
const RESIZE_HIT_SIZE: Pixels = px(4.);

/// Wraps `content` in the client-decorated frame, or passes it through when the
/// platform or the window manager owns the decorations.
///
/// The component library registers a root plugin that wraps every window in its
/// own `window_border()`. On Linux that border reserves a 20px shadow gutter,
/// clips everything to it, and rewrites the client inset every frame. Writing a
/// different inset here would flip `_GTK_FRAME_EXTENTS` between the two values
/// each frame, and the window manager answers that by nudging the toplevel —
/// the jitter this frame used to cause while dragging. So the inset stays with
/// the library, and this frame lives inside the gutter it reserves.
pub fn window_frame(
    content: impl IntoElement,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let content = content.into_any_element();
    let Decorations::Client { tiling } = window.window_decorations() else {
        return div()
            .id("window-backdrop")
            .size_full()
            .child(content)
            .into_any_element();
    };

    let radii = window_corner_radii(window, cx);
    // Lets the compositor treat the rounded corners as transparent.
    window.set_client_corner_radii(radii);
    let size = window.window_bounds().get_bounds().size;
    let border = cx.theme().window_border;

    let mut frame = div()
        .id("window-backdrop")
        .size_full()
        .overflow_hidden()
        .rounded_tl(radii.top_left)
        .rounded_tr(radii.top_right)
        .rounded_bl(radii.bottom_left)
        .rounded_br(radii.bottom_right)
        .border_color(border)
        .bg(cx.theme().background)
        // A press within the frame's edge band starts a native resize. Controls
        // that take the press stop propagation, so their click never resizes.
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            if !window.is_resizable() {
                return;
            }
            let Decorations::Client { tiling } = window.window_decorations() else {
                return;
            };
            if tiling.is_tiled() {
                return;
            }
            let size = window.window_bounds().get_bounds().size;
            let Some(edge) = resize_edge(window.mouse_position(), size, &tiling) else {
                return;
            };
            cx.stop_active_drag(window);
            cx.stop_propagation();
            window.start_window_resize(edge);
        })
        .when(!tiling.top, |frame| frame.border_t(BORDER_SIZE))
        .when(!tiling.bottom, |frame| frame.border_b(BORDER_SIZE))
        .when(!tiling.left, |frame| frame.border_l(BORDER_SIZE))
        .when(!tiling.right, |frame| frame.border_r(BORDER_SIZE))
        .child(content);

    if window.is_resizable() {
        frame = frame.child(
            div()
                .absolute()
                .inset_0()
                .children(resize_hit_zones(size, &tiling)),
        );
    }
    frame.into_any_element()
}

/// Corner radii for surfaces inside the frame.
///
/// The frame's border insets its content by one pixel, so a child that rounded
/// with the window radius would place its own arc outside the frame's arc and
/// paint into the transparent corner.
pub fn inner_corner_radii(window: &Window, cx: &App) -> Corners<Pixels> {
    inset_radii(window_corner_radii(window, cx))
}

fn inset_radii(radii: Corners<Pixels>) -> Corners<Pixels> {
    let inset = |radius: Pixels| (radius - BORDER_SIZE).max(px(0.));
    Corners {
        top_left: inset(radii.top_left),
        top_right: inset(radii.top_right),
        bottom_left: inset(radii.bottom_left),
        bottom_right: inset(radii.bottom_right),
    }
}

fn cursor_for_resize_edge(edge: ResizeEdge) -> CursorStyle {
    match edge {
        ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
        ResizeEdge::Left | ResizeEdge::Right => CursorStyle::ResizeLeftRight,
        ResizeEdge::TopLeft | ResizeEdge::BottomRight => CursorStyle::ResizeUpLeftDownRight,
        ResizeEdge::TopRight | ResizeEdge::BottomLeft => CursorStyle::ResizeUpRightDownLeft,
    }
}

/// Cursor-only overlay for each resize edge and corner; the resize itself starts
/// from the frame's `on_mouse_down`.
fn resize_hit_zones(size: Size<Pixels>, tiling: &Tiling) -> Vec<AnyElement> {
    if tiling.is_tiled() {
        return Vec::new();
    }

    let band = RESIZE_HIT_SIZE * 2.;
    let mut zones: Vec<AnyElement> = Vec::new();
    let mut push = |edge: ResizeEdge, origin: Point<Pixels>, zone: Size<Pixels>| {
        zones.push(
            div()
                .absolute()
                .left(origin.x)
                .top(origin.y)
                .w(zone.width)
                .h(zone.height)
                .cursor(cursor_for_resize_edge(edge))
                .into_any_element(),
        );
    };

    if !tiling.top {
        push(
            ResizeEdge::Top,
            point(px(0.), px(0.)),
            Size::new(size.width, band),
        );
    }
    if !tiling.bottom {
        push(
            ResizeEdge::Bottom,
            point(px(0.), size.height - band),
            Size::new(size.width, band),
        );
    }
    if !tiling.left {
        push(
            ResizeEdge::Left,
            point(px(0.), px(0.)),
            Size::new(band, size.height),
        );
    }
    if !tiling.right {
        push(
            ResizeEdge::Right,
            point(size.width - band, px(0.)),
            Size::new(band, size.height),
        );
    }

    // Corners are pushed after the edge strips so hit-testing prefers them.
    let corners = [
        (ResizeEdge::TopLeft, !tiling.top && !tiling.left, point(px(0.), px(0.))),
        (
            ResizeEdge::TopRight,
            !tiling.top && !tiling.right,
            point(size.width - band, px(0.)),
        ),
        (
            ResizeEdge::BottomLeft,
            !tiling.bottom && !tiling.left,
            point(px(0.), size.height - band),
        ),
        (
            ResizeEdge::BottomRight,
            !tiling.bottom && !tiling.right,
            point(size.width - band, size.height - band),
        ),
    ];
    for (edge, visible, origin) in corners {
        if visible {
            push(edge, origin, Size::new(band, band));
        }
    }

    zones
}

/// Hit-test the frame edges in window coordinates.
fn resize_edge(pos: Point<Pixels>, size: Size<Pixels>, tiling: &Tiling) -> Option<ResizeEdge> {
    let hit = RESIZE_HIT_SIZE;
    let on_left = pos.x <= hit;
    let on_right = pos.x >= size.width - hit;
    let on_top = pos.y <= hit;
    let on_bottom = pos.y >= size.height - hit;

    if !tiling.top && !tiling.left && on_top && on_left {
        return Some(ResizeEdge::TopLeft);
    }
    if !tiling.top && !tiling.right && on_top && on_right {
        return Some(ResizeEdge::TopRight);
    }
    if !tiling.bottom && !tiling.left && on_bottom && on_left {
        return Some(ResizeEdge::BottomLeft);
    }
    if !tiling.bottom && !tiling.right && on_bottom && on_right {
        return Some(ResizeEdge::BottomRight);
    }
    if !tiling.top && on_top {
        return Some(ResizeEdge::Top);
    }
    if !tiling.bottom && on_bottom {
        return Some(ResizeEdge::Bottom);
    }
    if !tiling.left && on_left {
        return Some(ResizeEdge::Left);
    }
    if !tiling.right && on_right {
        return Some(ResizeEdge::Right);
    }
    None
}

#[cfg(test)]
mod tests {
    // `gpui_kit::test` is in the macro namespace of the parent module, so the
    // built-in attribute is imported explicitly here.
    use super::resize_edge;
    use gpui_kit::{point, px, ResizeEdge, Size, Tiling};

    fn untiled() -> Tiling {
        Tiling::default()
    }

    #[::core::prelude::v1::test]
    fn edges_resize_only_inside_their_band() {
        let size = Size::new(px(100.), px(100.));
        assert_eq!(
            resize_edge(point(px(2.), px(50.)), size, &untiled()),
            Some(ResizeEdge::Left)
        );
        assert_eq!(
            resize_edge(point(px(50.), px(50.)), size, &untiled()),
            None
        );
        assert_eq!(
            resize_edge(point(px(99.), px(99.)), size, &untiled()),
            Some(ResizeEdge::BottomRight)
        );
    }

    #[::core::prelude::v1::test]
    fn tiled_edges_do_not_resize() {
        let size = Size::new(px(100.), px(100.));
        let tiling = Tiling {
            left: true,
            top: true,
            ..Tiling::default()
        };
        assert_eq!(resize_edge(point(px(1.), px(1.)), size, &tiling), None);
        assert_eq!(
            resize_edge(point(px(99.), px(99.)), size, &tiling),
            Some(ResizeEdge::BottomRight)
        );
    }

    #[::core::prelude::v1::test]
    fn inner_radii_stay_inside_the_frame() {
        use super::inset_radii;
        use gpui_kit::Corners;

        assert_eq!(inset_radii(Corners::all(px(16.))), Corners::all(px(15.)));
        assert_eq!(inset_radii(Corners::all(px(0.))), Corners::all(px(0.)));
    }
}
