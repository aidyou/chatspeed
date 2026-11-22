//! Shared row dispatch and stateless rows for the main and child transcripts.

use gpui_kit::component::{h_flex, v_flex, ActiveTheme};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use rust_i18n::t;

use crate::{accent, danger, pill, plain_text, ui::Icon, warning, MsgScope, Observer};
use chatspeed_gpui::presentation::{ActivityView, AlertKind, AlertView, Row};

pub(crate) fn latest_active_group(rows: &[Row], running: bool) -> Option<&str> {
    if !running {
        return None;
    }
    rows.iter().rev().find_map(|row| match row {
        Row::Activity(group) if group.ongoing => Some(group.key.as_str()),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::{latest_active_group, ActivityView, Row};

    #[test]
    fn sweep_selects_latest_running_group_even_with_later_non_group_rows() {
        let group = |key: &str, ongoing| {
            Row::Activity(ActivityView {
                key: key.into(),
                icon: "tool",
                summary: String::new(),
                thought_count: 0,
                error_count: 0,
                ongoing,
                items: Vec::new(),
            })
        };
        let rows = [
            group("first", true),
            group("latest", true),
            group("finished", false),
            Row::Divider,
        ];
        assert_eq!(latest_active_group(&rows, true), Some("latest"));
        assert_eq!(latest_active_group(&rows, false), None);
        assert_eq!(latest_active_group(&rows[2..], true), None);
    }
}

/// Shared dispatch for the window transcript and nested delegated transcripts.
pub(crate) fn row(
    window: &Observer,
    row: &Row,
    active_group: Option<&str>,
    scope: MsgScope,
    cx: &mut Context<Observer>,
) -> AnyElement {
    match row {
        Row::User(view) => window.user_row(view, scope, cx),
        Row::Assistant(view) => window.assistant_row(view, scope, cx),
        Row::Activity(group) => window.activity_row(
            group,
            active_group == Some(group.key.as_str()),
            scope,
            cx,
        ),
        Row::Tool(tool) => window.tool_row(tool, scope, cx),
        Row::SubAgent(card) => window.sub_agent_row(card, cx),
        Row::FinishTask(view) => window.finish_task_row(view, cx),
        Row::Snapshot(snapshot) => window.snapshot_row(snapshot, cx),
        Row::Alert(alert) => alert_row(alert, cx),
        Row::Divider => divider_row(cx),
    }
}

pub(crate) fn activity_group(
    group: &ActivityView,
    shimmer: bool,
    items: Option<Vec<AnyElement>>,
    on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    // The press hugs the icon and the summary; the rest of the line stays inert.
    let mut header = h_flex()
        .id(SharedString::from(group.key.clone()))
        .group("activity-header")
        .flex_none()
        .max_w_full()
        .min_w_0()
        .items_center()
        .gap(px(8.))
        .h(px(26.))
        .px(px(6.))
        .rounded(px(6.))
        .cursor_pointer();
    let primary = cx.theme().primary;
    let mut title = h_flex()
        .relative()
        .overflow_hidden()
        .h(px(22.))
        .min_w_0()
        .gap_2()
        .child(Icon::new(group.icon).size(15.).color(muted));
    if group.thought_count > 0 {
        title = title.child(pill(
            format!("{} x{}", t!("thought"), group.thought_count),
            accent(),
            Some(accent().opacity(0.12)),
        ));
    }
    if group.error_count > 0 {
        // Vue tints group errors into the background instead of shouting in red.
        title = title.child(
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
    title = title.child(
        div()
            .flex_shrink_1()
            .min_w_0()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .text_size(px(12.))
            .line_height(px(18.))
            .text_color(if group.ongoing {
                cx.theme().foreground
            } else {
                muted
            })
            .child(crate::ui::chrome_label(group.summary.clone())),
    );
    if shimmer && !cx.reduce_motion() {
        let clear = primary.opacity(0.);
        let peak = primary.opacity(0.35);
        title = title.child(
            div().absolute().inset_0().with_animation(
                "activity-sweep",
                Animation::new(std::time::Duration::from_secs(3))
                    .repeat_synced()
                    .with_max_fps(30.)
                    .with_easing(ease_in_out),
                move |track, phase| {
                    track.child(
                        canvas(
                            |_, _, _| (),
                            move |bounds, (), window, _| {
                                let half = bounds.size.width * 0.225;
                                let x = bounds.origin.x + (bounds.size.width + half * 2.) * phase
                                    - half * 2.;
                                let size = size(half, bounds.size.height);
                                for (offset, from, to) in
                                    [(px(0.), clear, peak), (half, peak, clear)]
                                {
                                    let band =
                                        Bounds::new(point(x + offset, bounds.origin.y), size);
                                    window.paint_quad(fill(
                                        band,
                                        linear_gradient(
                                            90.,
                                            linear_color_stop(from, 0.),
                                            linear_color_stop(to, 1.),
                                        ),
                                    ));
                                }
                            },
                        )
                        .size_full(),
                    )
                },
            ),
        );
    }
    header = header
        .child(title)
        .child(
            div()
                .id("activity-caret")
                .flex_none()
                .ml(px(7.))
                .invisible()
                .group_hover("activity-header", |style| style.visible())
                .child(crate::ui::rotating_icon(
                    format!("activity-{}", group.key),
                    "caret-right",
                    14.,
                    muted,
                    if items.is_some() { 0. } else { 90. },
                    if items.is_some() { 90. } else { 0. },
                )),
        )
        .on_click(on_toggle);
    v_flex()
        .w_full()
        .min_w_0()
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .items_center()
                .pl(px(2.))
                .child(header),
        )
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
