//! Footer under the prompt box.
//!
//! Mirrors the `.input-footer` block of `src/components/workflow/WorkflowInputArea.vue`:
//! the agent, model, quick-action and sandbox selectors on the left, the
//! continue/stop/send group on the right. Every selector reads the same data the
//! Vue panel does — the control-plane agents, the runtime model catalogue and the
//! workflow's own agent config — and writes through the allowlisted workflow
//! commands, so the native window never shows a control that cannot act.

use gpui_kit::component::popover::PopoverState;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use rust_i18n::t;
use serde_json::Value;

use super::model_config::{reasoning_budget, reasoning_level};
use crate::ui::{menu_popover::MenuPopover, Icon};
use crate::{Observer, QuickPanel};
use chatspeed_gpui::types::Command;

/// The actions under the box, in Vue's left/right arrangement.
pub fn footer(observer: &Observer, cx: &mut Context<Observer>) -> AnyElement {
    let accent = cx.theme().primary;
    let hover = cx.theme().muted;
    let muted = cx.theme().muted_foreground;
    let can_stop = observer.model.can_stop();
    let can_send = (!observer.composer.read(cx).value().trim().is_empty()
        || !observer.attachments.is_empty())
        && !observer.start_pending
        && !observer.clear_pending
        && observer.queued_edit.is_none()
        && observer.vision_task.is_none();

    h_flex()
        .w_full()
        .items_center()
        .gap(px(8.))
        .px_3()
        .pb_2()
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap(px(8.))
                .child(agent_selector(observer, hover, muted, cx))
                .child(model_selector(observer, hover, muted, cx))
                .child(quick_actions(observer, hover, muted, cx))
                .when(observer.selected.is_some(), |row| {
                    row.child(sandbox_selector(observer, hover, muted, cx))
                        .child(clear_context(observer, hover, muted, cx))
                }),
        )
        .when(observer.can_continue(), |row| {
            row.child(continue_button(accent, observer.resume_pending(), cx))
        })
        .when(can_stop, |row| {
            row.child(icon_action(
                "stop-workflow",
                "stop",
                cx.theme().foreground,
                hover,
                cx.listener(|this, _, window, cx| this.stop_workflow(window, cx)),
            ))
        })
        .child(
            h_flex()
                .id("send-message")
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(26.))
                .rounded(crate::ui::tokens::RADIUS_SMALL)
                .when(can_send, |view| {
                    view.cursor_pointer().hover(|style| style.bg(hover))
                })
                .child(
                    Icon::new(if observer.vision_task.is_some() {
                        "loading"
                    } else {
                        "send"
                    })
                    .size(20.)
                    .color(if can_send { accent } else { muted }),
                )
                .tooltip(crate::ui::tip(t!("send").to_string()))
                .tooltip_show_delay(crate::ui::TOOLTIP_DELAY)
                .on_click(cx.listener(|this, _, window, cx| this.send_message(window, cx))),
        )
        .into_any_element()
}

/// Theme colors one option row needs, read before the popover closure runs.
#[derive(Clone, Copy)]
struct OptionColors {
    primary: Hsla,
    hover: Hsla,
    muted: Hsla,
    foreground: Hsla,
}

impl OptionColors {
    fn new(cx: &App) -> Self {
        Self {
            primary: cx.theme().primary,
            hover: cx.theme().muted,
            muted: cx.theme().muted_foreground,
            foreground: cx.theme().foreground,
        }
    }
}

/// Selection state and tone for one quick-action row.
#[derive(Clone, Copy)]
struct MenuRowStyle {
    checked: bool,
    danger: bool,
    colors: OptionColors,
    disabled: bool,
}

impl MenuRowStyle {
    fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    fn new(colors: OptionColors, checked: bool, danger: bool) -> Self {
        Self {
            checked,
            danger,
            colors,
            disabled: false,
        }
    }
}

/// One footer pill: a label, an optional caret, and a hover surface.
///
/// Vue's agent trigger is a bordered capsule while the model trigger is a bare
/// text button, so the shape is passed in instead of duplicated.
fn pill(
    id: &'static str,
    icon: Option<&'static str>,
    label: String,
    caret: bool,
    enabled: bool,
    hover: Hsla,
    color: Hsla,
    bordered: bool,
    cx: &App,
) -> AnyElement {
    let primary = cx.theme().primary;
    let background = cx.theme().background;
    action_capsule(id)
        .gap(crate::ui::tokens::SPACE_4)
        .when(enabled, |row| row.cursor_pointer())
        .when(!enabled, |row| row.cursor_default())
        // Vue's agent pill and model trigger both sit on `--cs-bg-color`; the
        // theme background is the native token closest to that surface.
        .bg(background)
        .when(bordered, |row| {
            row.border_1().border_color(if color == primary {
                primary
            } else {
                cx.theme().border
            })
        })
        .when(enabled, |row| {
            row.hover(move |style| style.bg(hover).text_color(primary))
        })
        // A disabled pill keeps both its surface and its label color; without a
        // hover style GPUI can retain the previous interactive hover state.
        .when(!enabled, |row| {
            row.hover(move |style| style.bg(background).text_color(color))
        })
        .text_size(px(12.))
        .line_height(px(16.))
        .text_color(color)
        .when_some(icon, |row, icon| {
            row.child(Icon::new(icon).size(14.).color(color))
        })
        .child(
            div()
                .max_w(px(160.))
                .overflow_hidden()
                .text_ellipsis()
                .child(crate::ui::chrome_label(label.clone())),
        )
        .when(caret, |row| {
            row.child(Icon::new("caret-down").size(12.).color(color))
        })
        .into_any_element()
}

