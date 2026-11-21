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

use crate::ui::{status, Icon};
use crate::Observer;

mod footer;
pub(crate) mod message_queue;
pub mod panels;
pub mod suggestions;

/// Vue's expanded box tops out at 540px; this window keeps its actions visible.
const EXPANDED_HEIGHT: f32 = 420.;

/// Renders the state line, the prompt box and its footer.
pub fn composer(observer: &Observer, cx: &mut Context<Observer>) -> AnyElement {
    let mut column = v_flex()
        .flex_none()
        .w_full()
        .items_center()
        .px_4()
        .pb_3()
        .pt_2()
        .gap_2();

    // A local answer takes effect before the runtime reports its next phase.
    let answered = observer
        .model
        .awaiting_user_tool_call_id
        .as_deref()
        .is_some_and(|id| observer.ask_user_answered.contains_key(id));
    column = column.child(status::status(
        &observer.model,
        observer.workflow_running(),
        answered,
        cx,
    ));
    if observer.selected.is_some() {
        column = column.child(status::workflow_details(observer, cx));
    }
    let panel = suggestions::panel(observer, cx);
    // Vue floats the panel above the box instead of pushing the transcript up.
    let mut frame = v_flex()
        .relative()
        .w_full()
        .items_center()
        .child(box_body(observer, cx));
    if let Some(panel) = panel {
        frame = frame.child(
            h_flex()
                .absolute()
                .bottom(relative(1.))
                .left(px(0.))
                .right(px(0.))
                .mb(px(6.))
                .justify_center()
                .child(panel),
        );
    }
    column = column.child(frame);
    column.into_any_element()
}

/// Attached images above the textarea, matching Vue's preview strip.
fn attachment_row(observer: &Observer, cx: &mut Context<Observer>) -> AnyElement {
    if observer.attachments.is_empty() {
        return div().into_any_element();
    }
    let border = cx.theme().border;
    let muted = cx.theme().muted_foreground;
    let mut row = h_flex().w_full().flex_wrap().gap_2().px_3().pt_2();
    for (index, attachment) in observer.attachments.iter().enumerate() {
        row = row.child(
            div()
                .relative()
                .flex_none()
                .w(px(96.))
                .rounded(px(6.))
                .border_1()
                .border_color(border)
                .overflow_hidden()
                .child(
                    img(attachment.path.clone())
                        .w_full()
                        .h(px(54.))
                        .object_fit(ObjectFit::Cover),
                )
                .child(
                    div()
                        .w_full()
                        .px_1()
                        .py(px(2.))
                        .text_size(px(10.))
                        .text_color(muted)
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(attachment.name.clone()),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("attachment-remove-{index}")))
                        .absolute()
                        .top(px(2.))
                        .right(px(2.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(16.))
                        .rounded_full()
                        .bg(gpui_kit::black().opacity(0.55))
                        .cursor_pointer()
                        .child(Icon::new("close").size(9.).color(gpui_kit::white()))
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.remove_attachment(index, cx)),
                        ),
                ),
        );
    }
    row.into_any_element()
}

/// One outer shadow, matching the CSS `0 y blur spread color` shorthand.
///
/// gpui-kit does not re-export `box_shadow`, so the struct is built here.
fn drop_shadow(y: f32, blur: f32, spread: f32, color: Hsla) -> BoxShadow {
    BoxShadow {
        offset: point(px(0.), px(y)),
        blur_radius: px(blur),
        spread_radius: px(spread),
        inset: false,
        color,
    }
}

/// The framed box: the textarea, its expand toggle and the footer.
fn box_body(observer: &Observer, cx: &mut Context<Observer>) -> AnyElement {
    let border = cx.theme().border;
    let hover = cx.theme().muted;
    let muted = cx.theme().muted_foreground;
    let expanded = observer.composer_expanded;
    let accent = cx.theme().primary;
    let foreground = cx.theme().foreground;
    let focused = observer.composer_focused;

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

    // Vue's `.input`: a flat input surface with a hairline border and a soft
    // drop shadow, turning into the accent edge plus a 3px halo on focus. The
    // web stylesheet also stacks a faint gradient, which reads as a heavy wash
    // on a native surface, so the flat `--cs-input-bg-color` is used instead.
    let input_bg = if observer.appearance_dark {
        rgb(0x282b2d)
    } else {
        rgb(0xe5e6e5)
    };
    let (edge, shadows) = if focused {
        (
            accent.opacity(0.68),
            vec![
                drop_shadow(0., 0., 3., accent.opacity(0.12)),
                drop_shadow(12., 30., 0., foreground.opacity(0.07)),
            ],
        )
    } else {
        (
            border,
            vec![drop_shadow(10., 28., 0., foreground.opacity(0.05))],
        )
    };

    v_flex()
        .id("composer-box")
        .w_full()
        .min_w_0()
        .max_h(px(EXPANDED_HEIGHT))
        .rounded(px(12.))
        .border_1()
        .border_color(edge)
        .bg(input_bg)
        .shadow(shadows)
        // Vue's frame focuses the textarea when the padding around it is
        // clicked, not only the text itself.
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, window, cx| {
                let composer = this.composer.clone();
                composer.update(cx, |state, cx| state.focus(window, cx));
            }),
        )
        .when(!focused, |view| {
            view.hover(move |style| style.border_color(accent.opacity(0.26)))
        })
        .when(expanded, |view| view.h(px(EXPANDED_HEIGHT)))
        .child(attachment_row(observer, cx))
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
