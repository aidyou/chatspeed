//! Live status above the prompt box.
//!
//! Mirrors `src/components/workflow/StatusNotifier.vue`: the line reports what
//! the runtime is doing *now* — the tool it is waiting on or running, the latest
//! streamed sentence, or the plain thinking state — instead of a static list of
//! states. The order of the checks is the same as Vue's `displayState`, because
//! a pending approval must win over streamed text and vice versa.

use std::time::Duration;

use gpui_kit::base::{GlobalState, TextSelection};
use gpui_kit::component::{h_flex, v_flex, ActiveTheme};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use rust_i18n::t;
use serde_json::Value;

use crate::presentation::{format_display_path, single_line, tool_arguments, tool_name};
use crate::MONO_FONT;
use crate::ui::Icon;
use crate::{accent, danger, info, success, violet, warning, Model};

/// How long a finished tool stays in the status line, as in Vue.
const COMPLETED_VISIBLE: Duration = Duration::from_secs(5);

/// The longest text the line shows before it ellipsizes.
const PREVIEW_CHARS: usize = 72;

/// Tools that always wait for an approval before they run.
const APPROVAL_TOOLS: [&str; 3] = ["edit_file", "write_file", "bash"];

struct Live {
    text: String,
    icon: &'static str,
    warning: bool,
}

/// The runtime state line between the transcript and the box.
pub fn status(model: &Model, running: bool, answered: bool, cx: &App) -> AnyElement {
    render_status(live_state(model, running, answered), cx)
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum PanelMode {
    Circle,
    Compact,
    Expanded,
}

/// Vue's panel geometry: a 44px trigger, a 140x40 collapsed bar and a 350px
/// panel whose body stops growing at 600px.
const PANEL_WIDTH: f32 = 350.;
const COMPACT_WIDTH: f32 = 140.;
const HEADER_HEIGHT: f32 = 40.;
const TRIGGER_SIZE: f32 = 44.;
const BODY_MAX_HEIGHT: f32 = 600.;
const DEFAULT_RIGHT: f32 = 5.;
const DEFAULT_TOP: f32 = 160.;
const MIN_TOP_GAP: f32 = 5.;
const TODO_PREVIEW: usize = 10;
const RECENT_LIMIT: usize = 3;
const CHILD_LIMIT: usize = 5;
const RING_DOTS: usize = 32;
const TAU: f32 = std::f32::consts::TAU;

fn clamp_px(value: Pixels, low: Pixels, high: Pixels) -> Pixels {
    value.max(low).min(high.max(low))
}

/// Vue counts a closed task as done even when it ended badly.
fn is_done(task: &Value) -> bool {
    matches!(
        task["status"].as_str().unwrap_or(""),
        "completed" | "failed" | "data_missing"
    )
}

fn todo_status(task: &Value) -> &str {
    task["status"].as_str().unwrap_or("")
}

/// Vue's `contextUsageStatusClass`.
fn context_class(percent: u32) -> &'static str {
    if percent >= 90 {
        "complete"
    } else if percent >= 70 {
        "good"
    } else if percent >= 40 {
        "medium"
    } else {
        "start"
    }
}

/// Vue's `progressStatusClass`.
fn progress_class(percent: u32) -> &'static str {
    if percent == 100 {
        "complete"
    } else if percent >= 60 {
        "good"
    } else if percent >= 30 {
        "medium"
    } else {
        "start"
    }
}

fn context_fill(class: &str) -> Rgba {
    match class {
        "good" => warning(),
        "complete" => danger(),
        _ => success(),
    }
}

fn task_fill(class: &str) -> Rgba {
    match class {
        "start" => info(),
        "medium" => accent(),
        _ => success(),
    }
}

/// Tool-type rail colors from Vue's `.op-item.tool-*` rules.
fn tool_class(name: &str) -> &'static str {
    match name {
        "read_file" | "write_file" | "edit_file" | "list_dir" | "glob" | "grep" => "tool-file",
        "web_fetch" | "web_search" => "tool-network",
        "bash" => "tool-system",
        "todo_create" | "todo_update" | "todo_list" | "submit_plan" | "complete_workflow" => {
            "tool-todo"
        }
        _ => "tool-system",
    }
}

fn tool_class_color(class: &str) -> Rgba {
    match class {
        "tool-file" => accent(),
        "tool-network" => success(),
        "tool-todo" => violet(),
        _ => warning(),
    }
}

