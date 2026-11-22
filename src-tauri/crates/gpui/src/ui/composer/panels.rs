//! Configuration dialogs opened from the composer's `+` menu.
//!
//! Vue implements the model, skills, MCP and tool configuration as popovers or
//! dialogs anchored in the web page. The native window draws them as one modal
//! over the workflow frame, which keeps a long option list scrollable and lets
//! every switch repaint from the same live state the composer reads.

use gpui_kit::component::{h_flex, switch::Switch, v_flex, ActiveTheme, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use rust_i18n::t;
use serde_json::{json, Value};

use super::footer::{apply_model_choice, ModelChoice};
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
        QuickPanel::Mcp => mcp_panel(observer, cx),
        QuickPanel::Tools => tool_panel(observer, cx),
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
                    .min_w_0()
                    .pl(px(22.))
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(11.))
                    .text_color(muted)
                    .child(detail),
            )
        })
        .on_click(on_click)
        .into_any_element()
}

/// Switches between the model slots on the left and the grouped sources on the
/// right, the way Vue's model configuration dialog separates them.
fn model_panel(observer: &Observer, cx: &mut Context<Observer>) -> AnyElement {
    let generation = observer.model.generation;
    let configs = observer.model_configs();
    let slot = observer.model_config_slot.clone();
    let slot_config = configs[slot.as_str()].clone();
    let current_model = slot_config["model"].as_str().unwrap_or_default().to_owned();
    let current_provider = slot_config["id"].as_i64().unwrap_or_default();
    let proxy = observer.model_config_proxy;
    let border = cx.theme().border;
    let primary = cx.theme().primary;
    let muted = cx.theme().muted_foreground;

    let mut slots = v_flex().w(px(170.)).flex_none().gap(px(2.)).pr_3();
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
                .text_color(if selected { primary } else { cx.theme().foreground })
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.model_config_slot = target.clone();
                    cx.notify();
                })),
        );
    }

    // Providers and proxy aliases are different namespaces, like Vue's two
    // source tabs.
    let mut sources = h_flex().w_full().min_w_0().gap(px(4.)).mb(px(8.));
    for (index, key) in ["model_source_provider", "model_source_proxy"]
        .into_iter()
        .enumerate()
    {
        let active = (index == 1) == proxy;
        let show_proxy = index == 1;
        sources = sources.child(
            div()
                .id(("model-source", index))
                .flex_none()
                .px(px(8.))
                .py(px(4.))
                .rounded(px(8.))
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(if active { primary } else { muted })
                .when(active, |tab| tab.bg(primary.opacity(0.12)))
                .hover(|style| style.bg(cx.theme().muted))
                .child(t!(key).to_string())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.model_config_proxy = show_proxy;
                    cx.notify();
                })),
        );
    }

    let mut models = v_flex()
        .id("model-config-choices")
        .flex_1()
        .min_w_0()
        .max_h(px(360.))
        .overflow_y_scroll()
        .gap(px(2.));
    let mut count = 0usize;
    if proxy {
        if let Some(groups) = observer.proxy_targets.as_object() {
            for (group, aliases) in groups {
                let Some(aliases) = aliases.as_object() else {
                    continue;
                };
                if aliases.is_empty() {
                    continue;
                }
                models = models.child(model_group_label(group.clone(), cx));
                for (alias, targets) in aliases {
                    let model_key = format!("{group}@{alias}");
                    let selected = current_provider == 0 && current_model == model_key;
                    let choice = ModelChoice {
                        provider_id: 0,
                        model: model_key,
                        name: alias.clone(),
                        target: targets.as_array().and_then(|targets| targets.first()).cloned(),
                    };
                    count += 1;
                    models = models.child(model_choice_row(
                        &slot,
                        &slot_config,
                        &configs,
                        &choice,
                        selected,
                        generation,
                        cx,
                    ));
                }
            }
        }
    } else {
        for provider in &observer.models {
            let Some(provider_id) = provider["id"].as_i64() else {
                continue;
            };
            let Some(entries) = provider["models"].as_array() else {
                continue;
            };
            if entries.is_empty() {
                continue;
            }
            models = models.child(model_group_label(
                provider["name"].as_str().unwrap_or_default().to_owned(),
                cx,
            ));
            for entry in entries {
                let Some(model_id) = entry["id"].as_str() else {
                    continue;
                };
                let selected = current_provider == provider_id && current_model == model_id;
                let choice = ModelChoice {
                    provider_id,
                    model: model_id.to_owned(),
                    name: entry["name"].as_str().unwrap_or(model_id).to_owned(),
                    target: Some(entry.clone()),
                };
                count += 1;
                models = models.child(model_choice_row(
                    &slot,
                    &slot_config,
                    &configs,
                    &choice,
                    selected,
                    generation,
                    cx,
                ));
            }
        }
    }
    if count == 0 {
        models = models.child(empty_note(t!("model_placeholder").to_string(), cx));
    }

    h_flex()
        .w_full()
        .p_3()
        .gap_3()
        .border_t_1()
        .border_color(border)
        .child(slots)
        .child(div().w(px(1.)).self_stretch().bg(border))
        .child(v_flex().flex_1().min_w_0().child(sources).child(models))
        .into_any_element()
}

