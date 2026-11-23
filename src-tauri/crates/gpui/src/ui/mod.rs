//! Native workflow window rendering and reusable controls.

use gpui_kit::component::ActiveTheme;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

pub(crate) mod history_scrollbar;
mod icon;
mod menu_popover;
pub(crate) mod tabs;
pub(crate) mod terminal;

pub use icon::Icon;
pub(crate) use icon::{IconAssets, Spin};

pub(crate) mod tokens;
pub(crate) use tokens::{field_bg, surfaces, Surfaces};

/// Hover hints appear faster than the library's 500ms default.
pub(crate) const TOOLTIP_DELAY: std::time::Duration = std::time::Duration::from_millis(200);

/// Rotates an icon between two angles when a disclosure state flips.
///
/// The animation id includes the target angle, so each toggle starts a fresh
/// one-shot animation from the previous angle instead of jumping.
pub(crate) fn rotating_icon(
    id: impl Into<SharedString>,
    icon: &'static str,
    size: f32,
    color: Hsla,
    from: f32,
    to: f32,
) -> AnyElement {
    // A spring retargets from its current angle, so repeated toggles stay smooth
    // and never restart from a stale one-shot phase.
    Icon::new(icon)
        .size(size)
        .color(color)
        .with_spring(
            id.into(),
            SpringAnimation::new(SpringConfig::new(420., 32., 1.))
                .to(to)
                .from(from),
            move |icon, angle| icon.rotate(angle),
        )
        .into_any_element()
}

/// A compact label next to an icon. Keep its optical adjustment out of the row layout.
#[derive(IntoElement)]
pub(crate) struct ChromeLabel(SharedString);

pub(crate) fn chrome_label(text: impl Into<SharedString>) -> ChromeLabel {
    ChromeLabel(text.into())
}

/// A plain-text hover hint, the native counterpart of Vue's `title`/`el-tooltip`.
///
/// Element Plus renders tooltips at 12px on a dark surface with a tight 4px
/// radius. The component library's default is one step larger than the rest of
/// the titlebar, so the shared helper pins the compact size for every hint.
pub(crate) fn tip(
    text: impl Into<SharedString>,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text = text.into();
    move |window, cx| {
        gpui_kit::component::tooltip::Tooltip::new(text.clone())
            .text_size(px(12.))
            .line_height(px(18.))
            .px(px(8.))
            .py(px(4.))
            .rounded(px(4.))
            .border_color(rgb(0x303133))
            .bg(rgb(0x303133))
            .text_color(gpui_kit::white())
            .build(window, cx)
    }
}

/// Vue's sidebar tooltip: the conversation title with the primary authorized
/// folder underneath, so a collapsed badge still identifies both.
pub(crate) fn tip_session(
    title: String,
    root: Option<String>,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let title = SharedString::from(title);
    let root = root.map(SharedString::from);
    move |window, cx| {
        let title = title.clone();
        let root = root.clone();
        gpui_kit::component::tooltip::Tooltip::element(move |_, cx| {
            let mut content = v_flex().gap(px(2.)).child(
                div()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title.clone()),
            );
            if let Some(root) = root.clone() {
                content = content.child(
                    h_flex()
                        .items_center()
                        .gap(px(4.))
                        .child(
                            Icon::new("ext-folder")
                                .size(11.)
                                .color(cx.theme().muted_foreground),
                        )
                        .child(div().text_size(px(11.)).child(root)),
                );
            }
            content
        })
        .text_size(px(12.))
        .line_height(px(18.))
        .px(px(8.))
        .py(px(4.))
        .rounded(px(4.))
        .border_color(rgb(0x303133))
        .bg(rgb(0x303133))
        .text_color(gpui_kit::white())
        .build(window, cx)
    }
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
                .spin(id, icon::Spin::Accelerate),
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
        .child(rotating_icon(
            format!("disclosure-{group}"),
            "caret-right",
            14.,
            color,
            if open { 0. } else { 90. },
            if open { 90. } else { 0. },
        ))
}

/// Height of the custom titlebar, matching `--cs-titlebar-height`.
pub const TITLEBAR_HEIGHT: f32 = 40.;

/// Width of the expanded sidebar (`--sidebar-width`) and of the icon rail it
/// collapses into.
pub const SIDEBAR_WIDTH: f32 = 300.;
pub const SIDEBAR_COLLAPSED_WIDTH: f32 = 50.;
/// Vue pins the navigation rail to the same 50px column.
pub const RAIL_WIDTH: f32 = 50.;

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
    hint: Option<String>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    icon_button_element(
        id,
        Icon::new(icon).size(18.).color(color),
        hover,
        hint,
        on_click,
    )
}

pub(super) fn icon_button_element(
    id: &'static str,
    icon: Icon,
    hover: Hsla,
    hint: Option<String>,
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
        .when_some(hint, |button, text| {
            button.tooltip(tip(text)).tooltip_show_delay(TOOLTIP_DELAY)
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
pub(crate) mod confirm;
pub(crate) mod file_editor;
pub mod file_tree;
pub mod frame;
#[cfg(feature = "chat-hub")]
pub(crate) mod letter_avatar {
    use gpui_kit::{prelude::*, *};

    fn initial(name: &str) -> String {
        name.trim()
            .chars()
            .next()
            .map_or_else(|| "—".into(), |c| c.to_uppercase().collect())
    }

    fn badge(letter: String, size: f32) -> AnyElement {
        div()
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .size(px(size))
            .rounded_full()
            .bg(rgb(0x8a8f98))
            .text_size(px(size * 0.55))
            .line_height(px(size * 0.65))
            .text_color(white())
            .child(letter)
            .into_any_element()
    }

    pub(crate) fn with_logo(id: &'static str, name: &str, logo: &str, size: f32) -> AnyElement {
        let letter = initial(name);
        let logo = logo.trim();
        if !logo.starts_with("https://") && !logo.starts_with("http://") {
            return badge(letter, size);
        }
        let loading = letter.clone();
        img(logo)
            .id(id)
            .flex_none()
            .size(px(size))
            .rounded_full()
            .object_fit(ObjectFit::Contain)
            .with_loading(move || badge(loading.clone(), size))
            .with_fallback(move || badge(letter.clone(), size))
            .into_any_element()
    }

    #[cfg(test)]
    #[::core::prelude::v1::test]
    fn initial_handles_whitespace_and_unicode() {
        assert_eq!(initial("  chat"), "C");
        assert_eq!(initial("中文"), "中");
        assert_eq!(initial("  "), "—");
    }
}

pub mod sidebar;
pub mod status;
pub mod titlebar;
pub(crate) mod toast;
pub mod transcript;
pub(crate) mod window_pin;