fn grouped(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

fn dragged(event: &ClickEvent) -> bool {
    match event {
        ClickEvent::Mouse(event) => {
            let delta = event.up.position - event.down.position;
            delta.x.abs() > px(4.) || delta.y.abs() > px(4.)
        }
        _ => false,
    }
}

/// A press on the panel starts the move and, like Vue's `user-select: none`
/// header, must not seed a text selection in the transcript below it.
fn begin_drag(
    this: &mut crate::Observer,
    event: &MouseDownEvent,
    origin: Point<Pixels>,
    window: &mut Window,
    cx: &mut Context<crate::Observer>,
) {
    window.prevent_default();
    GlobalState::suppress_text_selection(cx);
    TextSelection::clear(window, cx);
    cx.stop_propagation();
    this.status_panel_drag = Some(event.position - origin);
}

pub(crate) fn overlay(
    observer: &crate::Observer,
    area: Bounds<Pixels>,
    cx: &mut Context<crate::Observer>,
) -> Option<AnyElement> {
    observer.selected.as_ref()?;
    let mode = observer.status_panel_mode;
    let background = cx.theme().background;
    let surface = cx.theme().muted;
    let border = cx.theme().border;
    let foreground = cx.theme().foreground;
    let muted = cx.theme().muted_foreground;
    let primary = cx.theme().primary;
    let danger_color = Hsla::from(danger());
    let hover = crate::ui::surfaces(observer.appearance_dark).hover;

    let total = observer.model.todo_list.len();
    let completed = observer
        .model
        .todo_list
        .iter()
        .filter(|task| is_done(task))
        .count();
    let percent = completed
        .checked_mul(100)
        .and_then(|value| value.checked_div(total))
        .unwrap_or(0) as u32;
    let context = observer
        .model
        .current_context_tokens
        .zip(observer.model.max_context_tokens)
        .filter(|(_, max)| *max > 0)
        .map(|(used, max)| ((used.min(max) * 100) / max) as u32)
        .map(|percent| percent.min(100));

    let (width, height) = match mode {
        PanelMode::Circle => (px(TRIGGER_SIZE), px(TRIGGER_SIZE)),
        PanelMode::Compact => (px(COMPACT_WIDTH), px(HEADER_HEIGHT)),
        PanelMode::Expanded => (
            px(PANEL_WIDTH),
            (area.size.height - px(MIN_TOP_GAP)).min(px(BODY_MAX_HEIGHT + HEADER_HEIGHT)),
        ),
    };
    let width = width.min(area.size.width);
    let height = height.min(area.size.height);
    let default = point(
        area.origin.x + area.size.width - width - px(DEFAULT_RIGHT),
        area.origin.y + px(DEFAULT_TOP),
    );
    let position = observer.status_panel_position.unwrap_or(default);
    let origin = point(
        clamp_px(
            position.x,
            area.origin.x,
            area.origin.x + area.size.width - width,
        ),
        clamp_px(
            position.y,
            area.origin.y + px(MIN_TOP_GAP),
            area.origin.y + area.size.height - height,
        ),
    );

    if mode == PanelMode::Circle {
        let class = context_class(context.unwrap_or(0));
        let ring: Hsla = context_fill(class).into();
        let mut trigger = div()
            .id("status-panel-trigger")
            .occlude()
            .absolute()
            .left(origin.x)
            .top(origin.y)
            .size(px(TRIGGER_SIZE))
            .rounded_full()
            .border_1()
            .border_color(border)
            .bg(background)
            .shadow_md()
            .cursor_pointer()
            .hover(move |style| style.bg(surface))
            .child(
                div()
                    .absolute()
                    .left(px(3.))
                    .top(px(3.))
                    .right(px(3.))
                    .bottom(px(3.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(Icon::new("list").size(18.).color(primary)),
            );
        if let Some(used_percent) = context {
            let fraction = used_percent as f32 / 100.;
            let center = TRIGGER_SIZE / 2.;
            let radius = TRIGGER_SIZE / 2. - 4.;
            for segment in 0..RING_DOTS {
                let angle = segment as f32 * TAU / RING_DOTS as f32;
                let filled = (segment as f32) < fraction * RING_DOTS as f32;
                trigger = trigger.child(
                    div()
                        .absolute()
                        .left(px(center - 1. + radius * angle.sin()))
                        .top(px(center - 1. - radius * angle.cos()))
                        .size(px(2.))
                        .rounded_full()
                        .bg(if filled {
                            ring
                        } else {
                            border.opacity(0.6)
                        }),
                );
            }
        }
        if percent > 0 {
            trigger = trigger.child(
                div()
                    .absolute()
                    .top(px(-4.))
                    .right(px(-4.))
                    .min_w(px(20.))
                    .px(px(4.))
                    .py(px(2.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(primary)
                    .border_2()
                    .border_color(background)
                    .text_size(px(10.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(cx.theme().primary_foreground)
                    .child(format!("{percent}%")),
            );
        }
        return Some(
            trigger
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        begin_drag(this, event, origin, window, cx);
                    }),
                )
                .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                    if dragged(event) {
                        return;
                    }
                    this.status_panel_mode = PanelMode::Expanded;
                    cx.notify();
                }))
                .into_any_element(),
        );
    }

    let mut left = h_flex()
        .min_w_0()
        .items_center()
        .gap(px(8.))
        .child(Icon::new("list").size(14.).color(muted))
        // Vue drags from anywhere in the header, so every non-button child of
        // the header owns the same press handler instead of only the icon.
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                begin_drag(this, event, origin, window, cx);
            }),
        );
    if mode == PanelMode::Expanded {
        left = left.child(
            div()
                .flex_none()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(foreground)
                .child(t!("status_panel_title").to_string())
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        begin_drag(this, event, origin, window, cx);
                    }),
                ),
        );
    }
    let mut actions = h_flex().flex_none().items_center().gap(px(8.));
    let chip = |label: String| {
        div()
            .flex_none()
            .px(px(8.))
            .py(px(2.))
            .rounded(px(8.))
            .bg(primary.opacity(0.08))
            .text_size(px(10.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(primary)
            .child(label)
            .into_any_element()
    };
    match mode {
        PanelMode::Compact => actions = actions.child(chip(format!("{percent}%"))),
        PanelMode::Expanded if total > 0 => {
            actions = actions.child(chip(format!("{completed}/{total}")))
        }
        _ => {}
    }
    let collapse_target = mode;
    actions = actions
        .child(
            div()
                .id("status-panel-collapse")
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .size(px(22.))
                .rounded_full()
                .cursor_pointer()
                .hover(move |style| style.bg(hover))
                .child(
                    Icon::new(match collapse_target {
                        PanelMode::Compact => "fullscreen-off",
                        _ => "minimize",
                    })
                    .size(14.)
                    .color(muted),
                )
                // The bar is a drag surface, so the controls claim their press.
                .on_mouse_down(MouseButton::Left, |_, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.status_panel_mode = match collapse_target {
                        PanelMode::Compact => PanelMode::Expanded,
                        _ => PanelMode::Compact,
                    };
                    cx.notify();
                })),
        )
        .child(
            div()
                .id("status-panel-close")
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .size(px(22.))
                .rounded_full()
                .cursor_pointer()
                .hover(move |style| style.bg(danger_color.opacity(0.12)))
                .child(Icon::new("close").size(12.).color(muted))
                .on_mouse_down(MouseButton::Left, |_, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                })
                .on_click(cx.listener(|this, _, _, cx| {
                    cx.stop_propagation();
                    this.status_panel_mode = PanelMode::Circle;
                    cx.notify();
                })),
        );

    // The collapsed bar reports context usage as a leading edge wash.
    let usage_bar = match (mode, context) {
        (PanelMode::Compact, Some(percent)) => Some(
            div()
                .absolute()
                .left_0()
                .top_0()
                .bottom_0()
                .w(relative(percent as f32 / 100.))
                .bg(context_fill(context_class(percent)).opacity(0.2))
                .into_any_element(),
        ),
        _ => None,
    };
    let mut header = h_flex()
        .id("status-panel-header")
        .w_full()
        .h(px(HEADER_HEIGHT))
        .flex_none()
        .items_center()
        .justify_between()
        .gap(px(8.))
        .px(px(12.))
        .cursor_pointer()
        .bg(surface)
        .border_color(border)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                begin_drag(this, event, origin, window, cx);
            }),
        )
        .children(usage_bar)
        .child(left)
        .child(actions);
    if mode == PanelMode::Expanded {
        header = header.border_b_1();
    }

    let mut panel = v_flex()
        .id("status-panel-overlay")
        .occlude()
        .absolute()
        .left(origin.x)
        .top(origin.y)
        .w(width)
        .h(height)
        .bg(background)
        .border_1()
        .border_color(border)
        .shadow_md()
        .overflow_hidden()
        .rounded(px(16.))
        .child(header);
    if mode == PanelMode::Expanded {
        panel = panel.child(
            div()
                .id("status-panel-body")
                .flex_1()
                .min_h_0()
                .w_full()
                .overflow_y_scroll()
                .p(px(12.))
                .child(workflow_details(observer, cx)),
        );
    }
    Some(panel.into_any_element())
}