/// One row of a selector popover, with the check mark Vue renders on the choice.
fn option_row(
    id: String,
    label: String,
    detail: String,
    selected: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    colors: OptionColors,
) -> AnyElement {
    let OptionColors {
        primary,
        hover,
        muted,
        foreground,
    } = colors;
    v_flex()
        .id(SharedString::from(id))
        .w_full()
        .min_w_0()
        .gap(px(2.))
        .px_2()
        .py_1()
        .rounded(crate::ui::tokens::RADIUS_SMALL)
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_size(px(12.))
                        .text_color(if selected { primary } else { foreground })
                        .child(crate::ui::chrome_label(label)),
                )
                .when(selected, |row| {
                    row.child(Icon::new("check").size(12.).color(primary))
                }),
        )
        .when(!detail.is_empty(), |row| {
            row.child(
                div()
                    .w_full()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .text_size(px(11.))
                    .text_color(muted)
                    .child(detail),
            )
        })
        .on_click(on_click)
        .into_any_element()
}

/// The agent pill: Vue's `AgentSelector`, limited to selectable primary agents.
fn agent_selector(
    observer: &Observer,
    hover: Hsla,
    muted: Hsla,
    cx: &mut Context<Observer>,
) -> AnyElement {
    let current = observer.current_agent_id();
    let label = observer
        .current_agent()
        .and_then(|agent| agent["name"].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| t!("agent_placeholder").to_string());
    // Vue disables the selector while a session is live, or once the workflow
    // has run without clearing its context. An empty choice list is equally
    // non-interactive even when switching is otherwise allowed.
    let agents: Vec<(String, String, String)> = observer
        .agents
        .iter()
        .filter(|agent| agent["role"].as_str().is_none_or(|role| role == "primary"))
        .filter(|agent| !agent["disabled"].as_bool().unwrap_or(false))
        .filter_map(|agent| {
            Some((
                agent["id"].as_str()?.to_owned(),
                agent["name"].as_str().unwrap_or_default().to_owned(),
                agent["description"].as_str().unwrap_or_default().to_owned(),
            ))
        })
        .collect();
    let session = observer.selected.clone();
    let editable = observer.can_switch_agent() && session.is_some() && !agents.is_empty();
    let primary = cx.theme().primary;
    let color = if editable {
        primary
    } else {
        muted.opacity(0.4)
    };
    let trigger_el = pill(
        "composer-agent",
        Some("agent"),
        label,
        true,
        editable,
        hover,
        color,
        true,
        cx,
    );
    let Some(session) = session else {
        return trigger_el;
    };
    if !observer.can_switch_agent() || agents.is_empty() {
        return trigger_el;
    }
    let selected_index = agents
        .iter()
        .position(|(id, _, _)| current.as_deref() == Some(id.as_str()));
    let selected = current.clone();
    let observer_entity = cx.entity();
    MenuPopover::new(
        "composer-agent-popover",
        trigger_el,
        move |_: &mut PopoverState, _: &mut Window, cx: &mut Context<PopoverState>| {
            let popover = cx.entity();
            let mut list = v_flex().id("composer-agent-list").w(px(240.)).gap(px(2.));
            for (id, name, description) in agents.clone() {
                let popover = popover.clone();
                let observer_entity = observer_entity.clone();
                let session = session.clone();
                let is_selected = selected.as_deref() == Some(id.as_str());
                let target = id.clone();
                list = list.child(option_row(
                    format!("composer-agent-{id}"),
                    name,
                    description,
                    is_selected,
                    move |_, window, cx| {
                        popover.update(cx, |state, cx| state.dismiss(window, cx));
                        observer_entity.update(cx, |this, cx| {
                            if this.can_switch_agent() && !is_selected {
                                this.submit_config(
                                    &session,
                                    "agent",
                                    Command::SetAgent {
                                        session: session.clone(),
                                        agent: target.clone(),
                                    },
                                    cx,
                                );
                            }
                        });
                    },
                    OptionColors::new(cx),
                ));
            }
            list
        },
    )
    .selected_index(selected_index)
    .into_any_element()
}

/// One selectable model, either a provider entry or a proxy alias.
#[derive(Clone)]
pub(crate) struct ModelChoice {
    pub provider_id: i64,
    pub model: String,
    pub name: String,
    /// Provider entry copied into the slot so capabilities stay in sync.
    pub target: Option<Value>,
}

/// The provider model behind an `(id, model)` pair, used for capabilities.
fn provider_model(models: &[Value], provider_id: i64, model: &str) -> Option<Value> {
    models
        .iter()
        .find(|provider| provider["id"].as_i64() == Some(provider_id))
        .and_then(|provider| {
            provider["models"]
                .as_array()?
                .iter()
                .find(|entry| entry["id"].as_str() == Some(model))
                .cloned()
        })
}

/// Model choices for one slot, matching Vue's `buildCurrentModelOptions`.
pub(crate) fn model_choices(observer: &Observer, config: &Value) -> (String, Vec<ModelChoice>) {
    model_choices_from(&observer.models, &observer.proxy_targets, config)
}

