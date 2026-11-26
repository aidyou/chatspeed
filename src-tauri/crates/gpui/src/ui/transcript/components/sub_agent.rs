//! The delegated-task card shown inline in the transcript.

use gpui_kit::component::{h_flex, v_flex, ActiveTheme};
use gpui_kit::*;
use rust_i18n::t;

use crate::{accent, danger, pill, success, ui::Icon, warning};
use chatspeed_gpui::presentation::{SubAgentStatus, SubAgentView};

fn status_color(status: SubAgentStatus) -> Rgba {
    match status {
        SubAgentStatus::Starting | SubAgentStatus::Running => accent(),
        SubAgentStatus::Completed => success(),
        SubAgentStatus::Failed => danger(),
        SubAgentStatus::Cancelled => rgb(0x848586),
    }
}

pub(crate) struct SubAgentDetails {
    pub task: AnyElement,
    pub result: Option<AnyElement>,
    pub cost: Option<AnyElement>,
    pub open_transcript: Option<AnyElement>,
}

pub(crate) fn sub_agent_card(
    card: &SubAgentView,
    details: Option<SubAgentDetails>,
    on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let open = details.is_some();
    let status_color = status_color(card.status);
    let muted = cx.theme().muted_foreground;
    let header = h_flex()
        .id(SharedString::from(card.key.clone()))
        .group("sub-agent-header")
        .max_w_full()
        .min_w_0()
        .items_center()
        .gap_2()
        .py_1()
        .pl_2()
        .cursor_pointer()
        .child(Icon::new("skill-relation-chart").size(15.).color(muted))
        .child(if card.status.is_running() {
            crate::ui::running_badge(card.status_label.clone(), "sub-agent-running", cx)
                .into_any_element()
        } else {
            pill(
                card.status_label.clone(),
                status_color,
                Some(status_color.opacity(0.12)),
            )
            .into_any_element()
        })
        .child(
            div()
                .flex_shrink_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(13.))
                .text_color(cx.theme().foreground)
                .child(card.agent.clone()),
        )
        .child(crate::ui::disclosure_caret("sub-agent-header", open, muted))
        .on_click(on_toggle);
    let mut body = v_flex().w_full().min_w_0().child(h_flex().min_w_0().child(header));
    if let Some(SubAgentDetails {
        task,
        result,
        cost,
        open_transcript,
    }) = details
    {
        let mut rows = h_flex().w_full().min_w_0().gap_3();
        for (label, value) in [
            (t!("sub_agent_agent").to_string(), card.agent.clone()),
            (t!("sub_agent_mode").to_string(), card.mode.clone()),
            (t!("sub_agent_tools").to_string(), card.tools.clone()),
            (t!("sub_agent_context").to_string(), card.context.clone()),
        ] {
            rows = rows.child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(muted)
                            .child(label.to_uppercase()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .text_size(px(13.))
                            .text_color(cx.theme().foreground)
                            .child(value),
                    ),
            );
        }
        body = body.child(
            v_flex()
                .w_full()
                .min_w_0()
                .gap_3()
                .rounded(px(8.))
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().muted)
                .p_3()
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .gap_2()
                        .child(Icon::new("task").size(15.).color(warning()))
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(cx.theme().foreground)
                                .child(t!("sub_agent_delegated_task").to_string()),
                        )
                        .child(div().flex_1())
                        .children(open_transcript),
                )
                .child(rows)
                .child(task)
                .children(result)
                .children(cost),
        );
    }
    body.into_any_element()
}

pub(crate) fn child_loading(cx: &App) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    h_flex()
        .w_full()
        .items_center()
        .gap_2()
        .pl_2()
        .py_1()
        .text_size(px(11.))
        .text_color(muted)
        .child(
            Icon::new("loading")
                .size(12.)
                .color(muted)
                .spin("child-loading", crate::ui::icon::Spin::Accelerate),
        )
        .child(t!("sub_agent_loading").to_string())
        .into_any_element()
}
