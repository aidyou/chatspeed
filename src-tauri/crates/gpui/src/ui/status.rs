//! Live status above the prompt box.
//!
//! Mirrors `src/components/workflow/StatusNotifier.vue`: the line reports what
//! the runtime is doing *now* — the tool it is waiting on or running, the latest
//! streamed sentence, or the plain thinking state — instead of a static list of
//! states. The order of the checks is the same as Vue's `displayState`, because
//! a pending approval must win over streamed text and vice versa.

use std::time::Duration;

use gpui_kit::component::{h_flex, ActiveTheme};
use gpui_kit::*;
use rust_i18n::t;
use serde_json::Value;

use crate::presentation::{format_display_path, single_line, tool_arguments, tool_name};
use crate::ui::Icon;
use crate::{warning, Model};

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

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PanelMode { Circle, Compact, Expanded }

pub(crate) fn overlay(observer: &crate::Observer, window: &Window, cx: &mut Context<crate::Observer>) -> Option<AnyElement> {
    observer.selected.as_ref()?;
    let mode = observer.status_panel_mode;
    let viewport = window.viewport_size();
    let width = match mode { PanelMode::Circle => px(40.), PanelMode::Compact => px(155.), PanelMode::Expanded => px(350.) }.min(viewport.width);
    let height = if mode == PanelMode::Expanded { px(380.).min((viewport.height - px(52.)).max(px(40.))) } else { px(40.) };
    let position = observer.status_panel_position.unwrap_or(point(viewport.width - px(370.), px(56.)));
    let origin = point(position.x.max(px(0.)).min((viewport.width - width).max(px(0.))), position.y.max(px(36.)).min((viewport.height - height).max(px(36.))));
    let theme = cx.theme();
    let total = observer.model.todo_list.len();
    let completed = observer.model.todo_list.iter().filter(|task| task["status"] == "completed").count();
    let progress = completed.checked_mul(100).and_then(|n| n.checked_div(total));
    let title = match mode {
        PanelMode::Circle => progress.map(|value| format!("{value}%")).unwrap_or_default(),
        PanelMode::Compact => progress.map(|value| format!("{value}%")).unwrap_or_else(|| t!("snapshot_task_state").to_string()),
        PanelMode::Expanded => format!("{}  {completed}/{total}", t!("snapshot_task_state")),
    };
    let header = h_flex().id("status-panel-header").h(px(40.)).px_2().gap_2().cursor_pointer()
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, _, _| {
            this.status_panel_drag = Some(event.position - origin);
        }))
        .child(if mode == PanelMode::Circle && progress.is_some() {
            div().text_xs().child(title).into_any_element()
        } else {
            h_flex().gap_2().child(Icon::new("list").size(14.).color(theme.muted_foreground)).child(title).into_any_element()
        })
        .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
            if let ClickEvent::Mouse(event) = event {
                let delta = event.up.position - event.down.position;
                if delta.x.abs() > px(4.) || delta.y.abs() > px(4.) { return; }
            }
            this.status_panel_mode = match this.status_panel_mode {
                PanelMode::Circle => PanelMode::Expanded,
                PanelMode::Compact => PanelMode::Expanded,
                PanelMode::Expanded => PanelMode::Compact,
            };
            cx.notify();
        }));
    let mut panel = div().id("status-panel-overlay").occlude().absolute().left(origin.x).top(origin.y).w(width).h(height)
        .bg(theme.background).border_1().border_color(theme.border).shadow_md().overflow_hidden()
        .rounded(px(if mode == PanelMode::Circle { 20. } else { 8. }));
    let context = observer.model.current_context_tokens.zip(observer.model.max_context_tokens)
        .filter(|(_, max)| *max > 0).map(|(used, max)| (used as f32 / max as f32).min(1.));
    if mode == PanelMode::Circle {
        panel = panel.child(header);
        if let Some(fraction) = context {
            for segment in 0..32 {
                let angle = segment as f32 * std::f32::consts::TAU / 32.;
                panel = panel.child(div().absolute().left(px(18. + 17. * angle.sin()))
                    .top(px(18. - 17. * angle.cos())).size(px(2.)).rounded_full()
                    .bg(if (segment as f32) < fraction * 32. { theme.primary } else { theme.muted }));
            }
        }
    } else {
        if let Some(fraction) = context.filter(|_| mode == PanelMode::Compact) {
            panel = panel.child(div().absolute().bottom_0().left_0().right_0().h(px(3.))
                .bg(theme.muted).child(div().w(relative(fraction)).h_full().bg(theme.primary)));
        }
        panel = panel.child(h_flex().justify_between().child(header).child(h_flex().gap_2()
            .child(div().id("status-panel-resize").cursor_pointer()
                .child(Icon::new(if mode == PanelMode::Compact { "fullscreen-off" } else { "minimize" }).size(14.).color(theme.muted_foreground))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.status_panel_mode = if mode == PanelMode::Compact { PanelMode::Expanded } else { PanelMode::Compact };
                    cx.notify();
                })))
            .child(div().id("status-panel-circle").px_2().cursor_pointer()
                .child(Icon::new("close").size(12.).color(theme.muted_foreground))
                .on_click(cx.listener(|this, _, _, cx| { this.status_panel_mode = PanelMode::Circle; cx.notify(); })))));
        if mode == PanelMode::Expanded {
            panel = panel.child(div().id("status-panel-scroll").h(height - px(40.)).overflow_y_scroll().p_3().child(workflow_details(observer, cx)));
        }
    }
    Some(panel.into_any_element())
}