fn section_header(icon: &'static str, title: String, meta: Option<String>, cx: &App) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    h_flex()
        .w_full()
        .min_w_0()
        .items_center()
        .gap(px(8.))
        .mb(px(8.))
        .child(Icon::new(icon).size(14.).color(cx.theme().primary))
        .child(
            div()
                .flex_none()
                .text_size(px(10.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(muted)
                .child(title.to_uppercase()),
        )
        .child(div().flex_1())
        .children(
            meta.map(|meta| div().flex_none().text_size(px(10.)).text_color(muted).child(meta)),
        )
        .into_any_element()
}

fn progress_bar(percent: u32, fill: Rgba, cx: &App) -> AnyElement {
    h_flex()
        .w_full()
        .min_w_0()
        .items_center()
        .gap(px(8.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .h(px(8.))
                .rounded_full()
                .bg(cx.theme().muted)
                .overflow_hidden()
                .child(
                    div()
                        .w(relative(percent as f32 / 100.))
                        .h_full()
                        .rounded_full()
                        .bg(fill),
                ),
        )
        .child(
            div()
                .flex_none()
                .min_w(px(36.))
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(cx.theme().foreground)
                .text_align(TextAlign::Right)
                .child(format!("{percent}%")),
        )
        .into_any_element()
}

fn empty_state(icon: &'static str, text: String, compact: bool, cx: &App) -> AnyElement {
    v_flex()
        .w_full()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .py(if compact { px(16.) } else { px(24.) })
        .px(px(12.))
        .child(Icon::new(icon).size(if compact { 24. } else { 28. }).color(cx.theme().muted_foreground))
        .child(
            div()
                .text_size(px(12.))
                .text_color(cx.theme().muted_foreground)
                .child(text),
        )
        .into_any_element()
}

struct ModelRow {
    label: String,
    name: String,
    source: String,
    source_class: &'static str,
}

fn model_name(config: &Value) -> Option<String> {
    config["model"]
        .as_str()
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

fn model_rows(observer: &crate::Observer) -> Vec<ModelRow> {
    let configs = observer.model_configs();
    let active = observer.active_model_config();
    let planning = observer.planning_mode;
    let mut rows = Vec::new();
    if let Some(name) = model_name(&configs["plan"]) {
        rows.push(ModelRow {
            label: t!("status_panel_plan_model").to_string(),
            name,
            source: if planning {
                t!("status_panel_active").to_string()
            } else {
                t!("status_panel_direct").to_string()
            },
            source_class: if planning { "active" } else { "direct" },
        });
    }
    if let Some(name) = model_name(&active) {
        rows.push(ModelRow {
            label: t!("status_panel_work_model").to_string(),
            name,
            source: t!("status_panel_active").to_string(),
            source_class: "active",
        });
    }
    let utility = model_name(&configs["utility"])
        .map(|name| (name, t!("status_panel_direct").to_string(), "direct"))
        .or_else(|| {
            model_name(&active).map(|name| {
                (
                    name,
                    t!("status_panel_fallback").to_string(),
                    "fallback",
                )
            })
        });
    if let Some((name, source, source_class)) = &utility {
        rows.push(ModelRow {
            label: t!("status_panel_utility_model").to_string(),
            name: name.clone(),
            source: source.clone(),
            source_class: *source_class,
        });
    }
    if let Some(name) = model_name(&configs["lite"]) {
        rows.push(ModelRow {
            label: t!("status_panel_lite_model").to_string(),
            name,
            source: t!("status_panel_direct").to_string(),
            source_class: "direct",
        });
    }
    if let Some(name) = model_name(&configs["vision"]) {
        rows.push(ModelRow {
            label: t!("status_panel_vision_model").to_string(),
            name,
            source: t!("status_panel_direct").to_string(),
            source_class: "direct",
        });
    }
    let global_title = &observer.status_title_model;
    let title = model_name(&configs["lite"])
        .map(|name| (name, t!("status_panel_direct").to_string(), "direct"))
        .or_else(|| {
            (global_title["id"].as_i64().unwrap_or_default() > 0)
                .then(|| model_name(global_title))
                .flatten()
                .map(|name| (name, t!("status_panel_global").to_string(), "global"))
        })
        .or_else(|| {
            utility
                .as_ref()
                .map(|(name, _, _)| {
                    (
                        name.clone(),
                        t!("status_panel_utility_fallback").to_string(),
                        "fallback",
                    )
                })
        })
        .or_else(|| {
            model_name(&active).map(|name| {
                (
                    name,
                    t!("status_panel_work_fallback").to_string(),
                    "fallback",
                )
            })
        });
    if let Some((name, source, source_class)) = title {
        rows.push(ModelRow {
            label: t!("status_panel_title_model").to_string(),
            name,
            source,
            source_class,
        });
    }
    rows
}

fn child_rows(observer: &crate::Observer) -> (Vec<crate::presentation::SubAgentView>, usize) {
    use crate::presentation::{live_child_card, Row};
    let mut cards: Vec<crate::presentation::SubAgentView> = Vec::new();
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for row in &observer.rows {
        if let Row::SubAgent(card) = row {
            if let Some(id) = card.task_id.clone() {
                if seen.insert(id) {
                    cards.push((**card).clone());
                }
            }
        }
    }
    for (id, progress) in &observer.model.sub_agent_progress {
        if seen.contains(id) {
            continue;
        }
        if let Some(card) = live_child_card(id, progress) {
            seen.insert(id.clone());
            cards.push(card);
        }
    }
    let total = cards.len();
    if cards.len() > CHILD_LIMIT {
        cards = cards.split_off(total - CHILD_LIMIT);
    }
    (cards, total)
}

fn child_card(
    card: &crate::presentation::SubAgentView,
    hover: Hsla,
    cx: &mut Context<crate::Observer>,
) -> AnyElement {
    use crate::presentation::SubAgentStatus;
    let color = match card.status {
        SubAgentStatus::Running => Hsla::from(accent()),
        SubAgentStatus::Completed => Hsla::from(success()),
        SubAgentStatus::Failed | SubAgentStatus::Cancelled => Hsla::from(danger()),
    };
    let muted = cx.theme().muted_foreground;
    let foreground = cx.theme().foreground;
    let background = cx.theme().background;
    let mut pill_row = h_flex()
        .flex_none()
        .items_center()
        .gap(px(4.))
        .min_w(px(58.))
        .justify_center()
        .px(px(8.))
        .py(px(2.))
        .rounded_full()
        .bg(background)
        .text_size(px(10.))
        .line_height(px(14.))
        .text_color(color);
    if card.status.is_running() {
        pill_row = pill_row.child(
            Icon::new("loading")
                .size(10.)
                .color(color)
                .spin(SharedString::from(format!("status-child-spin-{}", card.key)), crate::ui::icon::Spin::Eased),
        );
    }
    pill_row = pill_row.child(card.status_label.clone());

    let id = card.task_id.clone();
    let title = card.agent.clone();
    v_flex()
        .id(SharedString::from(format!("status-child-{}", card.key)))
        .w_full()
        .min_w_0()
        .gap(px(8.))
        .p(px(8.))
        .pl(px(12.))
        .mb(px(8.))
        .rounded(px(4.))
        .relative()
        .overflow_hidden()
        .bg(cx.theme().muted)
        .border_1()
        .border_color(cx.theme().border)
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(
            div()
                .absolute()
                .left_0()
                .top_0()
                .bottom_0()
                .w(px(3.))
                .bg(color),
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            let Some(id) = id.clone() else { return; };
            let durable = this.rows.iter().rposition(
                |row| matches!(row, crate::presentation::Row::SubAgent(card) if card.task_id.as_ref() == Some(&id)),
            );
            let index = durable.or_else(|| {
                this.model
                    .sub_agent_progress
                    .iter()
                    .filter(|(child, _)| {
                        !this.rows.iter().any(|row| matches!(row, crate::presentation::Row::SubAgent(card) if card.task_id.as_ref() == Some(*child)))
                    })
                    .position(|(child, _)| child == &id)
                    .map(|index| this.rows.len() + index)
            });
            if let Some(index) = index {
                this.timeline_scroll.follow = false;
                this.timeline_scroll.scroll_pending = false;
                this.timeline_scroll
                    .scroll
                    .scroll_to_item(index + usize::from(this.model.hidden > 0) + usize::from(this.switched.is_some()));
            } else {
                this.open_child_session(id, title.clone(), cx);
            }
            cx.notify();
        }))
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .items_center()
                .justify_between()
                .gap(px(8.))
                .child(
                    div()
                        .flex_none()
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(foreground)
                        .text_ellipsis()
                        .child(card.agent.clone()),
                )
                .child(pill_row),
        )
        .child(
            div()
                .w_full()
                .text_size(px(10.))
                .line_height(px(14.))
                .text_color(foreground)
                .line_clamp(2)
                .child(card.task.clone()),
        )
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .flex_none()
                        .text_size(px(10.))
                        .text_color(muted)
                        .child(t!("status_panel_latest").to_string().to_uppercase()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(10.))
                        .text_color(muted)
                        .text_ellipsis()
                        .child(card.result_preview.clone()),
                ),
        )
        .child(
            h_flex()
                .w_full()
                .items_center()
                .flex_wrap()
                .gap(px(8.))
                .child(stat_pill(
                    format!("{} {}", t!("status_panel_tools"), card.tools),
                    background,
                    muted,
                ))
                .child(stat_pill(
                    format!("{} {}", t!("status_panel_ctx"), card.context),
                    background,
                    muted,
                )),
        )
        .into_any_element()
}

