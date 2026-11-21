//! Native workflow window rendering and reusable controls.

use gpui_kit::*;

pub(crate) mod history_scrollbar;
mod icon;
mod menu_popover;
pub(crate) mod terminal;

pub use icon::Icon;
pub(crate) use icon::{IconAssets, Spin};

/// A compact label next to an icon. Keep its optical adjustment out of the row layout.
#[derive(IntoElement)]
pub(crate) struct ChromeLabel(SharedString);

pub(crate) fn chrome_label(text: impl Into<SharedString>) -> ChromeLabel {
    ChromeLabel(text.into())
}

impl RenderOnce for ChromeLabel {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let style = window.text_style();
        let size = style.font_size.to_pixels(window.rem_size());
        let fonts = cx.text_system();
        let font = fonts.resolve_font(&style.font());
        let offset = if self.0.contains('\n') {
            px(0.)
        } else {
            let ink_height = (fonts.cap_height(font, size) + fonts.x_height(font, size)) / 2.;
            ((ink_height + fonts.descent(font, size) - fonts.ascent(font, size)) / 2.)
                .max(px(-1.5))
                .min(px(1.5))
        };
        div().relative().top(offset).min_w_0().child(self.0)
    }
}

pub(crate) fn running_badge(label: String, id: impl Into<ElementId>, cx: &App) -> impl IntoElement {
    use gpui_kit::component::ActiveTheme;
    let color = cx.theme().primary;
    gpui_kit::component::h_flex()
        .flex_none()
        .gap_1()
        .px_2()
        .py(px(2.))
        .rounded(px(4.))
        .bg(color.opacity(0.12))
        .text_color(color)
        .text_size(px(11.))
        .child(
            Icon::new("loading")
                .size(12.)
                .color(color)
                .spin(id, icon::Spin::Eased),
        )
        .child(chrome_label(label))
}

/// Keep the caret's space reserved so hovering never moves the label.
pub(super) fn disclosure_caret(group: &'static str, open: bool, color: Hsla) -> impl IntoElement {
    div()
        .id("disclosure-caret")
        .flex_none()
        .invisible()
        .group_hover(group, |style| style.visible())
        .child(
            Icon::new(if open { "caret-down" } else { "caret-right" })
                .size(14.)
                .color(color),
        )
}

/// Height of the custom titlebar, matching `--cs-titlebar-height`.
pub const TITLEBAR_HEIGHT: f32 = 40.;

/// Width of the expanded sidebar (`--sidebar-width`) and of the icon rail it
/// collapses into.
pub const SIDEBAR_WIDTH: f32 = 300.;
pub const SIDEBAR_COLLAPSED_WIDTH: f32 = 50.;
/// Vue pins the navigation rail to the same 50px column.
pub const RAIL_WIDTH: f32 = 50.;

/// Vue surfaces the theme does not carry as named tokens.
///
/// The values are the light and dark entries of `--cs-bg-color-deep`,
/// `--cs-titlebar-bg-color`, `--cs-titlebar-border-color`,
/// `--cs-hover-bg-color` and `--cs-active-bg-color`, so the chrome matches the
/// web window instead of the component library's approximations.
pub struct Surfaces {
    /// Rail and compact sidebar background.
    pub deep: Hsla,
    /// Titlebar strip, one step lighter than the body it sits on.
    pub titlebar: Hsla,
    pub titlebar_border: Hsla,
    pub hover: Hsla,
    /// Selected row, tab and rail item background.
    pub active: Hsla,
}

pub fn surfaces(dark: bool) -> Surfaces {
    if dark {
        Surfaces {
            deep: rgb(0x232629).into(),
            titlebar: rgb(0x21221e).into(),
            titlebar_border: rgb(0x111111).into(),
            hover: rgb(0x2e2f2c).into(),
            active: rgb(0x2f3234).into(),
        }
    } else {
        Surfaces {
            deep: rgb(0xe7e7e6).into(),
            titlebar: rgb(0xfcfcfc).into(),
            titlebar_border: rgb(0xe2e2e0).into(),
            hover: rgb(0xefefef).into(),
            active: rgb(0xd2d3d1).into(),
        }
    }
}

/// A flat icon button used by the titlebar and the composer footer.
///
/// Titlebar buttons must be focusable children of the bar rather than window
/// controls, so the drag area keeps the rest of the bar. The 18px glyph in a
/// 26x22 box is Vue's `.icon-btn` at `--cs-font-size-lg`.
pub fn icon_button(
    id: &'static str,
    icon: &'static str,
    color: Hsla,
    hover: Hsla,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    icon_button_element(id, Icon::new(icon).size(18.).color(color), hover, on_click)
}

pub(super) fn icon_button_element(
    id: &'static str,
    icon: Icon,
    hover: Hsla,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .w(px(26.))
        .h(px(22.))
        .rounded(px(8.))
        .cursor_pointer()
        .hover(|style| style.bg(hover))
        .child(icon)
        // A control that takes the press must not also start the window move.
        .on_mouse_down(MouseButton::Left, |_, window, cx| {
            window.prevent_default();
            cx.stop_propagation();
        })
        .on_click(on_click)
        .into_any_element()
}

/// Wraps a control so `Popover` can own its open state.
///
/// `Popover::trigger` needs `Selectable`, which only the component library's own
/// controls implement; the chrome uses flat hand-drawn buttons instead.
pub struct Trigger {
    element: AnyElement,
    open: bool,
}

impl Trigger {
    pub fn new(element: impl IntoElement) -> Self {
        Self {
            element: element.into_any_element(),
            open: false,
        }
    }
}

impl gpui_kit::component::Selectable for Trigger {
    fn selected(mut self, selected: bool) -> Self {
        self.open = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.open
    }
}

impl IntoElement for Trigger {
    type Element = AnyElement;

    fn into_element(self) -> Self::Element {
        self.element
    }
}

/// A flat popover trigger: the inner element stays a plain, unstyled surface so
/// `Popover` receives the press and the titlebar drag area keeps the rest.
pub fn trigger(element: impl IntoElement) -> Trigger {
    Trigger::new(element)
}

pub mod approval;
pub mod composer;
pub(crate) mod file_editor;
pub mod file_tree;
pub mod frame;
pub mod sidebar;
pub mod status;
pub mod titlebar;
pub(crate) mod toast;
pub mod transcript;
pub(crate) mod window_pin;