fn workflow_details(observer: &crate::Observer, cx: &mut Context<crate::Observer>) -> AnyElement {
    use gpui_kit::component::v_flex;
    let model = &observer.model;
    let mut panel = v_flex().w_full().text_xs().gap_3();
    let tabs = ["status_main_agent", "status_sub_agents", "status_models", "status_efficiency"];
    let mut bar = h_flex().gap_2();
    for (index, key) in tabs.into_iter().enumerate() {
        bar = bar.child(div().id(("status-tab", index)).cursor_pointer().px_1().py_1()
            .text_color(if index == observer.status_panel_tab { cx.theme().primary } else { cx.theme().muted_foreground })
            .child(t!(key).to_string()).on_click(cx.listener(move |this, _, _, cx| {
                this.status_panel_tab = index;
                if index == 3 && this.efficiency_report.is_none() {
                    load_efficiency(this, cx);
                }
                cx.notify();
            })));
    }
    panel = panel.child(bar);
    if observer.status_panel_tab == 1 {
        let mut children = model.sub_agent_progress.clone();
        for row in &observer.rows {
            if let crate::presentation::Row::SubAgent(card) = row {
                if let Some(id) = &card.task_id {
                    let entry = children.entry(id.clone()).or_insert_with(|| serde_json::json!({}));
                    for (field, text) in [("task", &card.task), ("agent_name", &card.agent), ("workflow_state", &card.status_label), ("summary", &card.result_preview), ("tools_label", &card.tools), ("context_label", &card.context)] {
                        if entry[field].is_null() && !text.is_empty() { entry[field] = serde_json::json!(text); }
                    }
                }
            }
        }
        if children.is_empty() {
            return panel.child("—").into_any_element();
        }
        panel = panel.child(format!("{}: {}", t!("status_sub_agents"), children.len()));
        let mut children: Vec<_> = children.iter().collect();
        children.sort_by_key(|(_, value)| std::cmp::Reverse(value["updated_at_ms"].as_u64().unwrap_or_default()));
        for (id, value) in children.into_iter().take(5) {
            let name = value["agent_name"].as_str().unwrap_or(id);
            let state = value["workflow_state"].as_str().unwrap_or("—");
            let task = value["task"].as_str().unwrap_or_default();
            let child_id = id.clone();
            let child_title = name.to_owned();
            panel = panel.child(v_flex().id(gpui_kit::SharedString::from(format!("status-child-{id}"))).cursor_pointer().gap_1().p_2().rounded(px(6.)).bg(cx.theme().muted)
                .on_click(cx.listener(move |this, _, _, cx| {
                    let durable_index = this.rows.iter().rposition(|row| matches!(row, crate::presentation::Row::SubAgent(card) if card.task_id.as_ref() == Some(&child_id)));
                    let index = durable_index.or_else(|| {
                        this.model.sub_agent_progress.iter().filter(|(id, progress)| {
                            !this.rows.iter().any(|row| matches!(row, crate::presentation::Row::SubAgent(card) if card.task_id.as_ref() == Some(*id)))
                                && crate::presentation::live_child_card(id, progress).is_some()
                        }).position(|(id, _)| id == &child_id).map(|index| this.rows.len() + index)
                    });
                    if let Some(index) = index {
                        this.timeline_scroll.follow = false;
                        this.timeline_scroll.scroll_pending = false;
                        this.timeline_scroll.scroll.scroll_to_item(index + usize::from(this.model.hidden > 0) + usize::from(this.switched.is_some()));
                        cx.notify();
                    } else {
                        this.open_child_session(child_id.clone(), child_title.clone(), cx);
                    }
                }))
                .child(format!("{name} · {state}"))
                .child(single_line(task))
                .children(value["summary"].as_str().filter(|summary| !summary.is_empty()).map(single_line))
                .children(value["wait_reason"].as_str().filter(|reason| *reason != "none").map(single_line))
                .child(value["tools_label"].as_str().filter(|_| value["tool_calls_count"].is_null())
                    .map(str::to_owned).unwrap_or_else(|| format!("{}: {}", t!("sub_agent_tools"), value["tool_calls_count"].as_u64().map(|n| n.to_string()).unwrap_or_else(|| "—".into()))))
                .children(context_bar(value["current_context_tokens"].as_u64(), value["max_context_tokens"].as_u64(), cx)
                    .or_else(|| value["context_label"].as_str().map(|text| div().child(text.to_owned()).into_any_element()))));
        }
        return panel.into_any_element();
    }
    if observer.status_panel_tab == 2 {
        let configs = observer.model_configs();
        let work = observer.active_model_config();
        let global = &observer.status_title_model;
        let candidates = [
            (configs.get("lite").and_then(|v| v["model"].as_str()), t!("model_slot_lite")),
            (global["id"].as_u64().filter(|id| *id > 0).and_then(|_| global["model"].as_str()), t!("status_global_model")),
            (configs.get("utility").and_then(|v| v["model"].as_str()), t!("model_slot_utility")),
            (work["model"].as_str(), t!("model_slot_act")),
        ];
        let title = candidates.into_iter().find_map(|(name, source)| name.filter(|name| !name.is_empty()).map(|name| format!("{name} · {source}")));
        panel = panel.child(h_flex().gap_2().justify_between().child(t!("status_title_model").to_string()).child(title.unwrap_or_else(|| "—".into())));
        for (slot, key) in [("act", "model_slot_act"), ("plan", "model_slot_plan"), ("utility", "model_slot_utility"), ("vision", "model_slot_vision"), ("lite", "model_slot_lite")] {
            let configured = configs.get(slot).and_then(|v| v["model"].as_str()).filter(|name| !name.is_empty());
            let active = observer.active_model_config();
            let name = match slot {
                "act" => active["model"].as_str().filter(|name| !name.is_empty()).or(configured),
                "utility" => configured.or_else(|| active["model"].as_str().filter(|name| !name.is_empty())),
                _ => configured,
            }.unwrap_or("—");
            panel = panel.child(h_flex().gap_2().justify_between().child(t!(key).to_string()).child(name.to_owned()));
        }
        return panel.into_any_element();
    }
    if observer.status_panel_tab == 3 {
        panel = panel.child(div().id("workflow-efficiency").cursor_pointer()
            .child(if observer.efficiency_loading { "…".to_owned() } else { t!("status_efficiency").to_string() })
            .on_click(cx.listener(|this, _, _, cx| load_efficiency(this, cx))));
        if let Some(updated) = observer.efficiency_updated_at {
            panel = panel.child(t!("live_elapsed", text = t!("status_efficiency"), seconds = updated.elapsed().as_secs()).to_string());
        }
        if let Some(report) = &observer.efficiency_report {
            match report {
                Ok(report) => {
                    for agent in std::iter::once(&report["mainAgent"]).chain(report["subAgents"].as_array().into_iter().flatten()).filter(|agent| agent.is_object()) {
                        let title = ["title", "userQuery", "sessionId"].into_iter().find_map(|key| agent[key].as_str().filter(|s| !s.is_empty())).unwrap_or("—");
                        let mut card = v_flex().gap_1().p_2().rounded(px(6.)).bg(cx.theme().muted).child(title.to_owned());
                        if let Some(state) = agent["status"].as_str() { card = card.child(state.to_owned()); }
                        for (field, label) in [("convergenceScore", "status_convergence"), ("executionScore", "status_execution"), ("searchCalls", "status_search_calls"), ("readCalls", "status_read_calls"), ("editCalls", "status_edit_calls"), ("verificationCalls", "status_verification_calls"), ("repeatedReadEvents", "status_repeated_reads"), ("preEditReadCoverage", "status_read_coverage")] {
                            if let Some(number) = agent["metrics"][field].as_f64() {
                                card = card.child(h_flex().justify_between().gap_2().child(t!(label).to_string()).child(number.to_string()));
                            }
                        }
                        let metrics = &agent["metrics"];
                        let rounds = ["parallelSearchRounds", "parallelReadRounds"].into_iter().filter_map(|key| metrics[key].as_u64()).reduce(u64::saturating_add);
                        if let Some(rounds) = rounds {
                            card = card.child(h_flex().justify_between().gap_2().child(t!("status_parallel_rounds").to_string()).child(rounds.to_string()));
                        }
                        panel = panel.child(card);
                    }
                }
                Err(error) => panel = panel.child(error.clone()),
            }
        }
        return panel.into_any_element();
    }
    if observer.status_panel_tab == 0 {
        panel = panel.children(context_bar(model.current_context_tokens, model.max_context_tokens, cx));
        if !model.todo_list.is_empty() {
            let complete = model.todo_list.iter().filter(|item| item["status"] == "completed").count();
            let fraction = complete as f32 / model.todo_list.len() as f32;
            panel = panel.child(format!("{}: {complete}/{}", t!("todo_items"), model.todo_list.len()))
                .child(div().w_full().h(px(5.)).rounded(px(3.)).bg(cx.theme().muted)
                    .child(div().w(relative(fraction)).h_full().rounded(px(3.)).bg(cx.theme().primary)));
            let expanded = observer.expanded.contains("status-panel-all-todos");
            for item in model.todo_list.iter().take(if expanded { usize::MAX } else { 10 }) {
                if let Some(title) = item["subject"].as_str().or_else(|| item["title"].as_str()) {
                    panel = panel.child(div().text_ellipsis().child(title.to_owned()));
                }
            }
            if model.todo_list.len() > 10 {
                panel = panel.child(div().id("status-panel-all-todos").cursor_pointer()
                    .child(if expanded { "−".to_owned() } else { format!("+{}", model.todo_list.len() - 10) })
                    .on_click(cx.listener(|this, _, _, cx| this.toggle("status-panel-all-todos".into(), cx))));
            }
        }
        let mut recent = model.tool_order.iter().rev()
            .filter_map(|id| model.tools.get(id))
            .filter(|tool| {
                let name = tool_name(tool);
                !name.is_empty() && !matches!(name, "complete_workflow" | "answer_user")
            });
        let displayed: Vec<_> = recent.by_ref().take(5).collect();
        if !displayed.is_empty() {
            let total = displayed.len() + recent.count();
            panel = panel.child(format!("{} · {}: {total}", t!("snapshot_recent_actions"), t!("sub_agent_tools")));
            for tool in displayed {
                let (icon, color) = match tool_phase(tool) {
                    "done" => ("check", cx.theme().primary),
                    "failed" => ("error", cx.theme().danger),
                    _ => ("loading", cx.theme().muted_foreground),
                };
                panel = panel.child(h_flex().justify_between().gap_2()
                    .child(div().text_ellipsis().child(tool_name(tool).to_owned()))
                    .child(Icon::new(icon).size(12.).color(color)));
            }
        }
    }
    panel.into_any_element()
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

fn context_bar(used: Option<u64>, max: Option<u64>, cx: &App) -> Option<AnyElement> {
    let (used, max) = used.zip(max.filter(|max| *max > 0))?;
    let fraction = (used as f32 / max as f32).min(1.);
    Some(h_flex().gap_2().items_center()
        .child(div().flex_1().h(px(5.)).rounded(px(3.)).bg(cx.theme().muted)
            .child(div().w(relative(fraction)).h_full().rounded(px(3.)).bg(cx.theme().primary)))
        .child(format!("{}: {:.0}%", t!("sub_agent_context"), fraction * 100.)).into_any_element())
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
