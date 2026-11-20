//! User bubbles and assistant responses.

use gpui_kit::component::{h_flex, v_flex, ActiveTheme};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::{plain_text, ui::Icon};
use chatspeed_gpui::presentation::{user_message_lines, UserView};

const USER_MESSAGE_LINES: usize = 4;
// Four 1.6-line-height rows at 14px, plus room for the corner toggle.
const USER_MESSAGE_COLLAPSED_HEIGHT: f32 = 114.;

pub(crate) fn user_bubble(
    view: &UserView,
    toggle_key: String,
    expanded: bool,
    on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let expandable = user_message_lines(&view.text) > USER_MESSAGE_LINES;
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
                .bg(cx.theme().muted)
                .px_4()
                .py_3()
                .text_size(px(14.))
                .when(expandable && !expanded, |bubble| {
                    // Reserve the corner so the toggle never covers the text.
                    bubble
                        .max_h(px(USER_MESSAGE_COLLAPSED_HEIGHT))
                        .overflow_hidden()
                        .pr(px(28.))
                        .pb(px(28.))
                })
                .when(expandable, |bubble| bubble.child(toggle))
                .child(plain_text(&view.text, cx.theme().foreground)),
        )
        .into_any_element()
}

pub(crate) fn thought(
    key: String,
    title: String,
    text: &str,
    open: bool,
    on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    v_flex()
        .w_full()
        .min_w_0()
        .gap_2()
        .child(
            h_flex()
                .id(SharedString::from(key))
                .w_full()
                .min_w_0()
                .items_center()
                .gap_2()
                .h(px(24.))
                .cursor_pointer()
                .hover(|style| style.bg(cx.theme().muted))
                .child(Icon::new("reasoning").size(14.).color(muted))
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(13.))
                        .line_height(px(24.))
                        .text_color(muted)
                        .italic()
                        .child(title),
                )
                .child(
                    Icon::new(if open { "caret-down" } else { "caret-right" })
                        .size(12.)
                        .color(muted),
                )
                .on_click(on_toggle),
        )
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
