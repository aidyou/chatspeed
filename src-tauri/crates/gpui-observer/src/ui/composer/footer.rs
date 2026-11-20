//! Footer under the prompt box.
//!
//! Mirrors the `.input-footer` block of `src/components/workflow/WorkflowInputArea.vue`:
//! the agent, model, quick-action and sandbox selectors on the left, the
//! continue/stop/send group on the right. Every selector reads the same data the
//! Vue panel does — the control-plane agents, the runtime model catalogue and the
//! workflow's own agent config — and writes through the allowlisted workflow
//! commands, so the native window never shows a control that cannot act.

use gpui_kit::component::popover::Popover;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use rust_i18n::t;
use serde_json::Value;

use crate::ui::{trigger, Icon};
use crate::{danger, Observer};
use chatspeed_gpui_observer::types::Command;

/// The actions under the box, in Vue's left/right arrangement.
pub fn footer(observer: &Observer, cx: &mut Context<Observer>) -> AnyElement {
    let accent = cx.theme().primary;
    let hover = cx.theme().muted;
    let muted = cx.theme().muted_foreground;
    let running = observer.workflow_running();
    let can_send = !observer.composer.read(cx).value().trim().is_empty() && !observer.start_pending;

    h_flex()
        .w_full()
        .items_center()
        .gap_2()
        .px_3()
        .pb_2()
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap_1()
                .child(agent_selector(observer, hover, muted, cx))
                .child(model_selector(observer, hover, muted, cx))
                .child(quick_actions(observer, hover, muted, cx))
                .when(observer.selected.is_some(), |row| {
                    row.child(sandbox_selector(observer, hover, muted, cx))
                        .child(clear_context(observer, hover, muted, cx))
                })
                .when(observer.can_continue(), |row| {
                    row.child(continue_button(accent, hover, cx))
                }),
        )
        .when(running, |row| {
            row.child(icon_action(
                "stop-workflow",
                "stop",
                danger(),
                hover,
                cx.listener(|this, _, _, cx| this.stop_workflow(cx)),
            ))
        })
        .child(
            h_flex()
                .id("send-message")
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(26.))
                .rounded(px(6.))
                .when(can_send, |view| {
                    view.cursor_pointer().hover(|style| style.bg(hover))
                })
                .child(
                    Icon::new("send")
                        .size(16.)
                        .color(if can_send { accent } else { muted }),
                )
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

/// One footer pill: a label, an optional caret, and a hover surface.
fn pill(id: &'static str, label: String, caret: bool, hover: Hsla, color: Hsla) -> AnyElement {
    h_flex()
        .id(id)
        .flex_none()
        .items_center()
        .gap_1()
        .px_2()
        .py(px(2.))
        .rounded(px(6.))
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .text_size(px(11.))
        .text_color(color)
        .child(
            div()
                .max_w(px(160.))
                .overflow_hidden()
                .text_ellipsis()
                .child(label),
        )
        .when(caret, |row| {
            row.child(Icon::new("caret-down").size(10.).color(color))
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
        .rounded(px(6.))
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
                        .child(label),
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
    // has run without clearing its context.
    let editable = observer.can_switch_agent();
    let color = if editable { muted } else { muted.opacity(0.4) };
    let trigger_el = pill("composer-agent", label, editable, hover, color);
    let Some(session) = observer.selected.clone() else {
        return trigger_el;
    };
    if !editable {
        return trigger_el;
    }
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
    if agents.is_empty() {
        return trigger_el;
    }
    let selected = current.clone();
    let observer_entity = cx.entity();
    Popover::new("composer-agent-popover")
        .anchor(Anchor::BottomLeft)
        .trigger(trigger(trigger_el))
        .content(move |_, _, cx| {
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
                        let _ = observer_entity.update(cx, |this, _| {
                            let _ = this.worker.commands.try_send(
                                chatspeed_gpui_observer::types::Command::SetAgent {
                                    session: session.clone(),
                                    agent: target.clone(),
                                },
                            );
                        });
                    },
                    OptionColors::new(cx),
                ));
            }
            list.into_any_element()
        })
        .into_any_element()
}

