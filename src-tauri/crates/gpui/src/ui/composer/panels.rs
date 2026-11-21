//! Configuration dialogs opened from the composer's `+` menu.
//!
//! Vue implements the model, skills, MCP and tool configuration as popovers or
//! dialogs anchored in the web page. The native window draws them as one modal
//! over the workflow frame, which keeps a long option list scrollable and lets
//! every switch repaint from the same live state the composer reads.

use gpui_kit::component::{h_flex, v_flex, ActiveTheme};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use rust_i18n::t;
use serde_json::{json, Value};

use super::footer::{apply_model_choice, model_choices};
use crate::ui::Icon;
use crate::{Observer, QuickPanel, ToolDraft};
use chatspeed_gpui::types::Command;

const MODEL_SLOTS: [&str; 5] = ["act", "plan", "vision", "utility", "lite"];

/// The centered modal for the active configuration panel.
pub fn overlay(observer: &Observer, cx: &mut Context<Observer>) -> AnyElement {
    let Some(panel) = observer.quick_panel else {
        return div().into_any_element();
    };
    let (title, icon) = match panel {
        QuickPanel::Models => (t!("model_config").to_string(), "model"),
        QuickPanel::Skills => (t!("skills_config").to_string(), "skill"),
        QuickPanel::Mcp => (t!("mcp_config").to_string(), "mcp"),
        QuickPanel::Tools => (t!("tool_config").to_string(), "setting"),
        QuickPanel::Personality => (t!("execution_style").to_string(), "skill-mindmap-circle"),
    };
    let muted = cx.theme().muted_foreground;
    let foreground = cx.theme().foreground;
    let border = cx.theme().border;
    let surface = cx.theme().background;
    let body = match panel {
        QuickPanel::Models => model_panel(observer, cx),
        QuickPanel::Skills => skills_panel(observer, cx),
        QuickPanel::Mcp => tool_panel(observer, true, cx),
        QuickPanel::Tools => tool_panel(observer, false, cx),
        QuickPanel::Personality => personality_panel(observer, cx),
    };
    div()
        .absolute()
        .top(px(0.))
        .left(px(0.))
        .right(px(0.))
        .bottom(px(0.))
        .occlude()
        .flex()
        .items_center()
        .justify_center()
        .bg(black().opacity(0.32))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| this.close_quick_panel(cx)),
        )
        .child(
            v_flex()
                .id("composer-config-panel")
                .w(px(600.))
                .max_h(px(560.))
                .min_h_0()
                .rounded(px(12.))
                .border_1()
                .border_color(border)
                .bg(surface)
                .shadow(vec![BoxShadow {
                    offset: point(px(0.), px(18.)),
                    blur_radius: px(48.),
                    spread_radius: px(0.),
                    inset: false,
                    color: foreground.opacity(0.16),
                }])
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .gap_2()
                        .px_4()
                        .py_3()
                        .border_b_1()
                        .border_color(border)
                        .child(Icon::new(icon).size(15.).color(cx.theme().primary))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(14.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(foreground)
                                .child(title),
                        )
                        .child(
                            div()
                                .id("composer-config-close")
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .size(px(22.))
                                .rounded(px(6.))
                                .cursor_pointer()
                                .hover(|style| style.bg(cx.theme().muted))
                                .child(Icon::new("close").size(13.).color(muted))
                                .on_click(cx.listener(|this, _, _, cx| this.close_quick_panel(cx))),
                        ),
                )
                .child(body),
        )
        .into_any_element()
}

fn empty_note(text: String, cx: &App) -> AnyElement {
    div()
        .w_full()
        .py_6()
        .text_size(px(12.))
        .text_color(cx.theme().muted_foreground)
        .child(text)
        .into_any_element()
}

/// A check row used for the read-only lists and the multi-select panels.
fn check_row(
    id: String,
    label: String,
    detail: String,
    checked: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let primary = cx.theme().primary;
    let muted = cx.theme().muted_foreground;
    v_flex()
        .id(SharedString::from(id))
        .w_full()
        .min_w_0()
        .gap(px(1.))
        .px_2()
        .py(px(4.))
        .rounded(px(6.))
        .cursor_pointer()
        .hover(|style| style.bg(cx.theme().muted))
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .items_center()
                .gap_2()
                .child(
                    Icon::new(if checked { "check-circle" } else { "uncheck" })
                        .size(14.)
                        .color(if checked { primary } else { muted }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_size(px(12.))
                        .text_color(cx.theme().foreground)
                        .child(label),
                ),
        )
        .when(!detail.is_empty(), |row| {
            row.child(
                div()
                    .w_full()
                    .pl(px(22.))
                    .text_size(px(11.))
                    .text_color(muted)
                    .child(detail),
            )
        })
        .on_click(on_click)
        .into_any_element()
}

