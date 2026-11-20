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
use crate::ui::Icon;
#[derive(Clone, Copy)]
pub(crate) enum Decision {
    Reject,
    Approve,
    Whitelist,
    Batch,
}

#[derive(Debug, PartialEq, Eq)]
enum Change {
    Context,
    Remove,
    Add,
}

// Bound the comparison cost for large replacements; they still show every line.
fn diff_rows(diff: &[(bool, String)]) -> Vec<(Change, &str)> {
    let old: Vec<_> = diff
        .iter()
        .filter(|(added, _)| !added)
        .map(|(_, text)| text.as_str())
        .collect();
    let new: Vec<_> = diff
        .iter()
        .filter(|(added, _)| *added)
        .map(|(_, text)| text.as_str())
        .collect();
    let stride = new.len() + 1;
    if (old.len() + 1).saturating_mul(stride) > 250_000 {
        return old
            .into_iter()
            .map(|line| (Change::Remove, line))
            .chain(new.into_iter().map(|line| (Change::Add, line)))
            .collect();
    }
    let mut lengths = vec![0_u32; (old.len() + 1) * stride];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            let at = i * stride + j;
            lengths[at] = if old[i] == new[j] {
                1 + lengths[at + stride + 1]
            } else {
                lengths[at + stride].max(lengths[at + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut rows = Vec::new();
    while i < old.len() || j < new.len() {
        if i < old.len() && j < new.len() && old[i] == new[j] {
            rows.push((Change::Context, old[i]));
            i += 1;
            j += 1;
        } else if i < old.len()
            && (j == new.len() || lengths[(i + 1) * stride + j] >= lengths[i * stride + j + 1])
        {
            rows.push((Change::Remove, old[i]));
            i += 1;
        } else {
            rows.push((Change::Add, new[j]));
            j += 1;
        }
    }
    rows
}

/// Renders the approval panel shown inside a pending tool row.
pub fn approval_card(
    tool: &ToolView,
    reason: Option<&Entity<gpui_kit::component::input::TextareaState>>,
    pending_count: usize,
    dispatch: impl Fn(Decision, &mut Window, &mut App) + Clone + 'static,
    cx: &App,
) -> AnyElement {
    let border = cx.theme().border;
    let surface = cx.theme().background;
    let muted = cx.theme().muted_foreground;
    let foreground = cx.theme().foreground;
    let accent = cx.theme().primary;
    let danger = crate::danger();

    let mut body = v_flex().w_full().min_w_0().gap_1().text_size(px(11.));
    if let Some(diff) = &tool.diff {
        let mut old = 0;
        let mut new = 0;
        let mut lines = v_flex()
            .id("approval-diff-lines")
            .max_h(px(320.))
            .overflow_y_scroll();
        for (change, line) in diff_rows(diff) {
            let (number, sign, color) = match change {
                Change::Add => {
                    new += 1;
                    (new, "+", Hsla::from(crate::success()))
                }
                Change::Remove => {
                    old += 1;
                    (old, "−", Hsla::from(danger))
                }
                Change::Context => {
                    old += 1;
                    new += 1;
                    (new, " ", Hsla::transparent_black())
                }
            };
            lines = lines.child(
                h_flex()
                    .w_full()
                    .items_start()
                    .gap_2()
                    .px_2()
                    .bg(color.opacity(0.10))
                    .font_family(crate::MONO_FONT)
                    .child(
                        div()
                            .flex_none()
                            .w(px(40.))
                            .text_color(muted)
                            .child(number.to_string()),
                    )
                    .child(div().flex_none().text_color(color).child(sign))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_color(foreground)
                            .child(line.to_owned()),
                    ),
            );
        }
        body = body.child(
            v_flex()
                .rounded(px(6.))
                .border_1()
                .border_color(border)
                .overflow_hidden()
                .child(
                    div()
                        .px_2()
                        .py_1()
                        .bg(cx.theme().muted)
                        .text_color(muted)
                        .child(tool.target.clone()),
                )
                .child(lines),
        );
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
                .child(Icon::new("approval").size(14.).color(accent))
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

    use gpui_kit::component::{
        button::{Button, ButtonVariants},
        input::Textarea,
    };
    let mut actions = h_flex().w_full().flex_wrap().gap_2().justify_end();
    let mut buttons = vec![
        (
            "approval-reject",
            Decision::Reject,
            rust_i18n::t!("reject").into_owned(),
        ),
        (
            "approval-approve",
            Decision::Approve,
            rust_i18n::t!("approve").into_owned(),
        ),
    ];
    if tool.name != "submit_plan" {
        buttons.push((
            "approval-whitelist",
            Decision::Whitelist,
            rust_i18n::t!("approve_whitelist").into_owned(),
        ));
        if pending_count > 1 {
            buttons.push((
                "approval-batch",
                Decision::Batch,
                format!("{} ({pending_count})", rust_i18n::t!("approve_all")),
            ));
        }
    }
    for (id, decision, label) in buttons {
        let dispatch = dispatch.clone();
        let button = Button::new(id).label(label).rounded(px(100.));
        let button = match decision {
            Decision::Reject => button.outline(),
            Decision::Batch => button.warning(),
            Decision::Approve | Decision::Whitelist => button.primary(),
        };
        actions =
            actions.child(button.on_click(move |_, window, cx| dispatch(decision, window, cx)));
    }
    let card = if let Some(reason) = reason {
        card.child(Textarea::new(reason).w_full())
    } else {
        card
    };
    card.child(actions).into_any_element()
}
