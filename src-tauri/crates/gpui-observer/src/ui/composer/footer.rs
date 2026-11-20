//! Footer under the prompt box.
//!
//! Mirrors the `.input-footer` block of `src/components/workflow/WorkflowInputArea.vue`:
//! the quick actions on the left and the continue/stop/send group on the right.
//! Keeping it separate leaves room for Vue's model, agent and tool selectors
//! without growing the composer's layout code.

use gpui_kit::component::{h_flex, ActiveTheme};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use rust_i18n::t;

use crate::ui::Icon;
use crate::{danger, Observer};

/// The actions under the box, in Vue's left/right arrangement.
pub fn footer(observer: &Observer, cx: &mut Context<Observer>) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let accent = cx.theme().primary;
    let hover = cx.theme().muted;
    let running = observer.workflow_running();
    let can_send = !observer.composer.read(cx).value().trim().is_empty() && !observer.start_pending;

    h_flex()
        .w_full()
        .items_center()
        .gap_2()
        .px_3()
        .pb_2()
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap_1()
                .when(observer.selected.is_some(), |row| {
                    row.child(icon_action(
                        "composer-compress",
                        "compress",
                        muted,
                        hover,
                        cx.listener(|this, _, _, cx| this.manual_compress(cx)),
                    ))
                })
                .when(observer.can_continue(), |row| {
                    row.child(continue_button(accent, hover, cx))
                }),
        )
        .when(running, |row| {
            row.child(icon_action(
                "stop-workflow",
                "stop",
                danger(),
                hover,
                cx.listener(|this, _, _, cx| this.stop_workflow(cx)),
            ))
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
                .child(
                    Icon::new("send")
                        .size(16.)
                        .color(if can_send { accent } else { muted }),
                )
                .on_click(cx.listener(|this, _, window, cx| this.send_message(window, cx))),
        )
        .into_any_element()
}

/// One icon-only footer action.
fn icon_action(
    id: &'static str,
    icon: &'static str,
    color: impl Into<Hsla>,
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
        .hover(move |style| style.bg(hover))
        .child(Icon::new(icon).size(14.).color(color))
        // A press that lands here must not also reach the box behind it.
        .on_mouse_down(MouseButton::Left, |_, window, cx| {
            window.prevent_default();
            cx.stop_propagation();
        })
        .on_click(on_click)
        .into_any_element()
}

/// Vue offers "Continue" for every resumable status.
fn continue_button(accent: Hsla, hover: Hsla, cx: &mut Context<Observer>) -> AnyElement {
    h_flex()
        .id("continue-workflow")
        .flex_none()
        .items_center()
        .gap_1()
        .px_2()
        .py(px(2.))
        .rounded(px(6.))
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .text_size(px(11.))
        .text_color(accent)
        .child(Icon::new("send").size(11.).color(accent))
        .child(t!("continue_workflow").to_string())
        .on_click(cx.listener(|this, _, _, cx| this.continue_workflow(cx)))
        .into_any_element()
}
