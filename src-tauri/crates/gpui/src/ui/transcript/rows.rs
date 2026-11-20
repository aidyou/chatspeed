//! Shared row dispatch and stateless rows for the main and child transcripts.

use gpui_kit::component::{h_flex, v_flex, ActiveTheme};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use rust_i18n::t;

use crate::{accent, danger, pill, plain_text, ui::Icon, warning, Observer};
use chatspeed_gpui::presentation::{ActivityView, AlertKind, AlertView, Row};

/// Shared dispatch for the window transcript and nested delegated transcripts.
pub(crate) fn row(window: &Observer, row: &Row, cx: &mut Context<Observer>) -> AnyElement {
    match row {
        Row::User(view) => window.user_row(view, cx),
        Row::Assistant(view) => window.assistant_row(view, cx),
        Row::Activity(group) => window.activity_row(group, cx),
        Row::Tool(tool) => window.tool_row(tool, cx),
        Row::SubAgent(card) => window.sub_agent_row(card, cx),
        Row::FinishTask(view) => window.finish_task_row(view, cx),
        Row::Snapshot(snapshot) => window.snapshot_row(snapshot, cx),
        Row::Alert(alert) => alert_row(alert, cx),
        Row::Divider => divider_row(cx),
    }
}

pub(crate) fn activity_group(
    group: &ActivityView,
    items: Option<Vec<AnyElement>>,
    on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let mut header = h_flex()
        .id(SharedString::from(group.key.clone()))
        .group("activity-header")
        .w_full()
        .min_w_0()
        .items_center()
        .gap(px(8.))
        .h(px(26.))
        .pl_2()
        .rounded(px(6.))
        .cursor_pointer()
        .hover(|style| style.bg(cx.theme().muted))
        .child(Icon::new(group.icon).size(15.).color(muted));
    if group.thought_count > 0 {
        header = header.child(pill(
            format!("{} x{}", t!("thought"), group.thought_count),
            accent(),
            Some(accent().opacity(0.12)),
        ));
    }
    if group.error_count > 0 {
        // Vue tints group errors into the background instead of shouting in red.
        header = header.child(
            div()
                .flex_none()
                .px_2()
                .py(px(1.))
                .rounded(px(4.))
                .bg(cx.theme().border.opacity(0.35))
                .text_size(px(10.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(muted)
                .child(format!("{} x{}", t!("error"), group.error_count)),
        );
    }
    header = header
        .child(
            div()
                .flex_shrink_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(13.))
                .line_height(px(26.))
                .text_color(if group.ongoing {
                    cx.theme().foreground
                } else {
                    muted
                })
                .child(group.summary.clone()),
        )
        .child(
            div()
                .id("activity-caret")
                .flex_none()
                .ml(px(7.))
                .invisible()
                .group_hover("activity-header", |style| style.visible())
                .child(
                    Icon::new(if items.is_some() { "caret-down" } else { "caret-right" })
                        .size(14.)
                        .color(muted),
                ),
        )
        .on_click(on_toggle);
    v_flex()
        .w_full()
        .min_w_0()
        .child(header)
        .when_some(items, |body, items| {
            body.child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap_2()
                    .pl_4()
                    .pt_1()
                    .border_l_1()
                    .border_color(cx.theme().border)
                    .children(items),
            )
        })
        .into_any_element()
}

fn alert_row(view: &AlertView, cx: &App) -> AnyElement {
    let is_warning = view.kind == AlertKind::Warning;
    let (icon, color) = if is_warning {
        ("warning", warning())
    } else {
        ("error", danger())
    };
    v_flex()
        .w_full()
        .min_w_0()
        .gap_2()
        .rounded(px(8.))
        .border_1()
        .border_color(color.opacity(0.45))
        .bg(color.opacity(0.08))
        .px_4()
        .py_3()
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .items_center()
                .gap_2()
                .child(Icon::new(icon).size(18.).color(color))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(color)
                        .child(view.title.clone()),
                ),
        )
        .when(!view.body.is_empty(), |alert| {
            alert.child(
                div()
                    .w_full()
                    .min_w_0()
                    .text_size(px(13.))
                    .text_color(cx.theme().foreground)
                    .child(plain_text(&view.body, cx.theme().foreground)),
            )
        })
        .into_any_element()
}

pub(super) fn divider_row(cx: &App) -> AnyElement {
    h_flex()
        .w_full()
        .items_center()
        .gap_3()
        .py_3()
        .child(div().flex_1().h(px(1.)).bg(cx.theme().border))
        .child(
            div()
                .flex_none()
                .text_size(px(10.))
                .text_color(cx.theme().muted_foreground)
                .child(t!("clear_context_divider").to_string()),
        )
        .child(div().flex_1().h(px(1.)).bg(cx.theme().border))
        .into_any_element()
}