fn stat_pill(label: String, background: Hsla, color: Hsla) -> AnyElement {
    h_flex()
        .flex_none()
        .items_center()
        .px(px(8.))
        .py(px(1.))
        .rounded_full()
        .bg(background)
        .text_size(px(10.))
        .line_height(px(14.))
        .text_color(color)
        .child(label)
        .into_any_element()
}

fn efficiency_card(agent: &Value, compact: bool, cx: &App) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let foreground = cx.theme().foreground;
    let background = cx.theme().background;
    let border = cx.theme().border;
    let title = ["title", "userQuery", "sessionId"]
        .into_iter()
        .find_map(|key| agent[key].as_str().filter(|value| !value.is_empty()))
        .unwrap_or("—")
        .to_owned();
    let status = agent["status"].as_str().unwrap_or("").to_lowercase();
    let status_color = match status.as_str() {
        "running" | "thinking" | "pending" => cx.theme().primary,
        "completed" | "success" => Hsla::from(success()),
        "error" | "failed" | "cancelled" => Hsla::from(danger()),
        _ => muted,
    };
    let score =
        |field: &str| agent["metrics"][field].as_f64().unwrap_or_default().round() as i64;
    let score_pill = |label: String, value: i64| {
        let tone = if value >= 80 {
            Hsla::from(success())
        } else if value >= 60 {
            Hsla::from(warning())
        } else {
            Hsla::from(danger())
        };
        h_flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .px(px(8.))
            .py(px(8.))
            .rounded(px(4.))
            .bg(background)
            .border_1()
            .border_color(tone)
            .child(
                div()
                    .flex_none()
                    .text_size(px(10.))
                    .text_color(muted)
                    .child(label.to_uppercase()),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(13.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(foreground)
                    .child(value.to_string()),
            )
            .into_any_element()
    };
    let chip = |label: String, value: i64| {
        v_flex()
            .w_full()
            .gap(px(2.))
            .px(px(8.))
            .py(px(8.))
            .rounded(px(4.))
            .bg(background)
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(muted)
                    .child(label.to_uppercase()),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(foreground)
                    .font_family(MONO_FONT)
                    .child(value.to_string()),
            )
            .into_any_element()
    };
    let mut grid = h_flex().w_full().flex_wrap().gap(px(8.));
    let mut chips = vec![
        (t!("status_panel_search_calls").to_string(), score("searchCalls")),
        (t!("status_panel_read_calls").to_string(), score("readCalls")),
        (t!("status_panel_edit_calls").to_string(), score("editCalls")),
    ];
    if compact {
        chips.push((
            t!("status_panel_repeat_reads").to_string(),
            score("repeatedReadEvents"),
        ));
    } else {
        chips.push((
            t!("status_panel_verification_calls").to_string(),
            score("verificationCalls"),
        ));
        chips.push((
            t!("status_panel_parallel").to_string(),
            score("parallelSearchRounds") + score("parallelReadRounds"),
        ));
        chips.push((
            t!("status_panel_repeat_reads").to_string(),
            score("repeatedReadEvents"),
        ));
    }
    // Vue's metric grid keeps three (or two) equal columns inside the card; the
    // width leaves room for the 8px gap between cells.
    let cell = if compact { 0.47 } else { 0.30 };
    for (label, value) in chips {
        grid = grid.child(
            div()
                .w(relative(cell))
                .min_w_0()
                .child(chip(label, value)),
        );
    }
    let mut card = v_flex()
        .w_full()
        .min_w_0()
        .gap(px(8.))
        .p(px(8.))
        .px(px(12.))
        .rounded(px(4.))
        .bg(cx.theme().muted)
        .border_1()
        .border_color(border)
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .items_center()
                .justify_between()
                .gap(px(8.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(foreground)
                        .line_clamp(if compact { 1 } else { 2 })
                        .child(title),
                )
                .child(
                    div()
                        .flex_none()
                        .px(px(8.))
                        .py(px(1.))
                        .rounded_full()
                        .bg(background)
                        .text_size(px(10.))
                        .text_color(status_color)
                        .child(status.clone()),
                ),
        )
        .child(
            h_flex()
                .w_full()
                .gap(px(8.))
                .child(score_pill(
                    t!("status_panel_convergence").to_string(),
                    score("convergenceScore"),
                ))
                .child(score_pill(
                    t!("status_panel_execution").to_string(),
                    score("executionScore"),
                )),
        );
    if !compact {
        card = card.child(
            div()
                .w_full()
                .text_size(px(10.))
                .line_height(px(15.))
                .text_color(muted)
                .child(t!("status_panel_score_hint").to_string()),
        );
    }
    card = card.child(grid);
    if !compact {
        card = card.child(
            h_flex()
                .w_full()
                .items_center()
                .justify_between()
                .gap(px(8.))
                .text_size(px(11.))
                .text_color(muted)
                .child(t!("status_panel_pre_edit_coverage").to_string())
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!(
                            "{:.0}%",
                            agent["metrics"]["preEditReadCoverage"]
                                .as_f64()
                                .unwrap_or_default()
                        )),
                ),
        );
    }
    card.into_any_element()
}

struct OpRow {
    icon: &'static str,
    class: &'static str,
    label: String,
    status: &'static str,
}