/// One selectable model row; picking it writes the whole slot map.
fn model_choice_row(
    slot: &str,
    slot_config: &Value,
    configs: &Value,
    choice: &ModelChoice,
    selected: bool,
    generation: u64,
    cx: &mut Context<Observer>,
) -> AnyElement {
    let next = apply_model_choice(slot_config, choice);
    let all = configs.clone();
    let slot = slot.to_owned();
    check_row(
        format!("model-choice-{}-{}", choice.provider_id, choice.model),
        choice.name.clone(),
        String::new(),
        selected,
        cx.listener(move |this, _, _, cx| {
            if this.model.accepts(generation) && this.models_pending.is_none() {
                let mut configs = all.clone();
                configs[slot.as_str()] = next.clone();
                this.save_models(configs, cx);
                this.close_quick_panel(cx);
            }
        }),
        cx,
    )
}

/// Vue's skills dialog: an enable switch and the search field share one row.
fn skills_panel(observer: &Observer, cx: &mut Context<Observer>) -> AnyElement {
    let Some(draft) = observer.skills_draft.clone() else {
        return empty_note(t!("skills_empty").to_string(), cx);
    };
    let query = observer.skill_search.read(cx).value().trim().to_lowercase();
    let mut skills: Vec<(String, String)> = observer
        .skills
        .iter()
        .filter_map(|skill| {
            let name = skill["name"].as_str()?;
            let description = skill["description"].as_str().unwrap_or_default();
            (name.to_lowercase().contains(&query) || description.to_lowercase().contains(&query))
                .then(|| (name.to_owned(), description.to_owned()))
        })
        .collect();
    skills.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    let enabled = draft.enabled;
    let selected = draft.selected.clone();
    let border = cx.theme().border;
    let primary = cx.theme().primary;

    let header = h_flex()
        .w_full()
        .min_w_0()
        .items_center()
        .gap(px(16.))
        .px_3()
        .py_2()
        .border_b_1()
        .border_color(border)
        .child(
            h_flex()
                .flex_none()
                .items_center()
                .gap_2()
                .child(
                    Switch::new("skills-enabled")
                        .checked(enabled)
                        .small()
                        .on_click(cx.listener(|this, checked, _, cx| {
                            if let Some(draft) = this.skills_draft.as_mut() {
                                draft.enabled = *checked;
                                cx.notify();
                            }
                        })),
                )
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(cx.theme().foreground)
                        .child(t!("skill_enabled").to_string()),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(gpui_kit::component::input::Input::new(
                    &observer.skill_search,
                )),
        );

    let mut list = v_flex().w_full().min_w_0().gap(px(2.)).p_3();
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
                let Some(draft) = this.skills_draft.as_mut() else {
                    return;
                };
                if !draft.selected.remove(&key) {
                    draft.selected.insert(key.clone());
                }
                cx.notify();
            }),
            cx,
        ));
    }

    let cancel = div()
        .id("skills-cancel")
        .flex_none()
        .px(px(14.))
        .py(px(5.))
        .rounded(px(6.))
        .border_1()
        .border_color(border)
        .cursor_pointer()
        .text_size(px(14.))
        .text_color(cx.theme().foreground)
        .hover(|style| style.bg(cx.theme().muted))
        .child(t!("action_cancel").to_string())
        .on_click(cx.listener(|this, _, _, cx| this.close_quick_panel(cx)));
    let save = div()
        .id("skills-save")
        .flex_none()
        .px(px(14.))
        .py(px(5.))
        .rounded(px(6.))
        .bg(primary)
        .cursor_pointer()
        .text_size(px(14.))
        .text_color(cx.theme().primary_foreground)
        .hover(|style| style.opacity(0.9))
        .child(t!("session_save").to_string())
        .on_click(cx.listener(|this, _, _, cx| {
            let (Some(session), Some(draft)) =
                (this.selected.clone(), this.skills_draft.clone())
            else {
                return;
            };
            if this.submit_config(
                &session,
                "skills",
                Command::SetSkills {
                    session: session.clone(),
                    enabled: draft.enabled,
                    selected: draft.selected.into_iter().collect(),
                },
                cx,
            ) {
                this.close_quick_panel(cx);
            }
        }));

    v_flex()
        .id("skills-panel")
        .w_full()
        .min_w_0()
        .h(px(460.))
        .child(header)
        .child(
            v_flex()
                .id("skills-scroll")
                .w_full()
                .min_w_0()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(list),
        )
        .child(
            h_flex()
                .w_full()
                .flex_none()
                .justify_end()
                .gap(px(8.))
                .px_3()
                .py_2()
                .border_t_1()
                .border_color(border)
                .child(cancel)
                .child(save),
        )
        .into_any_element()
}

