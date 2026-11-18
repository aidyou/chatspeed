//! Inline approval card for pending tool calls and `submit_plan`.
//!
//! The card mirrors `src/components/workflow/ApprovalDialog.vue`: it renders
//! only while the runtime waits on that tool call, keeps the diff, command or
//! plan visible, and sends the decision through the workflow `approval` signal.
//! Once the decision lands the row keeps its diff as a normal collapsible
//! detail, which is why the card disappears on the next projection.

use gpui_kit::component::{h_flex, v_flex, ActiveTheme};
use gpui_kit::*;

use crate::presentation::ToolView;
use crate::Observer;

/// Renders the approval panel shown inside a pending tool row.
pub fn approval_card(tool: &ToolView, cx: &mut Context<Observer>) -> AnyElement {
    let border = cx.theme().border;
    let surface = cx.theme().muted;
    let muted = cx.theme().muted_foreground;
    let foreground = cx.theme().foreground;
    let accent = cx.theme().primary;
    let danger = crate::danger();
    let id = tool.tool_call_id.clone();

    let mut body = v_flex().w_full().min_w_0().gap_1().text_size(px(11.));
    if let Some(diff) = &tool.diff {
        for (added, line) in diff {
            body = body.child(
                div()
                    .w_full()
                    .min_w_0()
                    .px_2()
                    .font_family(crate::MONO_FONT)
                    .text_color(if *added {
                        Hsla::from(crate::success())
                    } else {
                        muted
                    })
                    .child(line.clone()),
            );
        }
    } else if !tool.detail.is_empty() {
        let color = if tool.name == "submit_plan" {
            foreground
        } else {
            muted
        };
        body = body.child(crate::plain_text(&tool.detail, color));
    }

    let card = v_flex()
        .w_full()
        .min_w_0()
        .gap_2()
        .mt_2()
        .p_3()
        .rounded(px(8.))
        .border_1()
        .border_color(accent.opacity(0.45))
        .bg(surface)
        .child(
            h_flex()
                .w_full()
                .items_center()
                .gap_2()
                .child(crate::font_icon("approval", 14., accent))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(foreground)
                        .child(rust_i18n::t!("awaiting_approval").to_string()),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(px(11.))
                        .text_color(muted)
                        .child(tool.action.clone()),
                ),
        )
        .child(body);

    let reject_id = id.clone();
    let approve_id = id.clone();
    card.child(
        h_flex()
            .w_full()
            .items_center()
            .gap_2()
            .child(
                h_flex()
                    .id("approval-reject")
                    .flex_none()
                    .items_center()
                    .gap_1()
                    .px_3()
                    .py(px(4.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(border)
                    .cursor_pointer()
                    .text_size(px(11.))
                    .text_color(muted)
                    .hover(|style| style.border_color(danger).text_color(danger))
                    .child(crate::font_icon("close", 12., muted))
                    .child(rust_i18n::t!("reject").to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.submit_approval(reject_id.clone(), false, false, cx)
                    })),
            )
            .child(div().flex_1())
            .child(
                h_flex()
                    .id("approval-approve")
                    .flex_none()
                    .items_center()
                    .gap_1()
                    .px_3()
                    .py(px(4.))
                    .rounded(px(6.))
                    .bg(accent)
                    .cursor_pointer()
                    .text_size(px(11.))
                    .text_color(gpui_kit::white())
                    .child(crate::font_icon("check", 12., gpui_kit::white()))
                    .child(rust_i18n::t!("approve").to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.submit_approval(approve_id.clone(), true, false, cx)
                    })),
            ),
    )
    .into_any_element()
}