/// Vue reads the panel's recent operations from the persisted tool messages, so
/// finished calls stay listed after the live tool registry drops them.
fn recent_operations(observer: &crate::Observer) -> (Vec<OpRow>, usize) {
    let tools: Vec<&crate::presentation::ToolView> = observer
        .rows
        .iter()
        .filter_map(|row| match row {
            crate::presentation::Row::Tool(tool) => Some(tool.as_ref()),
            _ => None,
        })
        .filter(|tool| !matches!(tool.name.as_str(), "complete_workflow" | "answer_user"))
        .collect();
    let total = tools.len();
    let rows = tools
        .iter()
        .rev()
        .take(RECENT_LIMIT)
        .map(|tool| {
            let status = match tool.status {
                crate::presentation::ToolStatus::Completed => "success",
                crate::presentation::ToolStatus::Failed
                | crate::presentation::ToolStatus::Rejected => "error",
                _ => "running",
            };
            let label = if tool.name == "bash" {
                match tool.command.as_deref().map(str::trim) {
                    Some(command) if !command.is_empty() => truncate(
                        &format!(
                            "Run {}",
                            single_line(&format_display_path(command, &observer.model.roots))
                        ),
                        PREVIEW_CHARS,
                    ),
                    _ => truncate(&tool.action, PREVIEW_CHARS),
                }
            } else if tool.target.is_empty() {
                truncate(&tool.action, PREVIEW_CHARS)
            } else {
                truncate(&format!("{} {}", tool.action, tool.target), PREVIEW_CHARS)
            };
            OpRow {
                icon: tool.icon,
                class: tool_class(&tool.name),
                label,
                status,
            }
        })
        .collect();
    (rows, total)
}

