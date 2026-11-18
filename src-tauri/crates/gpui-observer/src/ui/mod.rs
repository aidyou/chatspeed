//! Window chrome for the native workflow window.
//!
//! The transcript stays in the crate root; these modules own the titlebar,
//! the compact session sidebar, the composer, and the approval card so the
//! native window can mirror the Vue shell without growing one file further.

use gpui_kit::*;

/// Height of the custom titlebar, matching `--cs-titlebar-height`.
pub const TITLEBAR_HEIGHT: f32 = 40.;

/// Width of the expanded sidebar (`--sidebar-width`) and of the icon rail it
/// collapses into.
pub const SIDEBAR_WIDTH: f32 = 300.;
pub const SIDEBAR_COLLAPSED_WIDTH: f32 = 50.;

/// A flat icon button used by the titlebar and the composer footer.
///
/// Titlebar buttons must be focusable children of the bar rather than window
/// controls, so the drag area keeps the rest of the bar.
pub fn icon_button(
    id: &'static str,
    icon: &'static str,
    color: Hsla,
    hover: Hsla,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .size(px(26.))
        .rounded(px(6.))
        .cursor_pointer()
        .hover(|style| style.bg(hover))
        .child(crate::font_icon(icon, 16., color))
        // A control that takes the press must not also start the window move.
        .on_mouse_down(MouseButton::Left, |_, window, cx| {
            window.prevent_default();
            cx.stop_propagation();
        })
        .on_click(on_click)
        .into_any_element()
}

pub mod approval;
pub mod composer;
pub mod frame;
pub mod sidebar;
pub mod titlebar;
