//! Element Plus style confirmation dialog.
//!
//! The platform `window.prompt` is a bare native message box that ignores the
//! application theme. These confirmations are drawn inside the workflow window
//! so destructive actions (rewind, stop, closing a terminal) share the same
//! surface, spacing and button hierarchy as the rest of the native UI.

use gpui_kit::component::{h_flex, v_flex, ActiveTheme};
use gpui_kit::*;
use rust_i18n::t;

use super::icon::Icon;

#[derive(Clone, Debug)]
pub struct ConfirmRequest<K> {
    pub kind: K,
    pub title: String,
    pub message: String,
    pub confirm_label: String,
    pub danger: bool,
}

pub fn dialog<K>(
    request: &ConfirmRequest<K>,
    on_accept: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_cancel: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let border = cx.theme().border;
    let background = cx.theme().background;
    let foreground = cx.theme().foreground;
    let muted = cx.theme().muted_foreground;
    let primary = cx.theme().primary;
    let accent: Hsla = if request.danger {
        cx.theme().danger
    } else {
        primary
    };
    let accent_foreground: Hsla = if request.danger {
        gpui_kit::white()
    } else {
        cx.theme().primary_foreground
    };

    div()
        .id("confirm-dialog-backdrop")
        .absolute()
        .inset_0()
        .occlude()
        .flex()
        .items_center()
        .justify_center()
        .bg(black().opacity(0.45))
        .child(
            v_flex()
                .id("confirm-dialog")
                .w(px(420.))
                .max_w(relative(0.9))
                .rounded(super::tokens::RADIUS_MEDIUM)
                .border_1()
                .border_color(border)
                .bg(background)
                .overflow_hidden()
                .shadow(vec![BoxShadow {
                    offset: point(px(0.), px(12.)),
                    blur_radius: px(32.),
                    spread_radius: px(0.),
                    inset: false,
                    color: foreground.opacity(0.18),
                }])
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .gap_2()
                        .px(px(20.))
                        .pt(px(16.))
                        .pb(px(8.))
                        .child(Icon::new("warning").size(18.).color(accent))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(16.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(foreground)
                                .child(request.title.clone()),
                        ),
                )
                .child(
                    div()
                        .w_full()
                        .px(px(20.))
                        .py(px(12.))
                        .text_size(px(14.))
                        .line_height(px(22.))
                        .text_color(muted)
                        .child(request.message.clone()),
                )
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .justify_end()
                        .gap(px(8.))
                        .px(px(20.))
                        .pt(px(8.))
                        .pb(px(16.))
                        .child(
                            h_flex()
                                .id("confirm-dialog-cancel")
                                .flex_none()
                                .items_center()
                                .justify_center()
                                .h(px(30.))
                                .px(px(16.))
                                .rounded(super::tokens::RADIUS_MEDIUM)
                                .border_1()
                                .border_color(border)
                                .bg(background)
                                .cursor_pointer()
                                .hover(|style| style.bg(cx.theme().muted))
                                .text_size(px(13.))
                                .text_color(foreground)
                                .child(t!("action_cancel").to_string())
                                .on_click(on_cancel),
                        )
                        .child(
                            h_flex()
                                .id("confirm-dialog-accept")
                                .flex_none()
                                .items_center()
                                .justify_center()
                                .h(px(30.))
                                .px(px(16.))
                                .rounded(super::tokens::RADIUS_MEDIUM)
                                .bg(accent)
                                .cursor_pointer()
                                .hover(|style| style.opacity(0.9))
                                .text_size(px(13.))
                                .text_color(accent_foreground)
                                .child(request.confirm_label.clone())
                                .on_click(on_accept),
                        ),
                ),
        )
        .into_any_element()
}