/// Tools the selected agent can use, which is what Vue's tool dialog lists.
fn agent_tool_ids(observer: &Observer) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    if let Some(agent) = observer.current_agent() {
        let value = agent
            .get("availableTools")
            .or_else(|| agent.get("available_tools"));
        match value {
            Some(Value::Array(items)) => ids.extend(
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_owned)),
            ),
            Some(Value::String(text)) => {
                if let Ok(Value::Array(items)) = serde_json::from_str::<Value>(text) {
                    ids.extend(
                        items
                            .iter()
                            .filter_map(|item| item.as_str().map(str::to_owned)),
                    );
                }
            }
            _ => {}
        }
    }
    if ids.is_empty() {
        if let Some(draft) = observer.tool_draft.as_ref() {
            ids.extend(draft.available.iter().cloned());
            ids.extend(draft.auto_approve.iter().cloned());
        }
    }
    ids.retain(|id| !is_mcp_id(id));
    ids.sort();
    ids.dedup();
    ids
}

fn is_mcp_id(id: &str) -> bool {
    id.contains("__MCP__")
}

fn tool_display_name(observer: &Observer, id: &str) -> String {
    observer
        .tools
        .iter()
        .find(|tool| tool["id"].as_str() == Some(id))
        .and_then(|tool| tool["name"].as_str())
        .filter(|name| !name.is_empty() && *name != id)
        .unwrap_or(id)
        .to_owned()
}

fn shell_rules(observer: &Observer) -> Vec<(String, String)> {
    let config = observer.agent_config();
    let policy = config
        .get("shellPolicy")
        .or_else(|| config.get("shell_policy"));
    let Some(Value::Array(rules)) = policy else {
        return Vec::new();
    };
    let mut rules: Vec<(String, String)> = rules
        .iter()
        .filter_map(|rule| {
            let pattern = rule["pattern"].as_str()?.trim();
            (!pattern.is_empty()).then(|| {
                (
                    pattern.to_owned(),
                    rule["description"].as_str().unwrap_or_default().to_owned(),
                )
            })
        })
        .collect();
    rules.sort_by(|a, b| a.0.cmp(&b.0));
    rules
}

fn persist_shell_rules(
    observer: &mut Observer,
    rules: Vec<Value>,
    cx: &mut Context<Observer>,
) {
    let Some(session) = observer.selected.clone() else {
        return;
    };
    let mut config = observer.agent_config();
    config["shellPolicy"] = Value::Array(rules);
    let _ = observer
        .worker
        .commands
        .try_send(Command::SetAgentConfig {
            session,
            config,
        });
    cx.notify();
}

