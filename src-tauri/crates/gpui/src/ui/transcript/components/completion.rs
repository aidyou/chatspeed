//! Snapshot and completion cards; expansion state belongs to the transcript.

use gpui_kit::component::{h_flex, v_flex, ActiveTheme};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::{accent, danger, success, ui::Icon};
use chatspeed_gpui::presentation::{FinishTaskView, SnapshotView};

use super::cost_card;

pub(crate) fn snapshot_card(
    snapshot: &SnapshotView,
    markdown: Option<AnyElement>,
    on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let open = markdown.is_some();
    let muted = cx.theme().muted_foreground;
    let mut body = v_flex()
        .w_full()
        .min_w_0()
        .mb_2()
        .rounded(px(8.))
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().muted)
        .overflow_hidden()
        .child(
            h_flex()
                .id(SharedString::from(snapshot.key.clone()))
                .w_full()
                .min_w_0()
                .items_center()
                .gap_2()
                .px_3()
                .py_2()
                .cursor_pointer()
                .hover(|style| style.bg(cx.theme().background))
                .child(Icon::new("compress").size(14.).color(accent()))
                .child(
                    div()
                        .flex_none()
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(cx.theme().foreground)
                        .child(snapshot.title.clone()),
                )
                .when(!snapshot.preview.is_empty(), |view| {
                    view.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(10.))
                            .text_color(muted)
                            .child(snapshot.preview.clone()),
                    )
                })
                .when(snapshot.preview.is_empty(), |view| {
                    view.child(div().flex_1())
                })
                .child(
                    Icon::new(if open {
                        "double-arrow-up"
                    } else {
                        "double-arrow-down"
                    })
                    .size(14.)
                    .color(muted),
                )
                .on_click(on_toggle),
        );
    if let Some(markdown) = markdown {
        body = body.child(
            div()
                .w_full()
                .min_w_0()
                .px_3()
                .py_2()
                .border_t_1()
                .border_color(cx.theme().border)
                .child(markdown),
        );
    }
    body.into_any_element()
}

pub(crate) fn finish_task_card(
    view: &FinishTaskView,
    open: bool,
    on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let (icon, color) = if view.failed {
        ("check-x", danger())
    } else {
        ("check-circle", success())
    };
    let mut label = h_flex()
        .id(SharedString::from(view.key.clone()))
        .w_full()
        .min_w_0()
        .items_center()
        .gap_2()
        .pl_2()
        .child(Icon::new(icon).size(15.).color(color))
        .child(
            div()
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(color)
                .child(view.label.clone()),
        );
    if view.cost.is_some() {
        label = label
            .child(
                Icon::new(if open {
                    "double-arrow-up"
                } else {
                    "double-arrow-down"
                })
                .size(12.)
                .color(cx.theme().muted_foreground),
            )
            .cursor_pointer()
            .on_click(on_toggle);
    }
    let mut body = v_flex().w_full().min_w_0().gap_2().child(label);
    if open {
        if let Some(cost) = &view.cost {
            body = body.child(cost_card(cost, cx));
        }
    }
    body.into_any_element()
}