/// The model pill: the active provider's models, as Vue's compact selector lists.
fn model_selector(
    observer: &Observer,
    hover: Hsla,
    muted: Hsla,
    cx: &mut Context<Observer>,
) -> AnyElement {
    let label = observer.model_label();
    let trigger_el = pill("composer-model", label, true, hover, muted);
    let Some(session) = observer.selected.clone() else {
        return trigger_el;
    };
    let config = observer.active_model_config();
    let Some(provider_id) = config["id"].as_i64() else {
        return trigger_el;
    };
    let Some(provider) = observer.active_provider().cloned() else {
        return trigger_el;
    };
    let entries: Vec<(String, String)> = provider["models"]
        .as_array()
        .map(|models| {
            models
                .iter()
                .filter_map(|model| {
                    Some((
                        model["id"].as_str()?.to_owned(),
                        model["name"]
                            .as_str()
                            .or_else(|| model["id"].as_str())
                            .unwrap_or_default()
                            .to_owned(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    if entries.is_empty() {
        return trigger_el;
    }
    let current_model = config["model"].as_str().unwrap_or_default().to_owned();
    let base_config = config.clone();
    let all_configs = observer.model_configs();
    let observer_entity = cx.entity();
    Popover::new("composer-model-popover")
        .anchor(Anchor::BottomLeft)
        .trigger(trigger(trigger_el))
        .content(move |_, _, cx| {
            let popover = cx.entity();
            let mut list = v_flex().id("composer-model-list").w(px(240.)).gap(px(2.));
            for (id, name) in entries.clone() {
                let popover = popover.clone();
                let observer_entity = observer_entity.clone();
                let session = session.clone();
                let is_selected = current_model == id;
                // Vue copies the model's own capabilities into the slot config.
                let mut next = base_config.clone();
                next["id"] = Value::from(provider_id);
                next["model"] = Value::from(id.clone());
                if let Some(model) = provider["models"].as_array().and_then(|models| {
                    models
                        .iter()
                        .find(|model| model["id"].as_str() == Some(id.as_str()))
                }) {
                    for key in [
                        "functionCall",
                        "thinking",
                        "temperature",
                        "contextSize",
                        "maxTokens",
                    ] {
                        if !model[key].is_null() {
                            next[key] = model[key].clone();
                        }
                    }
                }
                let mut configs = all_configs.clone();
                configs["act"] = next;
                list = list.child(option_row(
                    format!("composer-model-{id}"),
                    name,
                    String::new(),
                    is_selected,
                    move |_, window, cx| {
                        popover.update(cx, |state, cx| state.dismiss(window, cx));
                        let _ = observer_entity.update(cx, |this, _| {
                            let _ = this.worker.commands.try_send(
                                chatspeed_gpui_observer::types::Command::SetModelConfig {
                                    session: session.clone(),
                                    configs: configs.clone(),
                                },
                            );
                        });
                    },
                    OptionColors::new(cx),
                ));
            }
            list.into_any_element()
        })
        .into_any_element()
}

/// Vue's `+` quick actions: the compress command, the runtime switches and the
/// approval level, grouped under the same section titles the web menu uses.
fn quick_actions(
    observer: &Observer,
    hover: Hsla,
    muted: Hsla,
    cx: &mut Context<Observer>,
) -> AnyElement {
    let trigger_el = div()
        .id("composer-quick-actions")
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .size(px(22.))
        .rounded(px(6.))
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(Icon::new("add").size(13.).color(muted))
        .into_any_element();
    let Some(session) = observer.selected.clone() else {
        return trigger_el;
    };
    let auto_compress = observer.auto_compress_enabled();
    let final_audit = observer.final_audit_enabled();
    let approval = observer.approval_level();
    let observer_entity = cx.entity();
    Popover::new("composer-quick-popover")
        .anchor(Anchor::BottomLeft)
        .trigger(trigger(trigger_el))
        .content(move |_, _, cx| {
            let popover = cx.entity();
            let observer_entity = observer_entity.clone();
            let session = session.clone();
            // Every entry acts on a fresh copy of the runtime state, so the
            // menu closes after the command like Vue's dropdown does.
            let compress = {
                let popover = popover.clone();
                let observer_entity = observer_entity.clone();
                move |window: &mut Window, cx: &mut App| {
                    popover.update(cx, |state, cx| state.dismiss(window, cx));
                    let _ = observer_entity.update(cx, |this, cx| this.manual_compress(cx));
                }
            };
            let toggle_auto_compress = {
                let popover = popover.clone();
                let observer_entity = observer_entity.clone();
                let session = session.clone();
                move |window: &mut Window, cx: &mut App| {
                    popover.update(cx, |state, cx| state.dismiss(window, cx));
                    let _ = observer_entity.update(cx, |this, _| {
                        let _ = this.worker.commands.try_send(Command::SetAutoCompress {
                            session: session.clone(),
                            enabled: !auto_compress,
                        });
                    });
                }
            };
            let toggle_final_audit = {
                let popover = popover.clone();
                let observer_entity = observer_entity.clone();
                let session = session.clone();
                move |window: &mut Window, cx: &mut App| {
                    popover.update(cx, |state, cx| state.dismiss(window, cx));
                    let _ = observer_entity.update(cx, |this, _| {
                        let _ = this.worker.commands.try_send(Command::SetFinalAudit {
                            session: session.clone(),
                            enabled: !final_audit,
                        });
                    });
                }
            };
            let mut list = v_flex()
                .id("composer-quick-list")
                .w(px(280.))
                .gap(px(1.))
                .child(menu_row(
                    "composer-quick-compress",
                    "compress",
                    t!("manual_compress").to_string(),
                    String::new(),
                    false,
                    OptionColors::new(cx),
                    compress,
                ))
                .child(menu_section(
                    t!("quick_runtime").to_string(),
                    cx.theme().border,
                    cx.theme().muted_foreground,
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
                    auto_compress,
                    OptionColors::new(cx),
                    toggle_auto_compress,
                ))
                .child(menu_row(
                    "composer-quick-final-audit",
                    "check-circle",
                    t!("final_audit_label").to_string(),
                    t!("final_audit_hint").to_string(),
                    final_audit,
                    OptionColors::new(cx),
                    toggle_final_audit,
                ))
                .child(menu_section(
                    t!("approval_level").to_string(),
                    cx.theme().border,
                    cx.theme().muted_foreground,
                ));
            for (value, icon, label, hint) in [
                (
                    "default",
                    "setting",
                    t!("approval_default").to_string(),
                    t!("approval_default_hint").to_string(),
                ),
                (
                    "smart",
                    "brain",
                    t!("approval_smart").to_string(),
                    t!("approval_smart_hint").to_string(),
                ),
                (
                    "full",
                    "yolo",
                    t!("approval_full").to_string(),
                    t!("approval_full_hint").to_string(),
                ),
            ] {
                let popover = popover.clone();
                let observer_entity = observer_entity.clone();
                let session = session.clone();
                let level = value.to_owned();
                list = list.child(menu_row(
                    &format!("composer-quick-approval-{value}"),
                    icon,
                    label,
                    hint,
                    approval == value,
                    OptionColors::new(cx),
                    move |window: &mut Window, cx: &mut App| {
                        popover.update(cx, |state, cx| state.dismiss(window, cx));
                        let _ = observer_entity.update(cx, |this, _| {
                            let _ = this.worker.commands.try_send(Command::SetApprovalLevel {
                                session: session.clone(),
                                level: level.clone(),
                            });
                        });
                    },
                ));
            }
            list.into_any_element()
        })
        .into_any_element()
}

/// One quick-action row: icon, label, optional hint and the current check.
fn menu_row(
    id: &str,
    icon: &'static str,
    label: String,
    hint: String,
    checked: bool,
    colors: OptionColors,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let OptionColors {
        primary,
        hover,
        muted,
        foreground,
    } = colors;
    v_flex()
        .id(SharedString::from(id.to_owned()))
        .w_full()
        .min_w_0()
        .gap(px(1.))
        .px_2()
        .py_1()
        .rounded(px(6.))
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .items_center()
                .gap_2()
                .child(Icon::new(icon).size(13.).color(muted))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_size(px(12.))
                        .text_color(foreground)
                        .child(label),
                )
                .when(checked, |row| {
                    row.child(Icon::new("check").size(12.).color(primary))
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
        .on_click(move |_, window, cx: &mut App| on_click(window, cx))
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

/// Sandbox execution mode, mirroring Vue's badge and its three options.
fn sandbox_selector(
    observer: &Observer,
    hover: Hsla,
    muted: Hsla,
    cx: &mut Context<Observer>,
) -> AnyElement {
    let mode = observer.sandbox_mode();
    let label = match mode.as_str() {
        "auto" => t!("sandbox_mode_auto").to_string(),
        "sandbox_only" => t!("sandbox_mode_only").to_string(),
        _ => t!("sandbox_mode_host").to_string(),
    };
    let color = if mode == "host_only" {
        muted
    } else {
        cx.theme().primary
    };
    let trigger_el = h_flex()
        .id("composer-sandbox")
        .flex_none()
        .items_center()
        .gap_1()
        .px_2()
        .py(px(2.))
        .rounded(px(6.))
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .text_size(px(11.))
        .text_color(color)
        .child(Icon::new("sandbox").size(13.).color(color))
        .child(div().child(label))
        .into_any_element();
    let Some(session) = observer.selected.clone() else {
        return trigger_el;
    };
    let scheme = observer.sandbox_scheme_id().or_else(|| {
        observer
            .sandbox_schemes
            .iter()
            .find(|scheme| scheme["enabled"].as_bool().unwrap_or(true))
            .and_then(|scheme| scheme["id"].as_str())
            .map(str::to_owned)
    });
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
    let current = mode.clone();
    let observer_entity = cx.entity();
    Popover::new("composer-sandbox-popover")
        .anchor(Anchor::BottomLeft)
        .trigger(trigger(trigger_el))
        .content(move |_, _, cx| {
            let popover = cx.entity();
            let mut list = v_flex().id("composer-sandbox-list").w(px(260.)).gap(px(2.));
            for (value, title, hint) in options.clone() {
                let popover = popover.clone();
                let observer_entity = observer_entity.clone();
                let session = session.clone();
                let scheme = scheme.clone();
                let is_selected = current == value;
                list = list.child(option_row(
                    format!("composer-sandbox-{value}"),
                    title,
                    hint,
                    is_selected,
                    move |_, window, cx| {
                        popover.update(cx, |state, cx| state.dismiss(window, cx));
                        let _ = observer_entity.update(cx, |this, _| {
                            let _ = this.worker.commands.try_send(
                                chatspeed_gpui_observer::types::Command::SetSandbox {
                                    session: session.clone(),
                                    mode: value.to_owned(),
                                    scheme: if value == "host_only" {
                                        None
                                    } else {
                                        scheme.clone()
                                    },
                                },
                            );
                        });
                    },
                    OptionColors::new(cx),
                ));
            }
            list.into_any_element()
        })
        .into_any_element()
}

/// Clears the context frame, which Vue only allows between turns.
fn clear_context(
    observer: &Observer,
    hover: Hsla,
    muted: Hsla,
    cx: &mut Context<Observer>,
) -> AnyElement {
    let enabled = observer.selected.is_some() && !observer.workflow_running();
    let color = if enabled { muted } else { muted.opacity(0.4) };
    div()
        .id("composer-clear-context")
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .size(px(22.))
        .rounded(px(6.))
        .when(enabled, |view| {
            view.cursor_pointer().hover(move |style| style.bg(hover))
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
        .rounded(px(6.))
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(Icon::new(icon).size(14.).color(color))
        // A press that lands here must not also reach the box behind it.
        .on_mouse_down(MouseButton::Left, |_, window, cx| {
            window.prevent_default();
            cx.stop_propagation();
        })
        .on_click(on_click)
        .into_any_element()
}

/// Vue offers "Continue" for every resumable status.
fn continue_button(accent: Hsla, hover: Hsla, cx: &mut Context<Observer>) -> AnyElement {
    h_flex()
        .id("continue-workflow")
        .flex_none()
        .items_center()
        .gap_1()
        .px_2()
        .py(px(2.))
        .rounded(px(6.))
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .text_size(px(11.))
        .text_color(accent)
        .child(Icon::new("send").size(11.).color(accent))
        .child(t!("continue_workflow").to_string())
        .on_click(cx.listener(|this, _, _, cx| this.continue_workflow(cx)))
        .into_any_element()
}
