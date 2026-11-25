//! Native workflow window rendering and reusable controls.

use gpui_kit::component::ActiveTheme;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

pub(crate) mod history_scrollbar;
pub(crate) use chatspeed_gpui::components::{select as menu_popover, tabs, tokens};
pub(crate) mod terminal;

pub use chatspeed_gpui::components::icon::Icon;
pub(crate) use chatspeed_gpui::components::icon::{self, IconAssets, Spin};
pub(crate) use chatspeed_gpui::components::tooltip::{tip, TOOLTIP_DELAY};

pub(crate) use tokens::{field_bg, layers, surfaces, Surfaces};

/// Samples disclosure rotation from a monotonic clock and explicitly wakes
/// GPUI Fast's cached transcript layers while the angle changes.
#[derive(IntoElement)]
struct DisclosureRotation {
    id: SharedString,
    icon: Icon,
    target: f32,
}

struct DisclosureMotion {
    from: f32,
    target: f32,
    started: std::time::Instant,
}

impl DisclosureMotion {
    fn angle(&self, now: std::time::Instant) -> (f32, bool) {
        let phase = (now.duration_since(self.started).as_secs_f32() / 0.18).min(1.);
        let moving = self.from != self.target && phase < 1.;
        (self.from + (self.target - self.from) * ease_in_out(phase), moving)
    }
}

#[cfg(test)]
#[::core::prelude::v1::test]
fn disclosure_rotation_reaches_the_target_and_stops_requesting_frames() {
    use std::time::{Duration, Instant};

    let started = Instant::now();
    let motion = DisclosureMotion { from: 0., target: 180., started };
    assert_eq!(motion.angle(started), (0., true));
    let (midpoint, moving) = motion.angle(started + Duration::from_millis(90));
    assert!(moving && midpoint > 0. && midpoint < 180.);
    assert_eq!(motion.angle(started + Duration::from_millis(200)), (180., false));
}

impl RenderOnce for DisclosureRotation {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let now = std::time::Instant::now();
        let state = window.use_keyed_state(self.id, cx, |_, _| {
            std::cell::RefCell::new(DisclosureMotion {
                from: self.target,
                target: self.target,
                started: now,
            })
        });
        let mut motion = state.read(cx).borrow_mut();
        if motion.target != self.target {
            *motion = DisclosureMotion {
                from: motion.angle(now).0,
                target: self.target,
                started: now,
            };
        }
        let (angle, moving) = motion.angle(now);
        if moving {
            window.request_animation_frame();
        }
        self.icon.rotate(angle)
    }
}

pub(crate) fn rotating_icon(
    id: impl Into<SharedString>,
    icon: &'static str,
    size: f32,
    color: Hsla,
    to: f32,
) -> AnyElement {
    DisclosureRotation {
        id: id.into(),
        icon: Icon::new(icon).size(size).color(color),
        target: to,
    }
    .into_any_element()
}

/// A compact label next to an icon. Keep its optical adjustment out of the row layout.
#[derive(IntoElement)]
pub(crate) struct ChromeLabel(SharedString);

pub(crate) fn chrome_label(text: impl Into<SharedString>) -> ChromeLabel {
    ChromeLabel(text.into())
}

/// Vue's sidebar tooltip: the conversation title with the status and primary
/// authorized folder underneath, so a collapsed badge still identifies both.
pub(crate) fn tip_session(
    title: String,
    root: Option<String>,
    status: Option<(String, Hsla)>,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let title = SharedString::from(title);
    let root = root.map(SharedString::from);
    let status = status.map(|(label, color)| (SharedString::from(label), color));
    move |window, cx| {
        let title = title.clone();
        let root = root.clone();
        let status = status.clone();
        let tooltip = chatspeed_gpui::components::tooltip::Tooltip::element(move |_, cx| {
            // On the tooltip's own surface, not the theme's: the panel is
            // inverted against the page, so `muted_foreground` would vanish.
            let muted = chatspeed_gpui::components::tooltip::dim(cx.theme().is_dark());
            let mut content = v_flex()
                .max_w(px(320.))
                .min_w_0()
                .gap(px(4.))
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title.clone()),
                );
            // Vue's `__meta` row: the status dot and label on the left, the
            // primary authorized folder on the right, both in the secondary
            // color at `--cs-font-size-xs`.
            if status.is_some() || root.is_some() {
                let mut meta = h_flex()
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .text_size(px(10.))
                    .text_color(muted);
                if let Some((label, color)) = status.clone() {
                    meta = meta.child(
                        h_flex()
                            .flex_none()
                            .items_center()
                            .gap(px(4.))
                            .child(div().size(px(6.)).rounded_full().bg(color))
                            .child(label),
                    );
                }
                if let Some(root) = root.clone() {
                    meta = meta.child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .justify_end()
                            .items_center()
                            .gap(px(4.))
                            .child(Icon::new("ext-folder").size(12.).color(muted))
                            .child(
                                div()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(root),
                            ),
                    );
                }
                content = content.child(meta);
            }
            content.into_any_element()
        });
        chatspeed_gpui::components::tooltip::compact(tooltip, cx).build(window, cx)
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

pub use chatspeed_gpui::components::trigger;

pub mod approval;
pub mod composer;
pub(crate) mod confirm {
    pub use chatspeed_gpui::components::confirm::dialog;
    pub type ConfirmRequest = chatspeed_gpui::components::confirm::ConfirmRequest<ConfirmKind>;

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum ConfirmKind {
        Rewind,
        StopWorkflow,
        CloseTerminal(usize),
        DeleteSession(String),
        DeleteAutomation(String),
        ReplaceShellRules { session: String, import: bool },
    }
}
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
pub mod transcript;
pub(crate) mod window_pin;