/// Switches between the model slots on the left and the models on the right.
fn model_panel(observer: &Observer, cx: &mut Context<Observer>) -> AnyElement {
    let generation = observer.model.generation;
    let configs = observer.model_configs();
    let slot = observer.model_config_slot.clone();
    let slot_config = configs[slot.as_str()].clone();
    let (_group, choices) = model_choices(observer, &slot_config);
    let current = slot_config["model"].as_str().unwrap_or_default().to_owned();
    let border = cx.theme().border;
    let primary = cx.theme().primary;
    let mut slots = v_flex().w(px(180.)).flex_none().gap(px(2.)).pr_3();
    for value in MODEL_SLOTS {
        let label = match value {
            "act" => t!("model_slot_act").to_string(),
            "plan" => t!("model_slot_plan").to_string(),
            "vision" => t!("model_slot_vision").to_string(),
            "utility" => t!("model_slot_utility").to_string(),
            _ => t!("model_slot_lite").to_string(),
        };
        let selected = slot == value;
        let target = value.to_owned();
        slots = slots.child(
            h_flex()
                .id(SharedString::from(format!("model-slot-{value}")))
                .w_full()
                .items_center()
                .gap_2()
                .px_2()
                .py_1()
                .rounded(px(6.))
                .cursor_pointer()
                .when(selected, |row| row.bg(primary.opacity(0.12)))
                .hover(|style| style.bg(cx.theme().muted))
                .text_size(px(12.))
                .text_color(if selected {
                    primary
                } else {
                    cx.theme().foreground
                })
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.model_config_slot = target.clone();
                    cx.notify();
                })),
        );
    }
    let mut models = v_flex()
        .id("model-config-choices")
        .flex_1()
        .min_w_0()
        .max_h(px(380.))
        .overflow_y_scroll();
    let all_configs = configs.clone();
    let base = slot_config.clone();
    let has_choices = !choices.is_empty();
    for choice in choices {
        let selected = current == choice.model;
        let next = apply_model_choice(&base, &choice);
        let mut configs = all_configs.clone();
        configs[slot.as_str()] = next;
        models = models.child(check_row(
            format!("model-choice-{}", choice.model),
            choice.name,
            String::new(),
            selected,
            cx.listener(move |this, _, _, cx| {
                if this.model.accepts(generation) && this.models_pending.is_none() {
                    this.save_models(configs.clone(), cx);
                    this.close_quick_panel(cx);
                }
            }),
            cx,
        ));
    }
    h_flex()
        .w_full()
        .p_3()
        .gap_3()
        .border_t_1()
        .border_color(border)
        .child(slots)
        .child(div().w(px(1.)).self_stretch().bg(border))
        .child(models)
        .when(!has_choices, |row| {
            row.child(empty_note(t!("model_placeholder").to_string(), cx))
        })
        .into_any_element()
}

fn skills_panel(observer: &Observer, cx: &mut Context<Observer>) -> AnyElement {
    let Some(draft) = observer.skills_draft.clone() else {
        return empty_note(t!("skills_empty").to_string(), cx);
    };
    let skills: Vec<(String, String)> = observer
        .skills
        .iter()
        .filter_map(|skill| {
            Some((
                skill["name"].as_str()?.to_owned(),
                skill["description"].as_str().unwrap_or("").to_owned(),
            ))
        })
        .collect();
    let enabled = draft.enabled;
    let selected = draft.selected.clone();
    let toggle_enabled = cx.listener(move |this, _, _, cx| {
        let Some(session) = this.selected.clone() else {
            return;
        };
        let enabled = !enabled;
        let selected = this
            .skills_draft
            .as_ref()
            .map(|draft| draft.selected.clone())
            .unwrap_or_default();
        if this.submit_config(&session, "skills", Command::SetSkills {
            session: session.clone(),
            enabled,
            selected: selected.into_iter().collect(),
        }, cx) {
            if let Some(draft) = this.skills_draft.as_mut() {
                draft.enabled = enabled;
            }
        }
    });
    let mut list = v_flex().w_full().min_w_0().gap(px(1.)).p_3();
    list = list.child(check_row(
        "skills-enabled".into(),
        t!("skill_enabled").to_string(),
        String::new(),
        enabled,
        toggle_enabled,
        cx,
    ));
    if skills.is_empty() {
        list = list.child(empty_note(t!("skills_empty").to_string(), cx));
    }
    for (name, description) in skills {
        let checked = selected.contains(&name);
        let key = name.clone();
        list = list.child(check_row(
            format!("skill-{name}"),
            name,
            description,
            checked,
            cx.listener(move |this, _, _, cx| {
                let Some(session) = this.selected.clone() else {
                    return;
                };
                let Some(mut draft) = this.skills_draft.clone() else {
                    return;
                };
                if checked {
                    draft.selected.remove(&key);
                } else {
                    draft.selected.insert(key.clone());
                }
                draft.enabled = true;
                if this.submit_config(&session, "skills", Command::SetSkills {
                    session: session.clone(),
                    enabled: draft.enabled,
                    selected: draft.selected.iter().cloned().collect(),
                }, cx) {
                    this.skills_draft = Some(draft);
                }
            }),
            cx,
        ));
    }
    v_flex()
        .id("config-panel-scroll")
        .w_full()
        .min_w_0()
        .max_h(px(460.))
        .overflow_y_scroll()
        .child(list)
        .into_any_element()
}

