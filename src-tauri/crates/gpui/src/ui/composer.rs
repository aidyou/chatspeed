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

pub(crate) mod footer;
pub(crate) mod message_queue;
pub(crate) mod model_config;
pub mod panels;
pub mod suggestions;

/// Vue's expanded box tops out at 540px; this window keeps its actions visible.
pub(crate) const EXPANDED_HEIGHT: f32 = 420.;

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
                .mb(px(8.))
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
                .rounded(px(8.))
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
    let height = if expanded {
        Some(px(EXPANDED_HEIGHT))
    } else {
        observer.composer_height
    };
    let natural_height = observer.composer_natural_height.clone();
    let accent = cx.theme().primary;
    let foreground = cx.theme().foreground;
    let focused = observer.composer_focused;

    // Vue sizes the textarea with `flex-grow: 1` beside the footer instead of
    // giving it a percentage height: a `h_full` here resolved against the whole
    // box, so the textarea (and its scrollbar) ran under the footer row and the
    // lines that passed it were hidden behind the buttons.
    let textarea = Textarea::new(&observer.composer)
        .appearance(false)
        .bordered(false)
        .w_full()
        .flex_1()
        .min_h(px(0.));

    // Vue's `.input`: a flat input surface with a hairline border and a soft
    // drop shadow, turning into the accent edge plus a 3px halo on focus. The
    // web stylesheet also stacks a faint gradient, which reads as a heavy wash
    // on a native surface, so the flat `--cs-input-bg-color` is used instead.
    let input_bg: Hsla = if observer.appearance_dark {
        rgb(0x282b2d).into()
    } else {
        gpui_kit::white()
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
        .relative()
        .when_some(height, |view, height| view.h(height))
        .when(height.is_none(), |view| {
            view.child(
                canvas(
                    |_, _, _| (),
                    move |bounds, (), _, _| {
                        natural_height.set(bounds.size.height.min(px(EXPANDED_HEIGHT)));
                    },
                )
                .absolute()
                .size_full(),
            )
        })
        .child(attachment_row(observer, cx))
        .child(
            // The footer below owns its own row, so this area only has to clip
            // what does not fit and keep a hair of breathing room above it.
            div()
                .w_full()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .pb(px(2.))
                .child(textarea),
        )
        .child(footer::footer(observer, cx))
        .child(
            div()
                .id("composer-resize")
                .absolute()
                .top_0()
                .left(px(12.))
                .right(px(32.))
                .h(px(4.))
                .cursor(CursorStyle::ResizeUpDown)
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        let minimum = this.composer_natural_height.get().min(px(EXPANDED_HEIGHT));
                        this.composer_resize =
                            Some((event.position.y, height.unwrap_or(minimum), minimum));
                        cx.stop_propagation();
                    }),
                ),
        )
        .child(
            div()
                .id("composer-expand")
                .group("composer-expand")
                .absolute()
                .top(px(4.))
                .right(px(4.))
                .size(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(8.))
                .cursor_pointer()
                .hover(move |style| style.bg(hover))
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .right_0()
                        .size(px(12.))
                        .rounded_tr(px(8.))
                        .border_t_1()
                        .border_r_1()
                        .border_color(muted.opacity(0.6))
                        .group_hover("composer-expand", |style| style.invisible()),
                )
                .child(
                    div()
                        .invisible()
                        .group_hover("composer-expand", |style| style.visible())
                        .child(
                            Icon::new(if expanded {
                                "fullscreen"
                            } else {
                                "fullscreen-off"
                            })
                            .size(14.)
                            .color(muted),
                        ),
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_composer_expanded(cx))),
        )
        .into_any_element()
}