/// Vue's tabs: available tools, auto approval and shell rules.
fn tool_panel(observer: &Observer, cx: &mut Context<Observer>) -> AnyElement {
    let Some(draft) = observer.tool_draft.clone() else {
        return empty_note(t!("tools_empty").to_string(), cx);
    };
    let tab = observer.tool_panel_tab.min(2);
    let ids = agent_tool_ids(observer);
    let rules = shell_rules(observer);
    let border = cx.theme().border;
    let primary = cx.theme().primary;
    let muted = cx.theme().muted_foreground;

    let mut tabs = h_flex()
        .w_full()
        .min_w_0()
        .gap(px(4.))
        .px_3()
        .py_2()
        .border_b_1()
        .border_color(border);
    for (index, title) in [
        format!("{} ({})", t!("tool_tab_available"), ids.len()),
        format!("{} ({})", t!("tool_tab_auto_approve"), ids.len()),
        format!("{} ({})", t!("tool_tab_shell"), rules.len()),
    ]
    .into_iter()
    .enumerate()
    {
        let active = index == tab;
        tabs = tabs.child(
            div()
                .id(("tool-tab", index))
                .flex_none()
                .px(px(8.))
                .py(px(4.))
                .rounded(px(8.))
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(if active { primary } else { muted })
                .when(active, |button| button.bg(primary.opacity(0.12)))
                .hover(|style| style.bg(cx.theme().muted))
                .child(title)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.tool_panel_tab = index;
                    cx.notify();
                })),
        );
    }

    let mut body = v_flex().w_full().min_w_0().flex_1().min_h_0().overflow_y_scroll().p_3();
    match tab {
        0 => {
            if ids.is_empty() {
                body = body.child(empty_note(t!("tools_empty").to_string(), cx));
            }
            for id in ids {
                let available = draft.available.contains(&id);
                let name = tool_display_name(observer, &id);
                body = body.child(toggle_row(
                    format!("tool-available-{id}"),
                    id.clone(),
                    name,
                    available,
                    {
                        let id = id.clone();
                        cx.listener(move |this, checked: &bool, _, cx| {
                            if *checked != this.tool_draft.as_ref().is_some_and(|draft| draft.available.contains(&id)) {
                                this.toggle_tool(&id, false, ToolFlag::Available, cx);
                            }
                        })
                    },
                    cx,
                ));
            }
        }
        1 => {
            let checkable: Vec<String> = ids
                .into_iter()
                .filter(|id| id != "bash")
                .filter(|id| draft.available.contains(id))
                .collect();
            if checkable.is_empty() {
                body = body.child(empty_note(t!("tools_empty").to_string(), cx));
            }
            for id in checkable {
                let approved = draft.auto_approve.contains(&id);
                let name = tool_display_name(observer, &id);
                body = body.child(toggle_row(
                    format!("tool-approve-{id}"),
                    id.clone(),
                    name,
                    approved,
                    {
                        let id = id.clone();
                        cx.listener(move |this, checked: &bool, _, cx| {
                            if *checked != this.tool_draft.as_ref().is_some_and(|draft| draft.auto_approve.contains(&id)) {
                                this.toggle_tool(&id, false, ToolFlag::AutoApprove, cx);
                            }
                        })
                    },
                    cx,
                ));
            }
        }
        _ => {
            let observer_entity = cx.entity();
            let config = observer.agent_config();
            let existing: Vec<Value> = config
                .get("shellPolicy")
                .or_else(|| config.get("shell_policy"))
                .and_then(|policy| policy.as_array().cloned())
                .unwrap_or_default();
            let mut rules_body = v_flex().w_full().min_w_0().gap(px(4.));
            if rules.is_empty() {
                rules_body = rules_body.child(empty_note(t!("tools_empty").to_string(), cx));
            }
            for (pattern, description) in rules {
                let mut remaining = existing.clone();
                remaining.retain(|rule| rule["pattern"].as_str() != Some(pattern.as_str()));
                let observer_entity = observer_entity.clone();
                rules_body = rules_body.child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .items_center()
                        .gap(px(8.))
                        .px_2()
                        .py_1()
                        .rounded(px(6.))
                        .hover(|style| style.bg(cx.theme().muted))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_size(px(12.))
                                .font_family(crate::MONO_FONT)
                                .text_color(cx.theme().foreground)
                                .child(pattern.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .px(px(4.))
                                .py(px(1.))
                                .rounded(px(4.))
                                .border_1()
                                .border_color(border)
                                .text_size(px(10.))
                                .text_color(muted)
                                .child(description),
                        )
                        .child(
                            div()
                                .id(SharedString::from(format!("shell-rule-remove-{pattern}")))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .size(px(18.))
                                .rounded(px(4.))
                                .cursor_pointer()
                                .hover(|style| style.bg(cx.theme().border))
                                .child(Icon::new("close").size(10.).color(muted))
                                .on_click(move |_, _, cx: &mut App| {
                                    let _ = observer_entity.update(cx, |this, cx| {
                                        persist_shell_rules(this, remaining.clone(), cx);
                                    });
                                }),
                        ),
                );
            }
            body = body.child(rules_body);
        }
    }

    v_flex()
        .id("tool-panel")
        .w_full()
        .min_w_0()
        .h(px(460.))
        .child(tabs)
        .child(body)
        .into_any_element()
}

