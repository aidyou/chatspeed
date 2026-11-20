//! Composer: the prompt box and the chrome around it.
//!
//! Mirrors `src/components/workflow/WorkflowInputArea.vue`: the runtime state
//! sits above a multi-line box whose height follows its text and can be
//! expanded, with the actions under it in [`footer`]. Enter sends; Shift+Enter
//! keeps a newline.
//!
//! Each visual group has its own function so adding Vue's model, agent or tool
//! selectors here never turns the layout into one long builder chain.

use gpui_kit::component::input::Textarea;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::ui::Icon;
use crate::{danger, success, Observer};

mod footer;

/// Vue's expanded box tops out at 540px; this window keeps its actions visible.
const EXPANDED_HEIGHT: f32 = 420.;

/// Renders the notice line, the state line, the prompt box and its footer.
pub fn composer(observer: &Observer, cx: &mut Context<Observer>) -> AnyElement {
    let mut column = v_flex()
        .flex_none()
        .w_full()
        .items_center()
        .px_4()
        .pb_3()
        .pt_2()
        .gap_2();

    if let Some(notice) = &observer.notice {
        column = column.child(notice_line(notice, cx));
    }
    column = column.child(status_line(observer, cx));
    column = column.child(box_body(observer, cx));
    column.into_any_element()
}

/// The last action failure, shown above the box until the next send.
fn notice_line(notice: &str, _cx: &mut Context<Observer>) -> AnyElement {
    h_flex()
        .w_full()
        .max_w(px(780.))
        .items_center()
        .gap_2()
        .px_3()
        .py_2()
        .rounded(px(8.))
        .bg(danger().opacity(0.12))
        .text_size(px(11.))
        .text_color(danger())
        .child(Icon::new("approval").size(12.).color(danger()))
        .child(div().flex_1().min_w_0().child(notice.to_owned()))
        .into_any_element()
}

/// Vue's `StatusNotifier`: the runtime state between transcript and box.
fn status_line(observer: &Observer, cx: &mut Context<Observer>) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let accent = cx.theme().primary;
    let running = observer.workflow_running();
    h_flex()
        .w_full()
        .max_w(px(780.))
        .items_center()
        .gap_2()
        .px_1()
        .text_size(px(11.))
        .text_color(muted)
        .child(
            div()
                .flex_none()
                .w(px(6.))
                .h(px(6.))
                .rounded_full()
                .bg(if running {
                    accent
                } else {
                    Hsla::from(success())
                }),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_ellipsis()
                .child(observer.status_line()),
        )
        .into_any_element()
}

/// The framed box: the textarea, its expand toggle and the footer.
fn box_body(observer: &Observer, cx: &mut Context<Observer>) -> AnyElement {
    let border = cx.theme().border;
    let hover = cx.theme().muted;
    let muted = cx.theme().muted_foreground;
    let expanded = observer.composer_expanded;

    let textarea = Textarea::new(&observer.composer)
        .appearance(false)
        .bordered(false)
        .w_full();
    // A capped box keeps its own scrollbar, so typing past the visible rows
    // scrolls instead of painting over the frame.
    let textarea = if expanded {
        textarea.h_full()
    } else {
        textarea
    };

    v_flex()
        .w_full()
        .max_w(px(780.))
        .max_h(px(EXPANDED_HEIGHT))
        .rounded(px(12.))
        .border_1()
        .border_color(border)
        .bg(cx.theme().background)
        .when(expanded, |view| view.h(px(EXPANDED_HEIGHT)))
        .child(
            div()
                .relative()
                .w_full()
                .min_h_0()
                .when(expanded, |view| view.flex_1())
                .child(textarea)
                // Vue keeps the expand toggle at the box's top-right.
                .child(
                    div()
                        .id("composer-expand")
                        .absolute()
                        .top(px(4.))
                        .right(px(6.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(20.))
                        .rounded(px(4.))
                        .cursor_pointer()
                        .hover(|style| style.bg(hover))
                        .child(
                            Icon::new(if expanded {
                                "double-arrow-down"
                            } else {
                                "double-arrow-up"
                            })
                            .size(12.)
                            .color(muted),
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_composer_expanded(cx))),
                ),
        )
        .child(footer::footer(observer, cx))
        .into_any_element()
}