fn workflow_details(observer: &crate::Observer, cx: &mut Context<crate::Observer>) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let foreground = cx.theme().foreground;
    let border = cx.theme().border;
    let background = cx.theme().background;
    let surface = cx.theme().muted;
    let primary = cx.theme().primary;
    let hover = crate::ui::surfaces(observer.appearance_dark).hover;

    let total = observer.model.todo_list.len();
    let completed = observer
        .model
        .todo_list
        .iter()
        .filter(|task| is_done(task))
        .count();
    let percent = completed
        .checked_mul(100)
        .and_then(|value| value.checked_div(total))
        .unwrap_or(0) as u32;
    let (children, child_total) = child_rows(observer);
    let (operations, tool_total) = recent_operations(observer);

    let tabs = [
        "status_panel_main_agent",
        "status_panel_sub_agents",
        "status_panel_models",
        "status_panel_efficiency",
    ];
    let mut bar = h_flex()
        .w_full()
        .min_w_0()
        .flex_none()
        .gap(px(4.))
        .p(px(4.))
        .mb(px(12.))
        .rounded(px(8.))
        .bg(surface)
        .border_1()
        .border_color(border);
    for (index, key) in tabs.into_iter().enumerate() {
        let active = index == observer.status_panel_tab;
        let mut tab = h_flex()
            .id(("status-tab", index))
            .flex_none()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .px(px(8.))
            .py(px(8.))
            .rounded(px(8.))
            .border_1()
            .border_color(if active {
                primary.opacity(0.45)
            } else {
                background.opacity(0.)
            })
            .bg(if active { background } else { background.opacity(0.) })
            .text_size(px(10.))
            .font_weight(FontWeight::SEMIBOLD)
            .whitespace_nowrap()
            .text_color(if active { primary } else { muted })
            .cursor_pointer()
            .child(t!(key).to_string())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.status_panel_tab = index;
                if index == 3 && this.efficiency_report.is_none() {
                    load_efficiency(this, cx);
                }
                cx.notify();
            }));
        if !active {
            tab = tab.hover(move |style| style.bg(hover));
        }
        if index == 1 && !children.is_empty() {
            tab = tab.child(
                div()
                    .flex_none()
                    .min_w(px(16.))
                    .h(px(16.))
                    .px(px(4.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(primary)
                    .text_size(px(10.))
                    .text_color(cx.theme().primary_foreground)
                    .child(children.len().to_string()),
            );
        }
        bar = bar.child(tab);
    }

    let mut body = v_flex().w_full().min_w_0().gap(px(16.)).child(bar);

    match observer.status_panel_tab {
        1 => {
            if children.is_empty() {
                body = body.child(empty_state(
                    "agent",
                    t!("status_panel_no_sub_agents").to_string(),
                    false,
                    cx,
                ));
            } else {
                let meta = if child_total > children.len() {
                    format!("{}/{}", children.len(), child_total)
                } else {
                    children.len().to_string()
                };
                let mut section = v_flex()
                    .w_full()
                    .min_w_0()
                    .gap(px(8.))
                    .child(section_header(
                        "agent",
                        t!("status_panel_child_agents").to_string(),
                        Some(meta),
                        cx,
                    ));
                for child in &children {
                    section = section.child(child_card(child, hover, cx));
                }
                body = body.child(section);
            }
        }
        2 => {
            let rows = model_rows(observer);
            if rows.is_empty() {
                body = body.child(empty_state(
                    "model",
                    t!("status_panel_no_models").to_string(),
                    false,
                    cx,
                ));
            } else {
                let mut section = v_flex().w_full().min_w_0().gap(px(8.)).child(section_header(
                    "model",
                    t!("status_panel_models").to_string(),
                    None,
                    cx,
                ));
                for row in rows {
                    let source_color = if row.source_class == "fallback" {
                        Hsla::from(warning())
                    } else {
                        primary
                    };
                    section = section.child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .items_center()
                            .justify_between()
                            .gap(px(8.))
                            .px(px(8.))
                            .py(px(8.))
                            .mb(px(4.))
                            .rounded(px(4.))
                            .bg(surface)
                            .border_1()
                            .border_color(border)
                            .child(
                                h_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(
                                        div()
                                            .flex_none()
                                            .text_size(px(11.))
                                            .text_color(muted)
                                            .child(row.label.to_uppercase()),
                                    )
                                    .child(
                                        div()
                                            .min_w_0()
                                            .text_size(px(12.))
                                            .text_color(foreground)
                                            .text_ellipsis()
                                            .child(row.name),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .px(px(8.))
                                    .py(px(1.))
                                    .rounded_full()
                                    .bg(background)
                                    .text_size(px(10.))
                                    .text_color(source_color)
                                    .child(row.source),
                            ),
                    );
                }
                body = body.child(section);
            }
        }
        3 => {
            let updated = observer.efficiency_updated_at.map(|updated| {
                t!(
                    "live_elapsed",
                    text = t!("status_panel_efficiency"),
                    seconds = updated.elapsed().as_secs()
                )
                .to_string()
            });
            let refresh_color = Hsla::from(accent());
            let mut refresh_icon = Icon::new("refresh").size(12.).color(refresh_color);
            if observer.efficiency_loading {
                refresh_icon =
                    refresh_icon.spin("status-efficiency-spin", crate::ui::icon::Spin::Linear);
            }
            let refresh = h_flex()
                .id("status-efficiency-refresh")
                .flex_none()
                .items_center()
                .gap(px(4.))
                .px(px(8.))
                .py(px(4.))
                .rounded(px(8.))
                .border_1()
                .border_color(border)
                .bg(surface)
                .text_size(px(10.))
                .text_color(muted)
                .cursor_pointer()
                .hover(move |style| style.bg(hover))
                .child(refresh_icon)
                .child(t!("status_panel_refresh").to_string())
                .on_click(cx.listener(|this, _, _, cx| load_efficiency(this, cx)));
            let header = h_flex()
                .w_full()
                .min_w_0()
                .items_center()
                .gap(px(8.))
                .mb(px(8.))
                .child(Icon::new("skill-piechart").size(14.).color(primary))
                .child(
                    div()
                        .flex_none()
                        .text_size(px(10.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(muted)
                        .child(t!("status_panel_efficiency").to_string().to_uppercase()),
                )
                .child(div().flex_1())
                .children(
                    updated.map(|updated| div().flex_none().text_size(px(10.)).text_color(muted).child(updated)),
                )
                .child(refresh);
            body = body.child(header);
            if observer.efficiency_loading && observer.efficiency_report.is_none() {
                return body
                    .child(empty_state(
                        "loading",
                        t!("status_panel_loading").to_string(),
                        false,
                        cx,
                    ))
                    .into_any_element();
            }
            match &observer.efficiency_report {
                None => {
                    body = body.child(empty_state(
                        "skill-piechart",
                        t!("status_panel_no_efficiency_data").to_string(),
                        false,
                        cx,
                    ));
                }
                Some(Err(error)) => {
                    body = body.child(
                        div()
                            .w_full()
                            .px(px(12.))
                            .py(px(8.))
                            .rounded(px(4.))
                            .bg(Hsla::from(danger()).opacity(0.08))
                            .text_size(px(12.))
                            .text_color(Hsla::from(danger()))
                            .child(error.clone()),
                    );
                }
                Some(Ok(report)) => {
                    let agents: Vec<Value> = std::iter::once(report["mainAgent"].clone())
                        .chain(
                            report["subAgents"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .cloned(),
                        )
                        .filter(|agent| agent.is_object())
                        .collect();
                    let Some((main, sub_agents)) = agents.split_first() else {
                        return body
                            .child(empty_state(
                                "skill-piechart",
                                t!("status_panel_no_efficiency_data").to_string(),
                                false,
                                cx,
                            ))
                            .into_any_element();
                    };
                    let mut main_section = v_flex().w_full().min_w_0().gap(px(8.)).child(
                        section_header(
                            "agent",
                            t!("status_panel_main_agent").to_string(),
                            None,
                            cx,
                        ),
                    );
                    main_section = main_section.child(efficiency_card(main, false, cx));
                    body = body.child(main_section);
                    let mut list = v_flex()
                        .w_full()
                        .min_w_0()
                        .gap(px(8.))
                        .child(section_header(
                            "agent",
                            t!("status_panel_sub_agents").to_string(),
                            Some(sub_agents.len().to_string()),
                            cx,
                        ));
                    if sub_agents.is_empty() {
                        list = list.child(empty_state(
                            "agent",
                            t!("status_panel_no_sub_agents").to_string(),
                            true,
                            cx,
                        ));
                    } else {
                        for agent in sub_agents {
                            list = list.child(efficiency_card(agent, true, cx));
                        }
                    }
                    body = body.child(list);
                }
            }
        }
        _ => {
            if let Some((used, max)) = observer
                .model
                .current_context_tokens
                .zip(observer.model.max_context_tokens)
                .filter(|(_, max)| *max > 0)
            {
                let percent = ((used.min(max) * 100) / max) as u32;
                let mut section = v_flex().w_full().min_w_0().gap(px(8.)).child(section_header(
                    "skill-piechart",
                    t!("status_panel_context_usage").to_string(),
                    None,
                    cx,
                ));
                section = section.child(progress_bar(
                    percent,
                    context_fill(context_class(percent)),
                    cx,
                ));
                section = section.child(
                    div()
                        .w_full()
                        .text_size(px(10.))
                        .text_color(muted)
                        .font_family(MONO_FONT)
                        .text_align(TextAlign::Right)
                        .child(format!("{} / {} tokens", grouped(used), grouped(max))),
                );
                body = body.child(section);
            }
            if total > 0 {
                let mut section = v_flex().w_full().min_w_0().gap(px(8.)).child(
                    section_header(
                        "skill-terminal",
                        t!("status_panel_progress").to_string(),
                        None,
                        cx,
                    ),
                );
                section = section.child(progress_bar(
                    percent,
                    task_fill(progress_class(percent)),
                    cx,
                ));
                body = body.child(section);

                let expanded = observer.expanded.contains("status-panel-all-todos");
                let shown = if expanded { total } else { total.min(TODO_PREVIEW) };
                let mut todos = v_flex()
                    .w_full()
                    .min_w_0()
                    .child(section_header(
                        "list",
                        t!("status_panel_todos").to_string(),
                        None,
                        cx,
                    ));
                for (index, task) in observer.model.todo_list.iter().take(shown).enumerate() {
                    let status = todo_status(task);
                    let (icon, color) = match status {
                        "completed" => ("check", Hsla::from(success())),
                        "in_progress" => ("loading", primary),
                        "failed" => ("error", Hsla::from(danger())),
                        "data_missing" => ("error", Hsla::from(warning())),
                        _ => ("uncheck", muted),
                    };
                    let text_color = match status {
                        "completed" => muted,
                        "in_progress" => primary,
                        "failed" => Hsla::from(danger()),
                        "data_missing" => Hsla::from(warning()),
                        _ => foreground,
                    };
                    let title = task["subject"]
                        .as_str()
                        .or_else(|| task["title"].as_str())
                        .unwrap_or_default()
                        .to_owned();
                    let mut icon_element = Icon::new(icon).size(14.).color(color);
                    if status == "in_progress" {
                        icon_element = icon_element
                            .spin(("status-todo-spin", index), crate::ui::icon::Spin::Linear);
                    }
                    let row = h_flex()
                        .w_full()
                        .min_w_0()
                        .items_center()
                        .gap(px(8.))
                        .py(px(8.))
                        .child(icon_element)
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(12.))
                                .text_color(text_color)
                                .text_ellipsis()
                                .when(status == "completed", |text| text.line_through())
                                .child(title),
                        );
                    todos = todos.child(row);
                }
                if total > TODO_PREVIEW {
                    todos = todos.child(
                        div()
                            .id("status-panel-all-todos")
                            .w_full()
                            .text_align(TextAlign::Center)
                            .pt(px(4.))
                            .text_size(px(12.))
                            .text_color(muted)
                            .italic()
                            .cursor_pointer()
                            .hover(move |style| style.text_color(primary))
                            .child(if expanded {
                                format!("− {}", t!("status_panel_todos"))
                            } else {
                                format!("+{} {}", total - TODO_PREVIEW, t!("status_panel_todos"))
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle("status-panel-all-todos".into(), cx)
                            })),
                    );
                }
                body = body.child(todos);
            }
            if !operations.is_empty() {
                let mut section = v_flex().w_full().min_w_0().gap(px(8.)).child(
                    section_header(
                        "tool",
                        t!("status_panel_recent_ops").to_string(),
                        Some(format!(
                            "{}: {tool_total}",
                            t!("status_panel_total_calls")
                        )),
                        cx,
                    ),
                );
                for (index, operation) in operations.iter().enumerate() {
                    let rail = Hsla::from(tool_class_color(operation.class));
                    let (status_icon, status_color) = match operation.status {
                        "success" => ("check", Hsla::from(success())),
                        "error" => ("error", Hsla::from(danger())),
                        _ => ("loading", primary),
                    };
                    let tinted = match operation.status {
                        "error" => Some(Hsla::from(danger())),
                        "running" => Some(primary),
                        _ => None,
                    };
                    let row = h_flex()
                        .w_full()
                        .min_w_0()
                        .items_center()
                        .justify_between()
                        .gap(px(8.))
                        .px(px(8.))
                        .py(px(8.))
                        .mb(px(4.))
                        .rounded(px(4.))
                        .border_l_2()
                        .border_color(rail)
                        .bg(tinted.map_or(surface, |color| color.opacity(0.08)))
                        .child(
                            h_flex()
                                .flex_1()
                                .min_w_0()
                                .items_center()
                                .gap(px(8.))
                                .child(Icon::new(operation.icon).size(14.).color(rail))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_size(px(12.))
                                        .text_color(foreground)
                                        .font_family(MONO_FONT)
                                        .text_ellipsis()
                                        .child(operation.label.clone()),
                                ),
                        )
                        .child(
                            div()
                                .flex_none()
                                .child(if operation.status == "running" {
                                    Icon::new(status_icon)
                                        .size(12.)
                                        .color(status_color)
                                        .spin(
                                            ("status-op-spin", index),
                                            crate::ui::icon::Spin::Linear,
                                        )
                                        .into_any_element()
                                } else {
                                    Icon::new(status_icon).size(12.).color(status_color).into_any_element()
                                }),
                        );
                    section = section.child(row);
                }
                body = body.child(section);
            }
            if total == 0 && operations.is_empty() {
                body = body.child(empty_state(
                    "file",
                    t!("status_panel_empty").to_string(),
                    false,
                    cx,
                ));
            }
        }
    }

    body.into_any_element()
}