/// Native-tool and MCP switches, persisted through the agent config.
fn tool_panel(observer: &Observer, mcp_only: bool, cx: &mut Context<Observer>) -> AnyElement {
    let Some(draft) = observer.tool_draft.clone() else {
        return empty_note(t!("mcp_empty").to_string(), cx);
    };
    let tools: Vec<(String, String, bool)> = observer
        .tools
        .iter()
        .filter_map(|tool| {
            let id = tool["id"].as_str()?.to_owned();
            let category = tool["category"].as_str().unwrap_or("");
            let is_mcp = category.eq_ignore_ascii_case("mcp") || id.contains("__MCP__");
            if is_mcp != mcp_only {
                return None;
            }
            Some((
                id,
                tool["name"].as_str().unwrap_or_default().to_owned(),
                is_mcp,
            ))
        })
        .collect();
    let mut list = v_flex().w_full().min_w_0().gap(px(1.)).p_3();
    if tools.is_empty() {
        list = list.child(empty_note(
            if mcp_only {
                t!("mcp_empty").to_string()
            } else {
                t!("tools_empty").to_string()
            },
            cx,
        ));
    }
    for (id, name, is_mcp) in tools {
        let available = if is_mcp {
            draft.mcp_available.contains(&id)
        } else {
            draft.available.contains(&id)
        };
        let auto_approve = if is_mcp {
            draft.mcp_auto_approve.contains(&id)
        } else {
            draft.auto_approve.contains(&id)
        };
        let auto_expand = draft.mcp_auto_expand.contains(&id);
        list = list.child(
            v_flex().w_full().min_w_0().gap_1().py_1().child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .text_size(px(12.))
                            .text_color(cx.theme().foreground)
                            .child(name),
                    )
                    .child(check_chip(
                        format!("tool-available-{id}"),
                        t!("mcp_available").to_string(),
                        available,
                        cx,
                        {
                            let id = id.clone();
                            cx.listener(move |this, _, _, cx| {
                                this.toggle_tool(&id, is_mcp, ToolFlag::Available, cx)
                            })
                        },
                    ))
                    .child(check_chip(
                        format!("tool-approve-{id}"),
                        t!("mcp_auto_approve").to_string(),
                        auto_approve,
                        cx,
                        {
                            let id = id.clone();
                            cx.listener(move |this, _, _, cx| {
                                this.toggle_tool(&id, is_mcp, ToolFlag::AutoApprove, cx)
                            })
                        },
                    ))
                    .when(is_mcp, |row| {
                        row.child(check_chip(
                            format!("tool-expand-{id}"),
                            t!("mcp_auto_expand").to_string(),
                            auto_expand,
                            cx,
                            {
                                let id = id.clone();
                                cx.listener(move |this, _, _, cx| {
                                    this.toggle_tool(&id, true, ToolFlag::AutoExpand, cx)
                                })
                            },
                        ))
                    }),
            ),
        );
    }
    v_flex()
        .id("config-panel-scroll")
        .w_full()
        .min_w_0()
        .max_h(px(460.))
        .overflow_y_scroll()
        .child(list)
        .into_any_element()
}

#[derive(Clone, Copy)]
enum ToolFlag {
    Available,
    AutoApprove,
    AutoExpand,
}

fn check_chip(
    id: String,
    label: String,
    checked: bool,
    cx: &App,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let primary = cx.theme().primary;
    let muted = cx.theme().muted_foreground;
    h_flex()
        .id(SharedString::from(id))
        .flex_none()
        .items_center()
        .gap_1()
        .px_2()
        .py(px(2.))
        .rounded(px(10.))
        .border_1()
        .border_color(if checked {
            primary.opacity(0.45)
        } else {
            cx.theme().border
        })
        .cursor_pointer()
        .text_size(px(10.))
        .text_color(if checked { primary } else { muted })
        .child(
            Icon::new(if checked { "check-circle" } else { "uncheck" })
                .size(11.)
                .color(if checked { primary } else { muted }),
        )
        .child(label)
        .on_click(on_click)
        .into_any_element()
}