/// Vue's MCP panel: the tool on top, its server underneath, switches on the right.
fn mcp_panel(observer: &Observer, cx: &mut Context<Observer>) -> AnyElement {
    let Some(draft) = observer.tool_draft.clone() else {
        return empty_note(t!("mcp_empty").to_string(), cx);
    };
    let mut tools: Vec<(String, String)> = observer
        .tools
        .iter()
        .filter_map(|tool| {
            let id = tool["id"].as_str()?.to_owned();
            let category = tool["category"].as_str().unwrap_or("");
            if !category.eq_ignore_ascii_case("mcp") && !is_mcp_id(&id) {
                return None;
            }
            let name = tool["name"].as_str().unwrap_or(&id).to_owned();
            Some((id, name))
        })
        .collect();
    for id in draft.mcp_available.iter().chain(draft.mcp_auto_approve.iter()) {
        if !tools.iter().any(|(existing, _)| existing == id) {
            tools.push((id.clone(), id.clone()));
        }
    }
    tools.sort_by(|a, b| a.0.cmp(&b.0));
    let border = cx.theme().border;
    let muted = cx.theme().muted_foreground;

    let header = h_flex()
        .w_full()
        .min_w_0()
        .items_center()
        .gap(px(8.))
        .px_3()
        .py_2()
        .border_b_1()
        .border_color(border)
        .child(div().flex_1().min_w_0())
        .child(
            div()
                .w(px(64.))
                .flex_none()
                .text_size(px(10.))
                .text_align(TextAlign::Center)
                .text_color(muted)
                .child(t!("mcp_available").to_string()),
        )
        .child(
            div()
                .w(px(64.))
                .flex_none()
                .text_size(px(10.))
                .text_align(TextAlign::Center)
                .text_color(muted)
                .child(t!("mcp_auto_approve").to_string()),
        )
        .child(
            div()
                .w(px(64.))
                .flex_none()
                .text_size(px(10.))
                .text_align(TextAlign::Center)
                .text_color(muted)
                .child(t!("mcp_auto_expand").to_string()),
        );

    let mut rows = v_flex().w_full().min_w_0().gap(px(2.)).p_3();
    if tools.is_empty() {
        rows = rows.child(empty_note(t!("mcp_empty").to_string(), cx));
    }
    for (id, name) in tools {
        let server = id.split("__MCP__").next().unwrap_or_default().to_owned();
        let available = draft.mcp_available.contains(&id);
        let approved = draft.mcp_auto_approve.contains(&id);
        let expanded = draft.mcp_auto_expand.contains(&id);
        let mut row = h_flex().w_full().min_w_0().items_center().gap(px(8.)).px_2().py_1();
        row = row.child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(1.))
                .child(
                    div()
                        .min_w_0()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(cx.theme().foreground)
                        .child(name),
                )
                .when(!server.is_empty(), |column| {
                    column.child(
                        div()
                            .min_w_0()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(11.))
                            .text_color(muted)
                            .child(server.clone()),
                    )
                }),
        );
        for (flag, checked) in [
            (ToolFlag::Available, available),
            (ToolFlag::AutoApprove, approved),
            (ToolFlag::AutoExpand, expanded),
        ] {
            let id = id.clone();
            row = row.child(
                div().w(px(64.)).flex_none().flex().items_center().justify_center().child(
                    Switch::new(SharedString::from(format!("mcp-{flag:?}-{id}")))
                        .checked(checked)
                        .small()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_tool(&id, true, flag, cx);
                        })),
                ),
            );
        }
        rows = rows.child(row);
    }

    v_flex()
        .id("mcp-panel")
        .w_full()
        .min_w_0()
        .h(px(460.))
        .child(header)
        .child(
            v_flex()
                .id("mcp-scroll")
                .w_full()
                .min_w_0()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(rows),
        )
        .into_any_element()
}

/// One labelled switch row used by the tools tabs.
fn toggle_row(
    id: String,
    label: String,
    detail: String,
    checked: bool,
    on_click: impl Fn(&bool, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    h_flex()
        .id(SharedString::from(id.clone()))
        .w_full()
        .min_w_0()
        .items_center()
        .gap(px(8.))
        .px_2()
        .py(px(4.))
        .rounded(px(6.))
        .hover(|style| style.bg(cx.theme().muted))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(1.))
                .child(
                    div()
                        .min_w_0()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(13.))
                        .font_family(crate::MONO_FONT)
                        .text_color(cx.theme().foreground)
                        .child(label),
                )
                .when(!detail.is_empty(), |column| {
                    column.child(
                        div()
                            .min_w_0()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(11.))
                            .text_color(cx.theme().muted_foreground)
                            .child(detail.clone()),
                    )
                }),
        )
        .child(
            Switch::new(SharedString::from(format!("{id}:switch")))
                .checked(checked)
                .small()
                .on_click(on_click),
        )
        .into_any_element()
}

#[derive(Clone, Copy, Debug)]
enum ToolFlag {
    Available,
    AutoApprove,
    AutoExpand,
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
