//! Composer: the prompt box and its footer actions.
//!
//! Mirrors the footer of `src/components/workflow/WorkflowInputArea.vue`:
//! a multi-line box, the runtime state on the left, and the stop/send actions
//! on the right. Enter sends; Shift+Enter keeps a newline.

use gpui_kit::component::input::Textarea;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::ui::Icon;
use crate::{danger, Observer};

/// Renders the notice line and the prompt box under the transcript.
pub fn composer(observer: &Observer, cx: &mut Context<Observer>) -> AnyElement {
    let border = cx.theme().border;
    let surface = cx.theme().background;
    let muted = cx.theme().muted_foreground;
    let accent = cx.theme().primary;
    let hover = cx.theme().muted;
    let empty = observer.composer.read(cx).value().trim().is_empty();
    let can_send = !empty && !observer.start_pending;
    let running = observer.workflow_running();

    let mut column = v_flex()
        .flex_none()
        .w_full()
        .items_center()
        .px_4()
        .pb_3()
        .pt_2()
        .gap_2();

    if let Some(notice) = &observer.notice {
        column = column.child(
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
                .child(div().flex_1().min_w_0().child(notice.clone())),
        );
    }

    column
        .child(
            v_flex()
                .w_full()
                .max_w(px(780.))
                .rounded(px(12.))
                .border_1()
                .border_color(border)
                .bg(surface)
                .child(
                    Textarea::new(&observer.composer)
                        .appearance(false)
                        .bordered(false)
                        .h(px(62.))
                        .w_full(),
                )
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .pb_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .text_size(px(10.))
                                .text_color(muted)
                                .child(observer.status_line()),
                        )
                        .when(running, |view| {
                            view.child(
                                h_flex()
                                    .id("stop-workflow")
                                    .flex_none()
                                    .items_center()
                                    .justify_center()
                                    .size(px(26.))
                                    .rounded(px(6.))
                                    .cursor_pointer()
                                    .hover(|style| style.bg(hover))
                                    .child(Icon::new("stop").size(14.).color(danger()))
                                    .on_click(cx.listener(|this, _, _, cx| this.stop_workflow(cx))),
                            )
                        })
                        .child(
                            h_flex()
                                .id("send-message")
                                .flex_none()
                                .items_center()
                                .justify_center()
                                .size(px(26.))
                                .rounded(px(6.))
                                .when(can_send, |view| {
                                    view.cursor_pointer().hover(|style| style.bg(hover))
                                })
                                .child(Icon::new("send").size(16.).color(if can_send {
                                    accent
                                } else {
                                    muted
                                }))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.send_message(window, cx);
                                })),
                        ),
                ),
        )
        .into_any_element()
}