fn load_efficiency(this: &mut crate::Observer, cx: &mut Context<crate::Observer>) {
    if this.efficiency_loading { return; }
    let Some(session) = this.selected.clone() else { return; };
    let generation = this.model.generation;
    let (reply, response) = tokio::sync::oneshot::channel();
    if this.worker.commands.try_send(chatspeed_gpui::types::Command::ReadEfficiency { session, reply }).is_err() {
        this.efficiency_report = Some(Err(t!("status_error").to_string()));
        cx.notify();
        return;
    }
    this.efficiency_loading = true;
    cx.notify();
    cx.spawn(async move |owner, cx| {
        let result = response.await.unwrap_or_else(|_| Err(t!("status_error").to_string()));
        let _ = owner.update(cx, |this, cx| {
            if !this.model.accepts(generation) { return; }
            this.efficiency_loading = false;
            this.efficiency_updated_at = result.is_ok().then(std::time::Instant::now);
            this.efficiency_report = Some(result);
            cx.notify();
        });
    }).detach();
}

fn render_status(live: Option<Live>, cx: &App) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let Some(live) = live else {
        return div().into_any_element();
    };
    let color = if live.warning {
        warning().into()
    } else {
        muted
    };
    let mut icon = Icon::new(live.icon).size(14.).color(color);
    if live.icon == "reasoning" {
        icon = icon.spin("live-reasoning-spin", super::icon::Spin::Linear);
    }
    h_flex()
        .w_full()
        .min_w_0()
        .min_h(px(24.))
        .items_center()
        .gap_2()
        .px_1()
        .text_size(px(12.))
        .text_color(color)
        .child(icon)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(super::chrome_label(live.text)),
        )
        .into_any_element()
}

fn live_state(model: &Model, running: bool, answered: bool) -> Option<Live> {
    let state = model.state.to_lowercase();
    let wait = model.wait_reason.to_lowercase();
    let waiting_user = !answered && (wait == "user_input" || state == "awaiting_user");
    let waiting_approval =
        wait == "approval" || state == "awaiting_approval" || state == "awaiting_auto_approval";
    let terminal = matches!(
        state.as_str(),
        "completed" | "failed" | "cancelled" | "error" | "stopped"
    );
    // Vue shows nothing for a finished task, and nothing before the first run
    // either; the connecting note is the only non-runtime message it keeps.
    if terminal || (!running && !waiting_user && !waiting_approval) {
        return None;
    }

    if pending_plan(model) {
        return Some(Live {
            text: t!("live_awaiting_plan_approval").to_string(),
            icon: "approval",
            warning: false,
        });
    }

    if waiting_user {
        return Some(Live {
            text: t!("live_awaiting_user").to_string(),
            icon: "ask_user",
            warning: true,
        });
    }

    let latest = model
        .tool_order
        .last()
        .and_then(|id| model.tools.get(id).map(|tool| (id, tool)));
    // An answered `ask_user` keeps its pending row until the runtime resumes;
    // Vue already moved on, so the line must not fall back to "awaiting".
    let answered_ask = answered && latest.is_some_and(|(_, tool)| tool_name(tool) == "ask_user");

    if let Some((id, tool)) = latest {
        let phase = tool_phase(tool);
        if !answered_ask && matches!(phase, "pending" | "running") {
            return Some(tool_status(model, id, tool, phase));
        }
    }

    if waiting_approval && !answered_ask {
        return Some(Live {
            text: t!("live_awaiting_approval").to_string(),
            icon: "approval",
            warning: false,
        });
    }

    if let Some(text) = latest_text(model) {
        return Some(Live {
            text,
            icon: "reasoning",
            warning: false,
        });
    }

    // A tool that just finished stays on screen briefly, like Vue's 5s window.
    if let Some((id, tool)) = latest {
        let fresh = model
            .tool_finished
            .get(id)
            .is_some_and(|at| at.elapsed() < COMPLETED_VISIBLE);
        if fresh && !answered_ask && tool_phase(tool) == "done" {
            return Some(tool_status(model, id, tool, "done"));
        }
    }

    if running {
        return Some(Live {
            text: t!("live_thinking").to_string(),
            icon: "reasoning",
            warning: false,
        });
    }

    None
}

/// The runtime waits on a plan confirmation while `submit_plan` is pending.
fn pending_plan(model: &Model) -> bool {
    model.pending_tool_ids.iter().any(|id| {
        model
            .tools
            .get(id)
            .is_some_and(|tool| tool_name(tool) == "submit_plan")
    })
}

