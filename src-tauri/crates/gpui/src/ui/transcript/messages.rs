//! User bubbles and assistant responses.

use gpui_kit::component::{h_flex, v_flex, ActiveTheme};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::{plain_text, ui::Icon};
use chatspeed_gpui::presentation::{user_message_lines, UserView};

const USER_MESSAGE_LINES: usize = 4;
const USER_MESSAGE_FONT_SIZE: f32 = 14.;
const USER_MESSAGE_LINE_HEIGHT: f32 = 22.;
const USER_MESSAGE_COLLAPSED_TEXT_HEIGHT: f32 =
    USER_MESSAGE_LINES as f32 * USER_MESSAGE_LINE_HEIGHT;

pub(crate) fn user_bubble(
    view: &UserView,
    toggle_key: String,
    expanded: bool,
    on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let expandable = user_message_lines(&view.text) > USER_MESSAGE_LINES;
    let collapsed = expandable && !expanded;
    let surface = cx.theme().muted;
    let text = div()
        .w_full()
        .min_w_0()
        .text_size(px(USER_MESSAGE_FONT_SIZE))
        .line_height(px(USER_MESSAGE_LINE_HEIGHT))
        .text_color(cx.theme().foreground)
        .when(collapsed, |text| {
            text.max_h(px(USER_MESSAGE_COLLAPSED_TEXT_HEIGHT))
                .overflow_hidden()
        })
        .child(view.text.clone());
    let toggle = h_flex()
        .id(SharedString::from(toggle_key))
        .absolute()
        .right_1()
        .bottom_1()
        .w(px(20.))
        .h(px(20.))
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .bg(cx.theme().muted)
        .cursor_pointer()
        .child(
            Icon::new(if expanded {
                "double-arrow-up"
            } else {
                "double-arrow-down"
            })
            .size(12.)
            .color(cx.theme().muted_foreground),
        )
        .on_click(on_toggle);
    h_flex()
        .w_full()
        .min_w_0()
        .justify_end()
        .child(
            div()
                .relative()
                .max_w(px(680.))
                .min_w_0()
                .rounded(px(16.))
                .border_1()
                .border_color(cx.theme().border)
                .bg(surface)
                .px_4()
                .py_3()
                // Keep the toggle in a side gutter instead of adding a blank row below.
                .when(expandable, |bubble| bubble.pr(px(28.)))
                .child(div().relative().child(text).when(collapsed, |body| {
                    body.child(
                        div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .bottom_0()
                            .h(px(12.))
                            .bg(linear_gradient(
                                180.,
                                linear_color_stop(surface.opacity(0.), 0.),
                                linear_color_stop(surface, 1.),
                            )),
                    )
                }))
                .when_some(view.delivery_label.clone(), |bubble, label| {
                    bubble.child(div().mt_1().text_size(px(11.)).text_color(cx.theme().muted_foreground).child(label))
                })
                .when(expandable, |bubble| bubble.child(toggle)),
        )
        .into_any_element()
}

pub(crate) fn thought(
    key: String,
    title: String,
    text: &str,
    open: bool,
    live: bool,
    on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    // The press only covers the icon and the label: Vue leaves the rest of the
    // line inert instead of turning the whole row into a button.
    let header = h_flex()
        .id(SharedString::from(key))
        .group("thought-header")
        .flex_none()
        .max_w_full()
        .min_w_0()
        .items_center()
        .gap_2()
        .h(px(24.))
        .px(px(2.))
        .rounded(px(4.))
        .cursor_pointer()
        .child({
            let bulb = Icon::new("reasoning").size(14.).color(muted);
            if live && !open {
                bulb.spin("thought-spin", crate::ui::icon::Spin::Linear)
            } else {
                bulb
            }
        })
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(13.))
                .line_height(px(18.))
                .text_color(muted)
                .italic()
                .child(crate::ui::chrome_label(title.clone())),
        )
        .child(crate::ui::disclosure_caret("thought-header", open, muted))
        .tooltip(crate::ui::tip(title))
        .on_click(on_toggle);
    v_flex()
        .w_full()
        .min_w_0()
        .gap_2()
        .child(h_flex().w_full().min_w_0().items_center().child(header))
        .when(open, |view| {
            view.child(
                div()
                    .w_full()
                    .min_w_0()
                    .rounded(px(4.))
                    .border_l_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().muted)
                    .px_3()
                    .py_2()
                    .text_size(px(13.))
                    .italic()
                    .text_color(cx.theme().muted_foreground)
                    .child(plain_text(text, cx.theme().muted_foreground)),
            )
        })
        .into_any_element()
}

pub(crate) fn assistant_message(
    thought: Option<AnyElement>,
    markdown: Option<AnyElement>,
) -> AnyElement {
    v_flex()
        .w_full()
        .min_w_0()
        .gap_2()
        .children(thought)
        .children(markdown)
        .into_any_element()
}