/// Same resolution for callers that own a detached catalog, such as the
/// automation editor's local model draft.
pub(crate) fn model_choices_from(
    models: &[Value],
    proxy_targets: &Value,
    config: &Value,
) -> (String, Vec<ModelChoice>) {
    let current = config["model"].as_str().unwrap_or("");
    let provider_id = config["id"].as_i64().unwrap_or_default();
    if provider_id == 0 && current.contains('@') {
        let group = current.split('@').next().unwrap_or("");
        let aliases: Vec<ModelChoice> = proxy_targets[group]
            .as_object()
            .map(|aliases| {
                aliases
                    .keys()
                    .map(|alias| {
                        let target = proxy_targets[group][alias]
                            .as_array()
                            .and_then(|targets| targets.first());
                        let target = target.and_then(|target| {
                            let id = target["id"].as_i64()?;
                            let model = target["model"].as_str()?;
                            provider_model(models, id, model)
                        });
                        ModelChoice {
                            provider_id: 0,
                            model: format!("{group}@{alias}"),
                            name: alias.clone(),
                            target,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        if aliases.is_empty() && !current.is_empty() {
            // Keep the pill responsive even before the settings poll delivers
            // its proxy map.
            let alias = current
                .split_once('@')
                .map(|(_, alias)| alias)
                .unwrap_or(current);
            return (
                group.to_owned(),
                vec![ModelChoice {
                    provider_id: 0,
                    model: current.to_owned(),
                    name: alias.to_owned(),
                    target: None,
                }],
            );
        }
        return (group.to_owned(), aliases);
    }
    let Some(provider) = models
        .iter()
        .find(|provider| provider["id"].as_i64() == Some(provider_id))
    else {
        // A stale provider id must not turn the pill into a dead control.
        if current.is_empty() {
            return (String::new(), Vec::new());
        }
        return (
            String::new(),
            vec![ModelChoice {
                provider_id,
                model: current.to_owned(),
                name: current.to_owned(),
                target: None,
            }],
        );
    };
    let label = provider["name"].as_str().unwrap_or_default().to_owned();
    let choices = provider["models"]
        .as_array()
        .map(|models| {
            models
                .iter()
                .filter_map(|entry| {
                    let id = entry["id"].as_str()?.to_owned();
                    Some(ModelChoice {
                        provider_id,
                        name: entry["name"]
                            .as_str()
                            .or_else(|| entry["id"].as_str())
                            .unwrap_or_default()
                            .to_owned(),
                        model: id,
                        target: Some(entry.clone()),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    (label, choices)
}

/// An explicit catalogue capability wins over a session's possibly stale setting.
fn model_supports_thinking(choice: &ModelChoice, config: &Value) -> bool {
    let enabled = |thinking: &Value| {
        thinking["type"]
            .as_str()
            .is_some_and(|kind| kind.eq_ignore_ascii_case("enabled"))
    };
    if let Some(target) = &choice.target {
        return target["reasoning"]
            .as_bool()
            .unwrap_or_else(|| enabled(&target["thinking"]));
    }
    // Unresolved proxy aliases and legacy entries only have the active slot to consult.
    config["id"].as_i64() == Some(choice.provider_id)
        && config["model"].as_str() == Some(choice.model.as_str())
        && enabled(&config["thinking"])
}

/// Applies a choice to a slot config, copying Vue's capability fields.
pub(crate) fn apply_model_choice(config: &Value, choice: &ModelChoice) -> Value {
    let mut next = config.clone();
    next["id"] = Value::from(choice.provider_id);
    next["model"] = Value::from(choice.model.clone());
    if let Some(target) = &choice.target {
        next["thinking"] =
            if target["thinking"].is_null() || target["reasoning"].as_bool() == Some(false) {
                serde_json::json!({ "type": "disabled" })
            } else {
                target["thinking"].clone()
            };
        for key in ["functionCall", "temperature", "contextSize", "maxTokens"] {
            if !target[key].is_null() {
                next[key] = target[key].clone();
            }
        }
    }
    next
}

#[cfg(test)]
mod model_effort_tests {
    use super::{apply_model_choice, model_supports_thinking, ModelChoice};
    use serde_json::{json, Value};

    fn model(name: &str, target: Option<Value>) -> ModelChoice {
        ModelChoice {
            provider_id: 7,
            model: name.into(),
            name: name.into(),
            target,
        }
    }

    #[test]
    fn model_effort_uses_catalogue_capability_over_stale_session_thinking() {
        let config = json!({
            "id": 7,
            "model": "plain",
            "thinking": {"type": "enabled", "budgetTokens": 8192}
        });
        assert!(!model_supports_thinking(
            &model("plain", Some(json!({"reasoning": false}))),
            &config,
        ));
        assert!(!model_supports_thinking(
            &model("plain", Some(json!({}))),
            &config,
        ));
        assert!(model_supports_thinking(
            &model("reasoner", Some(json!({"reasoning": true}))),
            &config,
        ));
        assert!(model_supports_thinking(
            &model("legacy", Some(json!({"thinking": {"type": "enabled"}}))),
            &config,
        ));
        assert!(model_supports_thinking(&model("plain", None), &config));
        assert!(!model_supports_thinking(&model("other", None), &config));
    }

    #[test]
    fn model_effort_does_not_leak_to_a_new_model() {
        let config = json!({
            "id": 7,
            "model": "reasoner",
            "thinking": {"type": "enabled", "budgetTokens": 8192}
        });
        let plain = model("plain", Some(json!({"reasoning": false})));
        let switched = apply_model_choice(&config, &plain);
        assert_eq!(switched["model"], "plain");
        assert_eq!(switched["thinking"], json!({"type": "disabled"}));
        assert!(!model_supports_thinking(&plain, &switched));
    }
}

fn model_effort_row(
    thinking: &Value,
    on_select: impl Fn(u32, &mut Window, &mut App) + 'static,
    cx: &App,
) -> impl IntoElement {
    let on_select = std::rc::Rc::new(on_select);
    let current = reasoning_level(thinking["budgetTokens"].as_u64().unwrap_or_default());
    let levels = crate::ui::tabs::capsule_segments_sized(
        "composer-thinking-level",
        vec![
            ("low", t!("reasoning_low").to_string()),
            ("medium", t!("reasoning_medium").to_string()),
            ("high", t!("reasoning_high").to_string()),
            ("max", t!("reasoning_max").to_string()),
        ],
        current,
        // A form-sized capsule outgrows this 12px popover, so drop to the
        // capsule's smallest step and keep the row as compact as the model names.
        20_f32,
        move |level, window, cx| on_select(reasoning_budget(level), window, cx),
    );
    h_flex()
        .w_full()
        .min_w_0()
        .items_center()
        .gap(px(8.))
        .px_2()
        .pb_1()
        .child(
            div()
                .flex_none()
                .whitespace_nowrap()
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(cx.theme().muted_foreground)
                .child(t!("model_reasoning_level").to_string()),
        )
        // The capsule wraps its segments inside the popover's narrow width
        // instead of pushing the label out of the row.
        .child(div().flex_1().min_w_0().child(levels))
}

/// The model pill: the active provider's models, as Vue's compact selector lists.
fn model_selector(
    observer: &Observer,
    hover: Hsla,
    muted: Hsla,
    cx: &mut Context<Observer>,
) -> AnyElement {
    let label = observer.model_label();
    let trigger_el = pill(
        "composer-model",
        None,
        label,
        true,
        true,
        hover,
        muted,
        false,
        cx,
    );
    let Some(session) = observer.selected.clone() else {
        return trigger_el;
    };
    // A pending save must not drop the popover: the menu is what triggered the
    // save, and unmounting it here would close the menu mid-selection. The
    // optimistic slot map already reflects the pending write.
    if observer.options.demo {
        return trigger_el;
    }
    let generation = observer.model.generation;
    let config = observer.active_model_config();
    let slot = if observer.planning_mode {
        "plan"
    } else {
        "act"
    };
    let current_model = config["model"].as_str().unwrap_or_default().to_owned();
    let (group, entries) = model_choices(observer, &config);
    if entries.is_empty() {
        return trigger_el;
    }
    let selected_index = entries
        .iter()
        .position(|choice| choice.model == current_model)
        .map(|index| index + usize::from(!group.is_empty()));
    let base_config = config.clone();
    let all_configs = observer.model_configs();
    let observer_entity = cx.entity();
    // Selecting a model or a reasoning level writes through `save_models` and
    // re-renders this list from the optimistic slot map, so the menu stays open
    // and the two choices can be made in one visit.
    MenuPopover::new(
        "composer-model-popover",
        trigger_el,
        move |_: &mut PopoverState, _: &mut Window, cx: &mut Context<PopoverState>| {
            let mut list = v_flex().id("composer-model-list").w_full().gap(px(4.));
            if !group.is_empty() {
                list = list.child(model_group_label(group.clone(), cx));
            }
            let colors = OptionColors::new(cx);
            for choice in &entries {
                let is_selected = current_model == choice.model;
                let effort =
                    (is_selected && model_supports_thinking(choice, &base_config)).then(|| {
                        let entity = observer_entity.clone();
                        let session = session.clone();
                        let model = choice.model.clone();
                        model_effort_row(
                            &base_config["thinking"],
                            move |budget, _, cx| {
                                entity.update(cx, |this, cx| {
                                    if this.model.accepts(generation)
                                        && this.selected.as_ref() == Some(&session)
                                        && this.planning_mode == (slot == "plan")
                                        && this.active_model_config()["model"].as_str()
                                            == Some(model.as_str())
                                    {
                                        let mut configs = this.model_configs();
                                        configs[slot] = this.active_model_config();
                                        configs[slot]["thinking"] = serde_json::json!({
                                            "type": "enabled", "budgetTokens": budget
                                        });
                                        this.save_models(configs, cx);
                                    }
                                });
                            },
                            cx,
                        )
                    });
                let observer_entity = observer_entity.clone();
                let session = session.clone();
                let mut configs = all_configs.clone();
                configs[slot] = apply_model_choice(&base_config, choice);
                let title = h_flex()
                    .id(SharedString::from(format!(
                        "composer-model-{}",
                        choice.model
                    )))
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded(crate::ui::tokens::RADIUS_SMALL)
                    .cursor_pointer()
                    .when(!is_selected, |row| {
                        row.hover(move |style| style.bg(colors.hover))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(12.))
                            .text_color(colors.foreground)
                            .child(crate::ui::chrome_label(choice.name.clone())),
                    )
                    .when(is_selected, |row| {
                        row.child(Icon::new("check").size(14.).color(colors.primary))
                    })
                    .on_click(move |_, _, cx| {
                        if !is_selected {
                            observer_entity.update(cx, |this, cx| {
                                if this.model.accepts(generation)
                                    && this.selected.as_ref() == Some(&session)
                                    && this.planning_mode == (slot == "plan")
                                {
                                    this.save_models(configs.clone(), cx);
                                }
                            });
                        }
                    });
                list = list.child(
                    v_flex()
                        .relative()
                        .min_w_0()
                        .border_1()
                        .border_color(if is_selected {
                            colors.primary.opacity(0.45)
                        } else {
                            transparent_black()
                        })
                        .rounded(crate::ui::tokens::RADIUS_SMALL)
                        .when(is_selected, |card| {
                            card.bg(colors.primary.opacity(0.09)).child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .top_1()
                                    .bottom_1()
                                    .w(px(2.))
                                    .rounded_full()
                                    .bg(colors.primary),
                            )
                        })
                        .child(title)
                        // Only the title selects the model; the effort buttons are siblings.
                        .when_some(effort, |card, effort| card.child(effort)),
                );
            }
            list
        },
    )
    .width(px(315.))
    .selected_index(selected_index)
    .into_any_element()
}

pub(super) fn model_group_label(label: String, cx: &App) -> AnyElement {
    div()
        .w_full()
        .px_2()
        .py_1()
        .text_size(px(10.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(cx.theme().muted_foreground)
        .child(crate::ui::chrome_label(label))
        .into_any_element()
}

fn action_capsule(id: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .h(px(24.))
        .px(crate::ui::tokens::SPACE_8)
        .rounded_full()
}

/// Vue's `+` quick actions: the compress command, the runtime switches and the
/// approval level, grouped under the same section titles the web menu uses.
fn quick_actions(
    observer: &Observer,
    hover: Hsla,
    muted: Hsla,
    cx: &mut Context<Observer>,
) -> AnyElement {
    let planning = observer.planning_mode;
    let can_plan = observer.can_toggle_planning();
    let auto_approve_plan = observer.agent_bool("autoApprovePlan", "auto_approve_plan", false);
    let auto_compress = observer.auto_compress_enabled();
    let final_audit = observer.final_audit_enabled();
    // The runtime ignores the toggle when no final-reviewer child agent can run
    // it; the row has to read as disabled instead of swallowing the click.
    let can_final_audit = observer.can_toggle_final_audit();
    let active_count = usize::from(planning)
        + usize::from(planning && auto_approve_plan)
        + usize::from(final_audit)
        + usize::from(auto_compress);
    let accent = cx.theme().primary;
    // Vue's `icon-btn`: 2x4 padding with a hover surface, no fixed box.
    let trigger_el = action_capsule("composer-quick-actions")
        .bg(cx.theme().background)
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(Icon::new("add").size(14.).color(muted))
        .tooltip(crate::ui::tip(t!("quick_actions").to_string()))
        .tooltip_show_delay(crate::ui::TOOLTIP_DELAY)
        .when(active_count > 0, |button| {
            button.child(
                div()
                    .absolute()
                    .top(px(-5.))
                    .right(px(-5.))
                    .min_w(px(15.))
                    .h(px(15.))
                    .px_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(accent)
                    .text_size(px(10.))
                    .line_height(px(15.))
                    .text_align(TextAlign::Center)
                    .text_color(gpui_kit::white())
                    .child(active_count.to_string()),
            )
        })
        .into_any_element();
    let Some(session) = observer.selected.clone() else {
        if observer.start_pending {
            return trigger_el;
        }
        let owner = cx.entity();
        return MenuPopover::new("composer-draft-quick-popover", trigger_el, move |_, _, cx| {
            let (enabled, can_toggle) = {
                let observer = owner.read(cx);
                (observer.final_audit_enabled(), observer.can_toggle_final_audit())
            };
            let owner = owner.clone();
            v_flex().id("composer-draft-quick-list").w(px(280.)).child(menu_row(
                "composer-draft-final-audit",
                "check-circle",
                t!("final_audit_label").to_string(),
                t!("final_audit_hint").to_string(),
                MenuRowStyle::new(OptionColors::new(cx), enabled, false).disabled(!can_toggle),
                move |_: &mut Window, cx: &mut App| {
                    owner.update(cx, |this, cx| {
                        this.toggle_runtime_preference("finalAudit", cx);
                    });
                },
            ))
        })
        .into_any_element();
    };
    let approval = observer.approval_level();
    let observer_entity = cx.entity();
    MenuPopover::new(
        "composer-quick-popover",
        trigger_el,
        move |_: &mut PopoverState, _: &mut Window, cx: &mut Context<PopoverState>| {
            let popover = cx.entity();
            let observer_entity = observer_entity.clone();
            let session = session.clone();
            let colors = OptionColors::new(cx);
            // Every entry acts on a fresh copy of the runtime state, so the
            // menu closes after the command like Vue's dropdown does.
            let compress = {
                let popover = popover.clone();
                let observer_entity = observer_entity.clone();
                move |window: &mut Window, cx: &mut App| {
                    popover.update(cx, |state, cx| state.dismiss(window, cx));
                    observer_entity.update(cx, |this, cx| this.manual_compress(cx));
                }
            };
            let attach = {
                let popover = popover.clone();
                let observer_entity = observer_entity.clone();
                move |window: &mut Window, cx: &mut App| {
                    popover.update(cx, |state, cx| state.dismiss(window, cx));
                    observer_entity.update(cx, |this, cx| this.pick_attachments(cx));
                }
            };
            let open_panel = |panel: QuickPanel| {
                let popover = popover.clone();
                let observer_entity = observer_entity.clone();
                move |window: &mut Window, cx: &mut App| {
                    popover.update(cx, |state, cx| state.dismiss(window, cx));
                    observer_entity.update(cx, |this, cx| this.open_quick_panel(panel, window, cx));
                }
            };
            // Runtime switches stay open: the menu is where a user flips
            // several of them in one visit, matching Vue's checkbox menu.
            let toggle_planning = {
                let observer_entity = observer_entity.clone();
                move |_: &mut Window, cx: &mut App| {
                    observer_entity.update(cx, |this, cx| {
                        this.toggle_planning(cx);
                    });
                }
            };
            let toggle_auto_approve_plan = {
                let observer_entity = observer_entity.clone();
                let session = session.clone();
                move |_: &mut Window, cx: &mut App| {
                    observer_entity.update(cx, |this, cx| {
                        let mut config = this.agent_config();
                        config["autoApprovePlan"] = Value::from(!auto_approve_plan);
                        let _ = this.worker.commands.try_send(Command::SetAgentConfig {
                            session: session.clone(),
                            config,
                        });
                        cx.notify();
                    });
                }
            };
            let toggle_auto_compress = {
                let observer_entity = observer_entity.clone();
                move |_: &mut Window, cx: &mut App| {
                    observer_entity.update(cx, |this, cx| {
                        this.toggle_runtime_preference("autoCompress", cx);
                    });
                }
            };
            let toggle_final_audit = {
                let observer_entity = observer_entity.clone();
                move |_: &mut Window, cx: &mut App| {
                    observer_entity.update(cx, |this, cx| {
                        this.toggle_runtime_preference("finalAudit", cx);
                    });
                }
            };
            let mut list = v_flex()
                .id("composer-quick-list")
                .w(px(280.))
                .gap(px(1.))
                .child(menu_row(
                    "composer-quick-attachment",
                    "attachment",
                    t!("add_attachment").to_string(),
                    String::new(),
                    MenuRowStyle::new(colors, false, false),
                    attach,
                ))
                .child(menu_row(
                    "composer-quick-compress",
                    "compress",
                    t!("manual_compress").to_string(),
                    String::new(),
                    MenuRowStyle::new(colors, false, false),
                    compress,
                ))
                .child(menu_section(
                    t!("quick_configuration").to_string(),
                    cx.theme().border,
                    cx.theme().muted_foreground,
                ))
                .child(menu_row(
                    "composer-quick-model-config",
                    "model",
                    t!("model_config").to_string(),
                    String::new(),
                    MenuRowStyle::new(colors, false, false),
                    open_panel(QuickPanel::Models),
                ))
                .child(menu_row(
                    "composer-quick-skills-config",
                    "skill",
                    t!("skills_config").to_string(),
                    String::new(),
                    MenuRowStyle::new(colors, false, false),
                    open_panel(QuickPanel::Skills),
                ))
                .child(menu_row(
                    "composer-quick-mcp-config",
                    "mcp",
                    t!("mcp_config").to_string(),
                    String::new(),
                    MenuRowStyle::new(colors, false, false),
                    open_panel(QuickPanel::Mcp),
                ))
                .child(menu_row(
                    "composer-quick-tools-config",
                    "hammer",
                    t!("tool_config").to_string(),
                    String::new(),
                    MenuRowStyle::new(colors, false, false),
                    open_panel(QuickPanel::Tools),
                ))
                .child(menu_row(
                    "composer-quick-personality",
                    "skill-mindmap-circle",
                    t!("execution_style").to_string(),
                    String::new(),
                    MenuRowStyle::new(colors, false, false),
                    open_panel(QuickPanel::Personality),
                ))
                .child(menu_section(
                    t!("quick_runtime").to_string(),
                    cx.theme().border,
                    cx.theme().muted_foreground,
                ))
                .child(menu_row(
                    "composer-quick-planning",
                    "skill-plan",
                    t!("planning_mode").to_string(),
                    t!("planning_mode_hint").to_string(),
                    MenuRowStyle::new(colors, planning, false).disabled(!can_plan),
                    toggle_planning,
                ));
            if planning {
                list = list.child(menu_row(
                    "composer-quick-auto-approve-plan",
                    "check-circle",
                    t!("auto_approve_plan").to_string(),
                    t!("auto_approve_plan_hint").to_string(),
                    MenuRowStyle::new(colors, auto_approve_plan, false),
                    toggle_auto_approve_plan,
                ));
            }
            list = list
                .child(menu_row(
                    "composer-quick-final-audit",
                    "check-circle",
                    t!("final_audit_label").to_string(),
                    t!("final_audit_hint").to_string(),
                    MenuRowStyle::new(colors, final_audit, false).disabled(!can_final_audit),
                    toggle_final_audit,
                ))
                .child(menu_row(
                    "composer-quick-auto-compress",
                    "compress",
                    if auto_compress {
                        t!("auto_compress_enabled").to_string()
                    } else {
                        t!("auto_compress_disabled").to_string()
                    },
                    t!("auto_compress_hint").to_string(),
                    MenuRowStyle::new(colors, auto_compress, false),
                    toggle_auto_compress,
                ))
                .child(menu_section(
                    t!("approval_level").to_string(),
                    cx.theme().border,
                    cx.theme().muted_foreground,
                ));
            for (value, icon, label, hint, danger) in [
                (
                    "default",
                    "setting",
                    t!("approval_default").to_string(),
                    t!("approval_default_hint").to_string(),
                    false,
                ),
                (
                    "smart",
                    "brain",
                    t!("approval_smart").to_string(),
                    t!("approval_smart_hint").to_string(),
                    false,
                ),
                (
                    "full",
                    "yolo",
                    t!("approval_full").to_string(),
                    t!("approval_full_hint").to_string(),
                    true,
                ),
            ] {
                let observer_entity = observer_entity.clone();
                let session = session.clone();
                let level = value.to_owned();
                list = list.child(menu_row(
                    &format!("composer-quick-approval-{value}"),
                    icon,
                    label,
                    hint,
                    MenuRowStyle::new(colors, approval == value, danger),
                    move |_: &mut Window, cx: &mut App| {
                        observer_entity.update(cx, |this, cx| {
                            if this.approval_level() != level {
                                this.submit_config(
                                    &session,
                                    "approval",
                                    Command::SetApprovalLevel {
                                        session: session.clone(),
                                        level: level.clone(),
                                    },
                                    cx,
                                );
                            }
                        });
                    },
                ));
            }
            list
        },
    )
    .into_any_element()
}

/// One quick-action row: icon, label, optional hint and the current check.
fn menu_row(
    id: &str,
    icon: &'static str,
    label: String,
    hint: String,
    style: MenuRowStyle,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let MenuRowStyle {
        checked,
        danger,
        colors,
        disabled,
    } = style;
    let OptionColors {
        primary,
        hover,
        muted,
        foreground,
    } = colors;
    let danger_color: Hsla = crate::danger().into();
    let accent = if danger { danger_color } else { primary };
    let icon_color = if checked { accent } else { muted };
    // Vue only tints the full-access row while it is the active level; an
    // inactive row keeps the ordinary text color.
    let text_color = if danger && checked {
        danger_color
    } else {
        foreground
    };
    v_flex()
        .id(SharedString::from(id.to_owned()))
        .w_full()
        .min_w_0()
        .gap(px(1.))
        .px_2()
        .py_1()
        .rounded(crate::ui::tokens::RADIUS_SMALL)
        .cursor_pointer()
        .when(checked, |row| row.bg(primary.opacity(0.12)))
        .hover(move |style| style.bg(hover))
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .items_center()
                .gap_2()
                .child(Icon::new(icon).size(13.).color(icon_color))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_size(px(12.))
                        .text_color(text_color)
                        .child(crate::ui::chrome_label(label)),
                )
                .when(checked, |row| {
                    row.child(Icon::new("check").size(12.).color(accent))
                }),
        )
        .when(!hint.is_empty(), |column| {
            column.child(
                div()
                    .w_full()
                    .min_w_0()
                    .pl(px(21.))
                    .text_size(px(11.))
                    .text_color(muted)
                    .child(hint),
            )
        })
        .when(disabled, |row| row.opacity(0.45).cursor_not_allowed())
        .when(!disabled, |row| {
            row.on_click(move |_, window, cx: &mut App| on_click(window, cx))
        })
        .into_any_element()
}

/// A titled group inside the quick actions menu.
fn menu_section(title: String, border: Hsla, muted: Hsla) -> AnyElement {
    div()
        .w_full()
        .pt_2()
        .pb_1()
        .px_2()
        .border_t_1()
        .border_color(border)
        .text_size(px(10.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(muted)
        .child(title.to_uppercase())
        .into_any_element()
}

fn menu_section_plain(title: String, muted: Hsla) -> AnyElement {
    div()
        .w_full()
        .pt_1()
        .pb_1()
        .px_2()
        .text_size(px(10.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(muted)
        .child(title.to_uppercase())
        .into_any_element()
}

/// Sandbox execution mode, mirroring Vue's badge and its three options.
fn sandbox_selector(
    observer: &Observer,
    hover: Hsla,
    muted: Hsla,
    cx: &mut Context<Observer>,
) -> AnyElement {
    let mode = observer.sandbox_mode();
    // Vue's `.sandbox-mode-badge` is an icon plus a corner-letter badge.
    let badge = match mode.as_str() {
        "auto" => "A",
        "sandbox_only" => "S",
        _ => "H",
    };
    let trigger_el = action_capsule("composer-sandbox")
        .bg(cx.theme().background)
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(Icon::new("sandbox").size(14.).color(muted))
        .child(
            div()
                .absolute()
                .top(px(-4.))
                .right(px(-4.))
                .min_w(px(15.))
                .h(px(15.))
                .px(px(2.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(cx.theme().primary)
                .text_size(px(9.))
                .line_height(px(15.))
                .text_align(TextAlign::Center)
                .font_weight(FontWeight::BOLD)
                .text_color(cx.theme().primary_foreground)
                .child(badge),
        )
        .into_any_element();
    let Some(session) = observer.selected.clone() else {
        return trigger_el;
    };
    let schemes: Vec<(String, String)> = observer
        .sandbox_schemes
        .iter()
        .filter(|scheme| scheme["enabled"].as_bool().unwrap_or(true))
        .filter_map(|scheme| {
            Some((
                scheme["id"].as_str()?.to_owned(),
                scheme["name"].as_str().unwrap_or_default().to_owned(),
            ))
        })
        .collect();
    let options = [
        (
            "auto",
            t!("sandbox_mode_auto").to_string(),
            t!("sandbox_mode_auto_hint").to_string(),
        ),
        (
            "sandbox_only",
            t!("sandbox_mode_only").to_string(),
            t!("sandbox_mode_only_hint").to_string(),
        ),
        (
            "host_only",
            t!("sandbox_mode_host").to_string(),
            String::new(),
        ),
    ];
    let mode = observer.sandbox_mode();
    let selected_index = (mode != "host_only")
        .then(|| observer.sandbox_scheme_id())
        .flatten()
        .and_then(|scheme| schemes.iter().position(|(id, _)| id == &scheme))
        .map(|index| index + options.len() + 2)
        .or_else(|| {
            options
                .iter()
                .position(|(value, _, _)| *value == mode)
                .map(|index| index + 1)
        });
    let observer_entity = cx.entity();
    MenuPopover::new(
        "composer-sandbox-popover",
        trigger_el,
        move |_: &mut PopoverState, _: &mut Window, cx: &mut Context<PopoverState>| {
            let observer = observer_entity.read(cx);
            let current = observer.sandbox_mode();
            let scheme = observer.sandbox_scheme_id();
            let pending = observer
                .config_pending
                .contains(&(session.clone(), "sandbox"));
            let mut list = v_flex()
                .id("composer-sandbox-list")
                .w(px(280.))
                .gap(px(2.))
                .when(pending, |list| list.opacity(0.55))
                .child(menu_section_plain(
                    t!("sandbox_execution_mode").to_string(),
                    cx.theme().muted_foreground,
                ));
            for (value, title, hint) in options.clone() {
                let observer_entity = observer_entity.clone();
                let session = session.clone();
                let next_scheme = if value == "host_only" {
                    None
                } else {
                    scheme
                        .clone()
                        .filter(|id| schemes.iter().any(|(candidate, _)| candidate == id))
                        .or_else(|| schemes.first().map(|(id, _)| id.clone()))
                };
                let is_selected = current == value;
                list = list.child(option_row(
                    format!("composer-sandbox-{value}"),
                    title,
                    hint,
                    is_selected,
                    move |_, _, cx| {
                        if is_selected || (value != "host_only" && next_scheme.is_none()) {
                            return;
                        }
                        observer_entity.update(cx, |this, cx| {
                            this.submit_config(
                                &session,
                                "sandbox",
                                Command::SetSandbox {
                                    session: session.clone(),
                                    mode: value.to_owned(),
                                    scheme: next_scheme.clone(),
                                },
                                cx,
                            );
                        });
                    },
                    OptionColors::new(cx),
                ));
            }
            // Vue only offers the profile list for the two sandboxed modes.
            if current != "host_only" {
                list = list.child(menu_section(
                    t!("sandbox_config").to_string(),
                    cx.theme().border,
                    cx.theme().muted_foreground,
                ));
                if schemes.is_empty() {
                    list = list.child(
                        div()
                            .px_2()
                            .py_2()
                            .text_size(px(11.))
                            .text_color(cx.theme().muted_foreground)
                            .child(t!("sandbox_empty").to_string()),
                    );
                }
                for (id, name) in schemes.clone() {
                    let observer_entity = observer_entity.clone();
                    let session = session.clone();
                    let mode = current.clone();
                    let selected = scheme.as_deref() == Some(id.as_str());
                    let refresh_id = id.clone();
                    list = list.child(
                        option_row(
                            format!("composer-sandbox-scheme-{id}"),
                            name,
                            String::new(),
                            selected,
                            move |_, _, cx| {
                                observer_entity.update(cx, |this, cx| {
                                    if !selected {
                                        this.submit_config(
                                            &session,
                                            "sandbox",
                                            Command::SetSandbox {
                                                session: session.clone(),
                                                mode: mode.clone(),
                                                scheme: Some(refresh_id.clone()),
                                            },
                                            cx,
                                        );
                                    }
                                });
                            },
                            OptionColors::new(cx),
                        )
                        .into_any_element(),
                    );
                }
                if !schemes.is_empty() {
                    let observer_entity = observer_entity.clone();
                    let session = session.clone();
                    let mode = current.clone();
                    let scheme = scheme.clone();
                    list = list.child(
                        h_flex()
                            .id("composer-sandbox-refresh")
                            .w_full()
                            .items_center()
                            .gap_2()
                            .px_2()
                            .py_1()
                            .rounded(crate::ui::tokens::RADIUS_SMALL)
                            .cursor_pointer()
                            .hover(|style| style.bg(cx.theme().muted))
                            .text_size(px(11.))
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                Icon::new("refresh")
                                    .size(12.)
                                    .color(cx.theme().muted_foreground),
                            )
                            .child(t!("sandbox_refresh").to_string())
                            .on_click(move |_, _, cx: &mut App| {
                                observer_entity.update(cx, |this, cx| {
                                    this.submit_config(
                                        &session,
                                        "sandbox",
                                        Command::SetSandbox {
                                            session: session.clone(),
                                            mode: mode.clone(),
                                            scheme: scheme.clone(),
                                        },
                                        cx,
                                    );
                                });
                            }),
                    );
                }
            }
            list
        },
    )
    .selected_index(selected_index)
    .into_any_element()
}

/// Clears the context frame, which Vue only allows between turns.
fn clear_context(
    observer: &Observer,
    hover: Hsla,
    muted: Hsla,
    cx: &mut Context<Observer>,
) -> AnyElement {
    let enabled = observer.can_clear_context() && !observer.clear_pending && !observer.options.demo;
    let color = if enabled { muted } else { muted.opacity(0.4) };
    let surface = cx.theme().background;
    action_capsule("composer-clear-context")
        .bg(surface)
        .text_color(color)
        .when(enabled, |view| {
            view.cursor_pointer().hover(move |style| style.bg(hover))
        })
        // Keep the disabled surface and glyph color stable even if a hover was
        // cached while the button was still enabled.
        .when(!enabled, |view| {
            view.cursor_default()
                .hover(move |style| style.bg(surface).text_color(color))
        })
        .child(Icon::new("clear-context").size(13.).color(color))
        .when(enabled, |view| {
            view.on_click(cx.listener(|this, _, _, cx| this.clear_context_frame(cx)))
        })
        .into_any_element()
}

/// One icon-only footer action.
fn icon_action(
    id: &'static str,
    icon: &'static str,
    color: impl Into<Hsla>,
    hover: Hsla,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .size(px(26.))
        .rounded(crate::ui::tokens::RADIUS_SMALL)
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(Icon::new(icon).size(20.).color(color))
        // A press that lands here must not also reach the box behind it.
        .on_mouse_down(MouseButton::Left, |_, window, cx| {
            window.prevent_default();
            cx.stop_propagation();
        })
        .on_click(on_click)
        .into_any_element()
}

/// Vue offers "Continue" for every resumable status.
fn continue_button(accent: Hsla, pending: bool, cx: &mut Context<Observer>) -> AnyElement {
    // Vue renders this as a small round primary button.
    let foreground = cx.theme().primary_foreground;
    h_flex()
        .id("continue-workflow")
        .flex_none()
        .items_center()
        .gap(px(4.))
        .px(px(8.))
        .py(px(4.))
        .rounded(px(9999.))
        .bg(accent)
        .when(pending, |button| button.opacity(0.75))
        .text_size(px(12.))
        .text_color(foreground)
        .child(if pending {
            Icon::new("loading").size(12.).color(foreground)
                .spin("continue-loading", crate::ui::Spin::Linear)
        } else {
            Icon::new("send").size(12.).color(foreground)
        })
        .child(t!("continue_workflow").to_string())
        .when(!pending, |button| {
            button.cursor_pointer().hover(|style| style.opacity(0.9))
                .on_click(cx.listener(|this, _, _, cx| this.continue_workflow(cx)))
        })
        .into_any_element()
}