impl Observer {
    /// Applies one tool switch to the draft and persists the whole config.
    fn toggle_tool(&mut self, id: &str, mcp: bool, flag: ToolFlag, cx: &mut Context<Self>) {
        let Some(session) = self.selected.clone() else {
            return;
        };
        let Some(draft) = self.tool_draft.as_mut() else {
            return;
        };
        let set = match (mcp, flag) {
            (false, ToolFlag::Available) => &mut draft.available,
            (false, _) => &mut draft.auto_approve,
            (true, ToolFlag::Available) => &mut draft.mcp_available,
            (true, ToolFlag::AutoApprove) => &mut draft.mcp_auto_approve,
            (true, ToolFlag::AutoExpand) => &mut draft.mcp_auto_expand,
        };
        if !set.remove(id) {
            set.insert(id.to_owned());
        }
        // Turning a tool off also drops its approval so the config stays a
        // subset of the available tools, like Vue's toggle handler.
        if !mcp && matches!(flag, ToolFlag::Available) && !draft.available.contains(id) {
            draft.auto_approve.remove(id);
        }
        if mcp && matches!(flag, ToolFlag::Available) && !draft.mcp_available.contains(id) {
            draft.mcp_auto_approve.remove(id);
            draft.mcp_auto_expand.remove(id);
        }
        if mcp && matches!(flag, ToolFlag::Available) {
            // The runtime reads one availability list for native and MCP tools.
            if draft.mcp_available.contains(id) {
                draft.available.insert(id.to_owned());
            } else {
                draft.available.remove(id);
            }
        }
        let config = self.config_with_tools();
        let _ = self
            .worker
            .commands
            .try_send(Command::SetAgentConfig { session, config });
        cx.notify();
    }

    fn config_with_tools(&self) -> Value {
        let mut config = self.agent_config();
        let empty = ToolDraft::default();
        let draft = self.tool_draft.clone().unwrap_or(empty);
        let mut available = draft.available.clone();
        available.extend(draft.mcp_available.iter().cloned());
        config["availableTools"] = json!(available);
        config["autoApprove"] = json!(draft.auto_approve);
        config["mcpTools"] = json!({
            "available": draft.mcp_available,
            "auto_approve": draft.mcp_auto_approve,
            "auto_expand": draft.mcp_auto_expand,
        });
        config
    }
}

fn personality_panel(observer: &Observer, cx: &mut Context<Observer>) -> AnyElement {
    let config = observer.agent_config();
    let agent_custom = observer
        .current_agent()
        .and_then(|agent| agent["personality"].as_str())
        .filter(|style| !style.is_empty() && !style.starts_with("preset:"))
        .map(str::to_owned);
    let current = config["personality"]
        .as_str()
        .filter(|style| !style.is_empty())
        .map(str::to_owned)
        .or_else(|| agent_custom.clone())
        .unwrap_or_else(|| "preset:default".to_owned());
    let mut options: Vec<(String, String, String)> = vec![
        (
            "preset:default".into(),
            t!("personality_default").to_string(),
            t!("personality_default_hint").to_string(),
        ),
        (
            "preset:executor".into(),
            t!("personality_executor").to_string(),
            t!("personality_executor_hint").to_string(),
        ),
        (
            "preset:companion".into(),
            t!("personality_companion").to_string(),
            t!("personality_companion_hint").to_string(),
        ),
        (
            "preset:expert".into(),
            t!("personality_expert").to_string(),
            t!("personality_expert_hint").to_string(),
        ),
        (
            "preset:researcher".into(),
            t!("personality_researcher").to_string(),
            t!("personality_researcher_hint").to_string(),
        ),
        (
            "preset:coach".into(),
            t!("personality_coach").to_string(),
            t!("personality_coach_hint").to_string(),
        ),
        (
            "preset:reviewer".into(),
            t!("personality_reviewer").to_string(),
            t!("personality_reviewer_hint").to_string(),
        ),
    ];
    if let Some(custom) = agent_custom {
        options.push((
            custom,
            t!("personality_custom").to_string(),
            t!("personality_custom_hint").to_string(),
        ));
    }
    let mut list = v_flex().w_full().min_w_0().gap(px(1.)).p_3();
    for (value, label, detail) in options {
        let selected = current == value;
        let target = value.clone();
        list = list.child(check_row(
            format!("personality-{value}"),
            label,
            detail,
            selected,
            cx.listener(move |this, _, _, cx| {
                if let Some(session) = this.selected.clone() {
                    let _ = this.worker.commands.try_send(Command::SetPersonality {
                        session,
                        personality: target.clone(),
                    });
                }
                this.close_quick_panel(cx);
            }),
            cx,
        ));
    }
    v_flex()
        .id("config-panel-scroll")
        .w_full()
        .min_w_0()
        .max_h(px(460.))
        .overflow_y_scroll()
        .child(list)
        .into_any_element()
}