/// The tool text for one call, matching Vue's `buildToolState` branches.
fn tool_status(model: &Model, id: &str, tool: &Value, phase: &str) -> Live {
    let name = tool_name(tool).to_lowercase();
    let args = tool_arguments(tool);
    let path = args["file_path"]
        .as_str()
        .or_else(|| args["path"].as_str())
        .map(|path| format_display_path(path, &model.roots))
        .unwrap_or_default();
    let command = args["command"]
        .as_str()
        .map(|command| {
            let normalized = single_line(command);
            truncate(&format!("Run {normalized}"), PREVIEW_CHARS)
        })
        .unwrap_or_default();

    let (text, icon) = match (phase, name.as_str()) {
        ("pending", "edit_file") if !path.is_empty() => (
            t!("live_awaiting_edit_approval", path = path).to_string(),
            "edit",
        ),
        ("pending", "write_file") if !path.is_empty() => (
            t!("live_awaiting_write_approval", path = path).to_string(),
            "write_file",
        ),
        ("pending", "bash") => (
            t!(
                "live_awaiting_bash_approval",
                command = command_or_label(&command, tool)
            )
            .to_string(),
            "bash",
        ),
        ("pending", _) if !APPROVAL_TOOLS.contains(&name.as_str()) => {
            (t!("live_awaiting_approval").to_string(), "warning")
        }
        ("running", "edit_file") if !path.is_empty() => {
            (t!("live_running_edit", path = path).to_string(), "edit")
        }
        ("running", "write_file") if !path.is_empty() => (
            t!("live_running_write", path = path).to_string(),
            "write_file",
        ),
        ("running", "bash") => (
            t!(
                "live_running_command",
                command = command_or_label(&command, tool)
            )
            .to_string(),
            "bash",
        ),
        ("running", _) => (
            t!("live_running_tool", tool = tool_label(tool)).to_string(),
            "tool",
        ),
        ("done", "edit_file") if !path.is_empty() => (
            t!("live_file_edited", path = path).to_string(),
            "check-circle",
        ),
        ("done", "write_file") if !path.is_empty() => (
            t!("live_file_created", path = path).to_string(),
            "check-circle",
        ),
        ("done", "bash") => (
            t!(
                "live_tool_completed",
                tool = command_or_label(&command, tool)
            )
            .to_string(),
            "check-circle",
        ),
        _ => (
            t!("live_tool_completed", tool = tool_label(tool)).to_string(),
            "check-circle",
        ),
    };

    let text = if phase == "running" {
        model
            .tool_started
            .get(id)
            .map(|started| {
                t!(
                    "live_elapsed",
                    text = text,
                    seconds = started.elapsed().as_secs()
                )
                .to_string()
            })
            .unwrap_or(text)
    } else {
        text
    };

    Live {
        text,
        icon: if phase == "pending" { "approval" } else { icon },
        warning: false,
    }
}

/// `execution_status` and the live event type both describe the same lifecycle.
fn tool_phase(tool: &Value) -> &'static str {
    let event = tool["type"].as_str().unwrap_or("");
    match event {
        "tool_started" => return "running",
        "tool_completed" => return "done",
        "tool_failed" => return "failed",
        _ => {}
    }
    match tool["execution_status"]
        .as_str()
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "pending_approval" => "pending",
        "running" | "approval_submitted" => "running",
        "completed" => "done",
        "failed" | "rejected" => "failed",
        _ => "running",
    }
}

fn command_or_label(command: &str, tool: &Value) -> String {
    if command.is_empty() {
        tool_label(tool)
    } else {
        command.to_owned()
    }
}

fn tool_label(tool: &Value) -> String {
    let name = tool_name(tool);
    let args = tool_arguments(tool);
    let detail = args["description"]
        .as_str()
        .or_else(|| args["query"].as_str())
        .or_else(|| args["pattern"].as_str())
        .or_else(|| args["path"].as_str())
        .unwrap_or("");
    if detail.is_empty() {
        name.to_owned()
    } else {
        truncate(&format!("{name} {detail}"), PREVIEW_CHARS)
    }
}

/// The newest streamed sentence, which Vue prefers over the thinking fallback.
fn latest_text(model: &Model) -> Option<String> {
    let live = &model.live;
    let content = preview_segment(&live.message);
    let reasoning = preview_segment(live.reasoning.as_deref().unwrap_or(""));
    let text = if content.is_empty() {
        reasoning
    } else {
        content
    };
    (!text.is_empty()).then_some(text)
}

/// Vue's `getPreviewSegment`: the last finished sentence, else the last segment.
fn preview_segment(text: &str) -> String {
    let cleaned = strip_reminders(text);
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        return String::new();
    }
    let latest = cleaned
        .rsplit(['。', '！', '？', '.', '!', '?'])
        .find(|segment| !segment.trim().is_empty())
        .unwrap_or(cleaned)
        .trim();
    truncate(latest, PREVIEW_CHARS)
}

/// Removes the reminder and thinking wrappers the streamed text still carries.
fn strip_reminders(text: &str) -> String {
    let mut cleaned = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<SYSTEM_REMINDER>") {
        cleaned.push_str(&rest[..start]);
        match rest[start..].find("</SYSTEM_REMINDER>") {
            Some(end) => rest = &rest[start + end + "</SYSTEM_REMINDER>".len()..],
            None => {
                rest = "";
                break;
            }
        }
    }
    cleaned.push_str(rest);
    let mut collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    for tag in ["<think>", "</think>", "<thinking>", "</thinking>"] {
        collapsed = collapsed.replace(tag, "");
    }
    collapsed.trim().to_owned()
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let head: String = text.chars().take(max.saturating_sub(3)).collect();
    format!("{head}...")
}

#[cfg(test)]
mod tests {
    use super::{live_state, preview_segment, tool_phase, truncate, Model};

    #[test]
    fn local_answer_hides_stale_waiting_status() {
        let mut model = Model::default();
        model.state = "awaiting_user".into();
        let waiting = live_state(&model, false, false).expect("unanswered prompt is visible");
        assert_eq!(waiting.icon, "ask_user");
        assert!(waiting.warning);
        assert!(live_state(&model, false, true).is_none());
    }

    #[test]
    fn live_status_strings_interpolate_their_placeholders() {
        for text in [
            rust_i18n::t!("live_elapsed", text = "Run cargo test", seconds = 4).to_string(),
            rust_i18n::t!("live_running_tool", tool = "grep").to_string(),
            rust_i18n::t!("live_awaiting_bash_approval", command = "ls").to_string(),
        ] {
            assert!(!text.contains("%{"), "unresolved placeholder in {text}");
            assert!(!text.contains("{{"), "vue-style placeholder in {text}");
        }
    }

    #[test]
    fn preview_keeps_the_latest_sentence_and_drops_reminders() {
        let text = "<SYSTEM_REMINDER>noise</SYSTEM_REMINDER> First done. Second part.";
        assert_eq!(preview_segment(text), "Second part");
    }

    #[test]
    fn truncation_marks_what_it_cut() {
        assert_eq!(truncate("abcdef", 6), "abcdef");
        assert_eq!(truncate("abcdefgh", 6), "abc...");
        assert_eq!(truncate("缓存命中率很高", 5), "缓存...");
    }

    #[test]
    fn tool_phases_follow_the_live_event_first() {
        assert_eq!(
            tool_phase(&serde_json::json!({"type": "tool_completed"})),
            "done"
        );
        assert_eq!(
            tool_phase(&serde_json::json!({"execution_status": "pending_approval"})),
            "pending"
        );
        assert_eq!(
            tool_phase(&serde_json::json!({"execution_status": "completed"})),
            "done"
        );
    }
}
