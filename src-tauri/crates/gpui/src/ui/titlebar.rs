//! Custom titlebar: shell actions, the authorized-path chip and the window
//! controls.
//!
//! Mirrors `src/components/window/Titlebar.vue` plus the slots the workflow view
//! fills in: the menu, the sidebar toggle, the approval queue and the sound menu
//! on the left, the authorized-path chip in the middle, and the workflow actions
//! on the right. The bar carries no session title, matching Vue.
//!
//! The bar is drawn here rather than by `gpui_kit::component::TitleBar` because
//! that component always appends its own window controls: their hover fill is a
//! rectangle the full height of the bar, which paints over the frame's rounded
//! top-right corner.

use chatspeed_gpui::chimes::{Chime, DecisionKind, PendingDecision};
use chatspeed_runtime_client::desktop_menu::Action;
use gpui_kit::base::InteractiveElementExt;
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme, Icon as KitIcon, IconName, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use rust_i18n::t;

use crate::ui::{icon_button, surfaces, trigger, Icon, TITLEBAR_HEIGHT};
use crate::Observer;

/// Width of a window control button, matching the platform convention.
const CONTROL_WIDTH: f32 = 34.;

/// Distance from the window edge to the first and last titlebar control.
///
/// Both groups keep that same inset, so the bar looks the same whether the
/// window controls sit on the right or the platform keeps them on the left
/// (macOS traffic lights).
const SIDE_INSET: f32 = 20.;
/// macOS hands the leading edge to the native traffic lights, so the first icon
/// clears them instead of sitting at [`SIDE_INSET`].
const MACOS_TRAFFIC_LIGHTS_INSET: f32 = 78.;

/// Same slot order as the composer model dialog, which Vue opens on "act".
const AUTOMATION_MODEL_SLOTS: [&str; 5] = crate::ui::composer::model_config::MODEL_SLOTS;

fn open_automation(observer: &mut Observer, window: &mut Window, cx: &mut Context<Observer>) {
    use chatspeed_gpui::components::{
        button::{Button, ButtonVariants},
        input::{Input, InputState, Textarea, TextareaState},
    };
    use chatspeed_gpui::components::dialog::WindowExt;
    if observer.automation_pending || window.has_active_dialog(cx) {
        return;
    }
    let primary_agents: Vec<(String, String)> = observer
        .agents
        .iter()
        .filter(|entry| {
            entry["role"].as_str() == Some("primary")
                && entry["disabled"] != serde_json::json!(true)
        })
        .filter_map(|entry| {
            Some((
                entry["id"].as_str()?.to_owned(),
                entry["name"].as_str()?.to_owned(),
            ))
        })
        .collect();
    let Some(agent) = observer
        .current_agent_id()
        .filter(|id| primary_agents.iter().any(|(primary, _)| primary == id))
        .or_else(|| primary_agents.first().map(|(id, _)| id.clone()))
    else {
        observer.show_toast(
            crate::ToastKind::Warning,
            t!("automation_agent_required").to_string(),
            cx,
        );
        return;
    };
    let agent_label = primary_agents
        .iter()
        .find(|(id, _)| id == &agent)
        .map(|(_, name)| name.clone())
        .unwrap_or_else(|| agent.clone());
    let title = cx.new(|cx| InputState::new(window, cx));
    let prompt = cx.new(|cx| TextareaState::new(window, cx));
    let interval = cx.new(|cx| {
        let mut input = InputState::new(window, cx).placeholder("5");
        input.set_value("5", window, cx);
        input
    });
    let daily_times = std::rc::Rc::new(std::cell::RefCell::new(vec![cx.new(|cx| {
        let mut state = gpui_kit::component::time_field::TimeFieldState::new(window, cx);
        if let Ok(time) = "09:00".parse() {
            state.set_time(time, window, cx);
        }
        state
    })]));
    let prompt_file = cx.new(|cx| InputState::new(window, cx));
    use gpui_kit::component::{
        date_picker::{DatePicker, DatePickerState},
        time_field::TimePrecision,
    };
    let effective_range = cx.new(|cx| DatePickerState::range(window, cx).date_format("%Y-%m-%d"));
    let once_date = cx.new(|cx| {
        DatePickerState::new(window, cx)
            .time_precision(TimePrecision::Minute)
            .date_format("%Y-%m-%d %H:%M")
    });
    let allowed_paths = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let shell = cx.new(|cx| {
        TextareaState::new(window, cx)
            .placeholder(t!("automation_shell_command_placeholder").to_string())
    });
    // The Vue editor is a two-tab form; the native dialog keeps the same split
    // and the same basic/advanced field order.
    let tab = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let frequency = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let continuous = std::rc::Rc::new(std::cell::Cell::new(false));
    let self_review = std::rc::Rc::new(std::cell::Cell::new(false));
    let enabled = std::rc::Rc::new(std::cell::Cell::new(true));
    // Bits 1..=7 are Monday..=Sunday, matching the backend's weekday numbers.
    let weekdays = std::rc::Rc::new(std::cell::Cell::new(0b1111_1110u32));
    let agents = primary_agents;
    let selected_agent = std::rc::Rc::new(std::cell::RefCell::new(agent.clone()));
    let selected_agent_label = std::rc::Rc::new(std::cell::RefCell::new(agent_label.clone()));
    let agent_config = std::rc::Rc::new(std::cell::RefCell::new(observer.agent_config()));
    let act_config = agent_config.borrow()["models"]["act"].clone();
    let selected_model = std::rc::Rc::new(std::cell::RefCell::new(act_config));
    let selected_model_label = std::rc::Rc::new(std::cell::RefCell::new(observer.model_label()));
    // The automation model button opens the same slot/provider/model
    // configuration as the composer's "+" menu, but edits a local draft so the
    // dialog can be cancelled without touching the live workflow.
    let mut model_editor = agent_config.borrow()["models"].clone();
    if !model_editor.is_object() {
        model_editor = serde_json::json!({});
    }
    for slot in AUTOMATION_MODEL_SLOTS {
        if !model_editor[slot].is_object() {
            model_editor[slot] = serde_json::json!({
                "id": 0,
                "model": "",
                "temperature": -0.1,
                "contextSize": 128000,
                "maxTokens": 0,
            });
        }
    }
    let model_editor = std::rc::Rc::new(std::cell::RefCell::new(model_editor));
    let model_catalog = observer.models.clone();
    let proxy_targets = observer.proxy_targets.clone();
    let model_config = crate::ui::composer::model_config::ModelConfigState::new(
        model_editor.borrow().clone(),
        "act",
        model_catalog.clone(),
        proxy_targets.clone(),
        window,
        cx,
    );
    let owner = cx.entity().downgrade();
    window.open_dialog(cx, move |dialog, window, cx| {
        use crate::ui::menu_popover::MenuPopover;
        use gpui_kit::component::popover::PopoverState;
        use gpui_kit::component::{switch::Switch, Disableable, Sizable};
        let agents = agents.clone();
        let selected_agent = selected_agent.clone();
        let selected_agent_label = selected_agent_label.clone();
        let selected_model = selected_model.clone();
        let selected_model_label = selected_model_label.clone();
        let model_editor = model_editor.clone();
        let model_config = model_config.clone();
        let model_catalog = model_catalog.clone();
        let proxy_targets = proxy_targets.clone();
        let agent_config_state = agent_config.clone();
        let daily_times_state = daily_times.clone();
        let allowed_paths_state = allowed_paths.clone();
        let (title_state, prompt_state, interval_state) =
            (title.clone(), prompt.clone(), interval.clone());
        let (prompt_file_state, shell_state) = (prompt_file.clone(), shell.clone());
        let once_date_state = once_date.clone();
        let (agent_for_submit, owner) = (selected_agent.clone(), owner.clone());
        let (tab_state, frequency_state, weekdays_state) =
            (tab.clone(), frequency.clone(), weekdays.clone());
        let (continuous_state, self_review_state, enabled_state) =
            (continuous.clone(), self_review.clone(), enabled.clone());
        let (submit_continuous, submit_self_review, submit_enabled) = (
            continuous_state.clone(),
            self_review_state.clone(),
            enabled_state.clone(),
        );
        let (submit_frequency, submit_weekdays) =
            (frequency_state.clone(), weekdays_state.clone());
        let submit_agent_config = agent_config_state.clone();
        let submit_model = selected_model.clone();
        let submit_daily_times = daily_times_state.clone();
        let submit_allowed_paths = allowed_paths_state.clone();
        let submit_title_state = title_state.clone();
        let submit_prompt_state = prompt_state.clone();
        let submit_prompt_file_state = prompt_file_state.clone();
        let submit_interval_state = interval_state.clone();
        let submit_shell_state = shell_state.clone();
        let submit_effective_range = effective_range.clone();
        let submit_once_date_state = once_date_state.clone();
        let form_button = |id: SharedString| {
            Button::new(id).h(px(32.)).pt(px(0.)).pb(px(2.)).line_height(px(16.))
        };
        let submit = form_button("create-automation-submit".into())
            .primary()
            .label(t!("create_automation").to_string())
            .h(px(32.))
            .px(px(16.))
            .text_size(px(14.))
            .on_click(move |_, window, cx| {
                let name = submit_title_state.read(cx).value().trim().to_owned();
                let prompt = submit_prompt_state.read(cx).value().trim().to_owned();
                let prompt_file = submit_prompt_file_state.read(cx).value().trim().to_owned();
                let shell_command = submit_shell_state.read(cx).value().trim().to_owned();
                let schedule_value = match submit_frequency.get() {
                    0 => submit_daily_times
                        .borrow()
                        .iter()
                        .map(|state| state.read(cx).time().format("%H:%M").to_string())
                        .collect::<Vec<_>>()
                        .join(","),
                    1 => format!(
                        "{}m",
                        submit_interval_state
                            .read(cx)
                            .value()
                            .trim()
                    ),
                    _ => submit_once_date_state
                        .read(cx)
                        .date_time()
                        .format("%Y-%m-%d %H:%M")
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                };
                let mut schedule = parse_automation_schedule(&schedule_value);
                let allowed_paths: Vec<String> =
                    submit_allowed_paths.borrow().clone();
                let accepted = owner.update(cx, |this, cx| {
                    if this.automation_pending { return false; }
                    let Some((kind, mut schedule_config)) = schedule.take().filter(|_| {
                        !name.is_empty() && (!prompt.is_empty() || !prompt_file.is_empty())
                    }) else {
                        this.show_toast(crate::ToastKind::Warning, t!("automation_required_fields").to_string(), cx);
                        return false;
                    };
                    if submit_frequency.get() != 2 {
                        let days: Vec<u32> = (1..=7)
                            .filter(|day| submit_weekdays.get() & (1 << day) != 0)
                            .collect();
                        let range = submit_effective_range.read(cx).date_time();
                        let (start, end) = (range.start(), range.end());
                        if days.is_empty() || !valid_automation_date_range(start, end) {
                            this.show_toast(crate::ToastKind::Warning, t!("automation_required_fields").to_string(), cx);
                            return false;
                        }
                        schedule_config["weekdays"] = serde_json::json!(days);
                        schedule_config["start_date"] = serde_json::json!(start.map(|date| date.format("%Y-%m-%d").to_string()));
                        schedule_config["end_date"] = serde_json::json!(end.map(|date| date.format("%Y-%m-%d").to_string()));
                        if kind == "interval" {
                            let anchor = submit_daily_times.borrow().first().map(|state| {
                                state.read(cx).time().format("%H:%M").to_string()
                            });
                            schedule_config["anchor_time"] = serde_json::json!(anchor.as_deref().unwrap_or("09:00"));
                        }
                    }
                    let shell_config = if shell_command.is_empty() {
                        serde_json::Value::Null
                    } else {
                        serde_json::json!({ "command": shell_command })
                    };
                    let mut submit_config = submit_agent_config.borrow().clone();
                    submit_config["models"]["act"] = submit_model.borrow().clone();
                    let spec = serde_json::json!({
                        "title": name,
                        "prompt": prompt,
                        "prompt_file_path": if prompt_file.is_empty() {
                            serde_json::Value::Null
                        } else {
                            serde_json::json!(prompt_file)
                        },
                        "agent_id": agent_for_submit.borrow().clone(),
                        "agent_config": submit_config,
                        "allowed_paths": allowed_paths,
                        "shell_config": shell_config,
                        "schedule_kind": kind,
                        "schedule_config": schedule_config,
                        "continuous_context": kind != "once" && submit_continuous.get(),
                        "self_review": submit_self_review.get(),
                        "enabled": submit_enabled.get(),
                    });
                    if this.worker.commands.try_send(chatspeed_gpui::types::Command::CreateAutomation(spec)).is_err() {
                        this.show_toast(crate::ToastKind::Error, t!("automation_create_failed").to_string(), cx);
                        return false;
                    }
                    this.automation_pending = true;
                    true
                }).unwrap_or(false);
                if accepted { window.close_dialog(cx); }
            });
        let border = cx.theme().border;
        let primary = cx.theme().primary;
        let muted = cx.theme().muted_foreground;
        let tabs = div().w_full().mt(px(16.)).child(crate::ui::tabs::underline_tabs(
            "automation-tab",
            vec![
                t!("automation_tab_basic").to_string(),
                t!("automation_tab_advanced").to_string(),
            ],
            tab_state.get(),
            {
                let tab_state = tab_state.clone();
                move |index, window, _| {
                    tab_state.set(index);
                    window.refresh();
                }
            },
            cx,
        ));
        let label_color = cx.theme().foreground;
        let field = |label: String, control: AnyElement| {
            let (required, label) = match label.strip_prefix('*') {
                Some(label) => (true, label.to_owned()),
                None => (false, label),
            };
            h_flex()
                .w_full()
                .min_w_0()
                .items_start()
                .gap(super::tokens::SPACE_12)
                .child(
                    h_flex()
                        .w(px(120.))
                        .min_h(px(32.))
                        .flex_none()
                        .items_center()
                        .justify_end()
                        .gap(px(2.))
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(label_color)
                        .when(required, |row| {
                            row.child(div().text_color(cx.theme().danger).child("*"))
                        })
                        .child(label),
                )
                .child(div().flex_1().min_w_0().child(control))
                .into_any_element()
        };
        let pick_paths = |state: Entity<InputState>,
                          directories: bool,
                          multiple: bool,
                          append: bool,
                          window: &mut Window,
                          cx: &mut App| {
            let receiver = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
                files: !directories,
                directories,
                multiple,
                prompt: None,
            });
            let state_for_pick = state.clone();
            let window_handle = window.window_handle();
            cx.spawn(async move |cx| {
                let Ok(Ok(Some(paths))) = receiver.await else {
                    return;
                };
                if paths.is_empty() {
                    return;
                }
                let selected = paths
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                let text = if append {
                    let current = state_for_pick.read_with(cx, |state, _| state.value());
                    let current = current.trim();
                    if current.is_empty() {
                        selected
                    } else {
                        format!("{current}, {selected}")
                    }
                } else {
                    selected
                };
                let _ = cx.update(|cx| {
                    let _ = window_handle.update(cx, |_, window, cx| {
                        state_for_pick.update(cx, |state, cx| {
                            state.set_value(text, window, cx);
                        });
                        window.refresh();
                    });
                });
            })
            .detach();
        };
        let pick_directories = |paths: std::rc::Rc<
                                   std::cell::RefCell<Vec<String>>,
                               >,
                               window: &mut Window,
                               cx: &mut App| {
            let receiver = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
                files: false,
                directories: true,
                multiple: true,
                prompt: None,
            });
            let window_handle = window.window_handle();
            cx.spawn(async move |cx| {
                let Ok(Ok(Some(selected))) = receiver.await else {
                    return;
                };
                let _ = cx.update(|cx| {
                    let _ = window_handle.update(cx, |_, window, _| {
                        let mut paths = paths.borrow_mut();
                        for path in selected {
                            let path = path.display().to_string();
                            if !paths.contains(&path) {
                                paths.push(path);
                            }
                        }
                        window.refresh();
                    });
                });
            })
            .detach();
        };
        let body = if tab_state.get() == 0 {
            v_flex()
                .gap(super::tokens::SPACE_12)
                .child(field(
                    format!("*{}", t!("automation_task_title")),
                    Input::new(&title)
                        .w_full()
                        .h(px(36.))
                        .min_h(px(36.))
                        .py(px(4.))
                        .px(px(8.))
                        .bg(crate::ui::field_bg(cx))
                        .into_any_element(),
                ))
                .child(field(
                    format!(
                        "{}{}",
                        if prompt_file_state.read(cx).value().trim().is_empty() {
                            "*"
                        } else {
                            ""
                        },
                        t!("automation_prompt")
                    ),
                    Textarea::new(&prompt)
                        .w_full()
                        .min_h(px(120.))
                        .bg(crate::ui::field_bg(cx))
                        .h(px(120.))
                        .into_any_element(),
                ))
                .child(field(
                    format!(
                        "{}{}",
                        if prompt_state.read(cx).value().trim().is_empty() {
                            "*"
                        } else {
                            ""
                        },
                        t!("automation_prompt_file")
                    ),
                    v_flex()
                        .w_full()
                        .min_w_0()
                        .gap(px(8.))
                        .child(
                            h_flex()
                                .w_full()
                                .min_w_0()
                                .gap(px(8.))
                                .child(
                                    div().flex_1().min_w_0().child(
                                        Input::new(&prompt_file)
                                            .readonly(true)
                                            .w_full()
                                            .py(px(4.))
                                            .px(px(8.))
                                            .min_h(px(32.))
                                            .h(px(32.))
                                            .bg(crate::ui::field_bg(cx))
                                            .when(
                                                !prompt_file_state
                                                    .read(cx)
                                                    .value()
                                                    .trim()
                                                    .is_empty(),
                                                |input| {
                                                    let state = prompt_file_state.clone();
                                                    input.suffix(
                                                        div()
                                                            .id("automation-prompt-file-clear")
                                                            .flex_none()
                                                            .flex()
                                                            .items_center()
                                                            .justify_center()
                                                            .size(px(14.))
                                                            .rounded_full()
                                                            .bg(muted.opacity(0.65))
                                                            .cursor_pointer()
                                                            .hover(|style| {
                                                                style.opacity(0.8)
                                                            })
                                                            .child(
                                                                Icon::new("close")
                                                                    .size(8.)
                                                                    .color(gpui_kit::white()),
                                                            )
                                                            .on_click(move |_, window, cx| {
                                                                state.update(cx, |state, cx| {
                                                                    state.set_value("", window, cx);
                                                                });
                                                                window.refresh();
                                                            }),
                                                    )
                                                },
                                            ),
                                    ),
                                )
                                .child(
                                    form_button("automation-prompt-file-pick".into())
                                        .label(t!("automation_select_file").to_string())
                                        .h(px(32.))
                                        .text_size(px(12.))
                                        .on_click({
                                            let state = prompt_file.clone();
                                            move |_, window, cx| {
                                                pick_paths(
                                                    state.clone(),
                                                    false,
                                                    false,
                                                    false,
                                                    window,
                                                    cx,
                                                );
                                            }
                                        }),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(muted)
                                .child(t!("automation_prompt_file_hint").to_string()),
                        )
                        .into_any_element(),
                ))
                .child(field(
                    format!("*{}", t!("automation_agent")),
                    MenuPopover::new(
                        "automation-agent-menu",
                        h_flex()
                            .id("automation-agent-trigger")
                            .w_full()
                            .h(px(32.))
                            .min_w_0()
                            .items_center()
                            .justify_between()
                            .gap(px(8.))
                            .px(px(8.))
                            .rounded(super::tokens::RADIUS_MEDIUM)
                            .border_1()
                            .border_color(border)
                            .bg(crate::ui::field_bg(cx))
                            .cursor_pointer()
                            .child(
                                div()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .text_size(px(12.))
                                    .text_color(primary)
                                    .child(selected_agent_label.borrow().clone()),
                            )
                            .child(Icon::new("caret-down").size(10.).color(muted))
                            .into_any_element(),
                        {
                            let agents = agents.clone();
                            let selected_agent = selected_agent.clone();
                            let selected_agent_label = selected_agent_label.clone();
                            move |_: &mut PopoverState,
                                  _: &mut Window,
                                  cx: &mut Context<PopoverState>| {
                                let popover = cx.entity();
                                let mut list = v_flex()
                                    .id("automation-agent-list")
                                    .w(px(240.))
                                    .gap(px(2.));
                                for (id, name) in agents.clone() {
                                    let popover = popover.clone();
                                    let selected_agent = selected_agent.clone();
                                    let selected_agent_label = selected_agent_label.clone();
                                    let is_selected = *selected_agent.borrow() == id;
                                    let id_for_click = id.clone();
                                    let name_for_click = name.clone();
                                    list = list.child(
                                        div()
                                            .id(SharedString::from(format!(
                                                "automation-agent-{id}"
                                            )))
                                            .w_full()
                                            .px_2()
                                            .py_1()
                                            .rounded(super::tokens::RADIUS_SMALL)
                                            .cursor_pointer()
                                            .text_size(px(12.))
                                            .when(is_selected, |row| {
                                                row.bg(cx.theme().primary.opacity(0.12))
                                            })
                                            .hover(|style| style.bg(cx.theme().muted))
                                            .child(name)
                                            .on_click(move |_, window, cx| {
                                                popover.update(cx, |state, cx| {
                                                    state.dismiss(window, cx)
                                                });
                                                *selected_agent.borrow_mut() =
                                                    id_for_click.clone();
                                                *selected_agent_label.borrow_mut() =
                                                    name_for_click.clone();
                                                window.refresh();
                                            }),
                                    );
                                }
                                list
                            }
                        },
                    )
                    .into_any_element(),
                ))
                .child(field(
                    t!("automation_model").to_string(),
                    MenuPopover::new(
                        "automation-model-menu",
                        h_flex()
                            .id("automation-model-trigger")
                            .w_full()
                            .h(px(32.))
                            .min_w_0()
                            .items_center()
                            .justify_between()
                            .gap(px(8.))
                            .px(px(8.))
                            .rounded(super::tokens::RADIUS_MEDIUM)
                            .border_1()
                            .border_color(border)
                            .bg(crate::ui::field_bg(cx))
                            .cursor_pointer()
                            .on_click({
                                let editor = model_editor.clone();
                                let config = agent_config_state.clone();
                                let state = model_config.clone();
                                move |_, window, cx| {
                                    let mut editor = editor.borrow_mut();
                                    *editor = config.borrow()["models"].clone();
                                    if !editor.is_object() {
                                        *editor = serde_json::json!({});
                                    }
                                    for slot_name in AUTOMATION_MODEL_SLOTS {
                                        if !editor[slot_name].is_object() {
                                            editor[slot_name] = serde_json::json!({
                                                "id": 0,
                                                "model": "",
                                                "temperature": -0.1,
                                                "contextSize": 128000,
                                                "maxTokens": 0,
                                            });
                                        }
                                    }
                                    let configs = editor.clone();
                                    drop(editor);
                                    state.update(cx, |this, cx| {
                                        this.configs = configs;
                                        this.set_slot("act", window, cx);
                                        cx.notify();
                                    });
                                }
                            })
                            .child(
                                h_flex()
                                    .min_w_0()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(Icon::new("setting").size(13.).color(primary))
                                    .child(
                                        div()
                                            .min_w_0()
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .text_size(px(12.))
                                            .child(selected_model_label.borrow().clone()),
                                    ),
                            )
                            .child(Icon::new("caret-down").size(10.).color(muted))
                            .into_any_element(),
                        {
                            let state = model_config.clone();
                            let editor = model_editor.clone();
                            let config_state = agent_config_state.clone();
                            let selected_model = selected_model.clone();
                            let selected_model_label = selected_model_label.clone();
                            let catalog = model_catalog.clone();
                            let targets = proxy_targets.clone();
                            move |_: &mut PopoverState,
                                  _: &mut Window,
                                  cx: &mut Context<PopoverState>| {
                                let popover = cx.entity();
                                let cancel_popover = popover.clone();
                                let save_popover = popover.clone();
                                let save_state = state.clone();
                                let save_editor = editor.clone();
                                let save_config_state = config_state.clone();
                                let save_selected_model = selected_model.clone();
                                let save_selected_model_label = selected_model_label.clone();
                                let save_catalog = catalog.clone();
                                let save_targets = targets.clone();
                                let footer =
                                    crate::ui::composer::model_config::model_config_footer(
                                        "automation-model-config",
                                        true,
                                        move |_, window, cx| {
                                            cancel_popover.update(cx, |state, cx| {
                                                state.dismiss(window, cx)
                                            });
                                        },
                                        move |_, window, cx| {
                                            let models = save_state.read(cx).configs.clone();
                                            *save_editor.borrow_mut() = models.clone();
                                            save_config_state.borrow_mut()["models"] =
                                                models.clone();
                                            let act = models["act"].clone();
                                            let label =
                                                crate::ui::composer::footer::model_choices_from(
                                                    &save_catalog, &save_targets, &act,
                                                )
                                                .1
                                                .into_iter()
                                                .find(|choice| {
                                                    choice.model
                                                        == act["model"]
                                                            .as_str()
                                                            .unwrap_or_default()
                                                })
                                                .map(|choice| choice.name)
                                                .or_else(|| {
                                                    act["model"]
                                                        .as_str()
                                                        .filter(|model| !model.is_empty())
                                                        .map(str::to_owned)
                                                })
                                                .unwrap_or_else(|| {
                                                    t!("model_placeholder").to_string()
                                                });
                                            *save_selected_model.borrow_mut() = act;
                                            *save_selected_model_label.borrow_mut() = label;
                                            save_popover.update(cx, |state, cx| {
                                                state.dismiss(window, cx)
                                            });
                                            window.refresh();
                                        },
                                        cx,
                                    );
                                div().id("automation-model-config-panel").w(px(480.)).min_w_0().child(
                                    crate::ui::composer::model_config::model_config_panel(
                                        &state, footer, cx,
                                    ),
                                )
                            }
                        },
                    )
                    .into_any_element(),
                ))
                .child(field(
                    format!("*{}", t!("automation_frequency")),
                    crate::ui::tabs::capsule_segments(
                        "automation-frequency",
                        vec![
                            ("daily", t!("automation_frequency_daily").to_string()),
                            ("interval", t!("automation_frequency_interval").to_string()),
                            ("once", t!("automation_frequency_once").to_string()),
                        ],
                        ["daily", "interval", "once"][frequency_state.get()],
                        {
                            let frequency_state = frequency_state.clone();
                            let continuous_state = continuous_state.clone();
                            move |value, window, _| {
                                let index = match value {
                                    "daily" => 0,
                                    "interval" => 1,
                                    "once" => 2,
                                    _ => return,
                                };
                                frequency_state.set(index);
                                if index == 2 {
                                    continuous_state.set(false);
                                }
                                window.refresh();
                            }
                        },
                    )
                    .into_any_element(),
                ))
                .child(match frequency_state.get() {
                    0 => {
                        let times = daily_times_state.clone();
                        let mut rows = v_flex().w_full().gap(px(8.));
                        let count = times.borrow().len();
                        for index in 0..count {
                            let state = times.borrow()[index].clone();
                            let add_times = times.clone();
                            let remove_times = times.clone();
                            rows = rows.child(
                                h_flex()
                                    .w_full()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(
                                        gpui_kit::component::time_field::TimeField::new(&state)
                                            .w(px(110.))
                                            .text_size(px(12.)),
                                    )
                                    .child(
                                        form_button(SharedString::from(format!(
                                            "automation-time-add-{index}"
                                        )))
                                        .label("+".to_string())
                                        .on_click(move |_, window, cx| {
                                            let mut times = add_times.borrow_mut();
                                            if times.len() < 24 {
                                                times.push(
                                                    cx.new(|cx| {
                                                        gpui_kit::component::time_field::TimeFieldState::new(window, cx)
                                                    }),
                                                );
                                            }
                                            window.refresh();
                                        }),
                                    )
                                    .child(
                                        form_button(SharedString::from(format!(
                                            "automation-time-remove-{index}"
                                        )))
                                        .w(px(32.))
                                        .px(px(0.))
                                        .disabled(times.borrow().len() <= 1)
                                        .child(Icon::new("trash").size(14.))
                                        .on_click(move |_, window, _| {
                                            let mut times = remove_times.borrow_mut();
                                            if times.len() > 1 {
                                                times.remove(index);
                                            }
                                            window.refresh();
                                        }),
                                    ),
                            );
                        }
                        field(t!("automation_execution_time").to_string(), rows.into_any_element())
                    }
                    1 => {
                        field(
                            t!("automation_interval_minutes").to_string(),
                            h_flex()
                                .w(px(168.))
                                .items_center()
                                .rounded(super::tokens::RADIUS_MEDIUM)
                                .border_1()
                                .border_color(border)
                                .bg(crate::ui::field_bg(cx))
                                .child(
                                    div()
                                        .id("automation-interval-minus")
                                        .flex_none()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .w(px(28.))
                                        .h(px(28.))
                                        .cursor_pointer()
                                        .text_color(muted)
                                        .hover(|style| style.bg(cx.theme().muted))
                                        .child("−")
                                        .on_click({
                                            let interval = interval_state.clone();
                                            move |_, window, cx| {
                                                let value = interval
                                                    .read(cx)
                                                    .value()
                                                    .trim()
                                                    .parse::<u32>()
                                                    .unwrap_or(5)
                                                    .saturating_sub(5)
                                                    .max(5);
                                                interval.update(cx, |state, cx| {
                                                    state.set_value(value.to_string(), window, cx)
                                                });
                                                window.refresh();
                                            }
                                        }),
                                )
                                .child(
                                    div().flex_1().min_w_0().child(
                                        Input::new(&interval)
                                            .appearance(false)
                                            .bordered(false)
                                            .xsmall()
                                            .text_align(TextAlign::Center),
                                    ),
                                )
                                .child(
                                    div()
                                        .id("automation-interval-plus")
                                        .flex_none()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .w(px(28.))
                                        .h(px(28.))
                                        .cursor_pointer()
                                        .text_color(muted)
                                        .hover(|style| style.bg(cx.theme().muted))
                                        .child("+")
                                        .on_click({
                                            let interval = interval_state.clone();
                                            move |_, window, cx| {
                                                let value = interval
                                                    .read(cx)
                                                    .value()
                                                    .trim()
                                                    .parse::<u32>()
                                                    .unwrap_or(5)
                                                    .saturating_add(5)
                                                    .clamp(5, 259_200);
                                                interval.update(cx, |state, cx| {
                                                    state.set_value(value.to_string(), window, cx)
                                                });
                                                window.refresh();
                                            }
                                        }),
                                )
                                .into_any_element(),
                        )
                    }
                    _ => field(
                        t!("automation_execution_date").to_string(),
                        DatePicker::new(&once_date_state)
                            .cleanable(true)
                            .text_size(px(12.))
                            .into_any_element(),
                    ),
                })
                .when(frequency_state.get() != 2, |form| {
                    let weekdays = weekdays_state.clone();
                    let mut picker = h_flex().w_full().flex_wrap().gap(px(4.));
                    for (day, key) in [
                        (1u32, "automation_weekday_short_mon"),
                        (2, "automation_weekday_short_tue"),
                        (3, "automation_weekday_short_wed"),
                        (4, "automation_weekday_short_thu"),
                        (5, "automation_weekday_short_fri"),
                        (6, "automation_weekday_short_sat"),
                        (7, "automation_weekday_short_sun"),
                    ] {
                        let weekdays = weekdays.clone();
                        let selected = weekdays.get() & (1 << day) != 0;
                        picker = picker.child(
                            form_button(SharedString::from(format!(
                                "automation-weekday-{day}"
                            )))
                            .label(t!(key).to_string())
                            .h(px(32.))
                            .px(px(8.))
                            .text_size(px(12.))
                            .when(selected, |button| button.primary())
                            .on_click(move |_, window, _| {
                                let bits = weekdays.get();
                                weekdays.set(bits ^ (1 << day));
                                window.refresh();
                            }),
                        );
                    }
                    form.child(field(
                        t!("automation_weekdays").to_string(),
                        picker.into_any_element(),
                    ))
                    .child(field(
                        t!("automation_effective_range").to_string(),
                        DatePicker::new(&effective_range)
                            .cleanable(true)
                            .text_size(px(12.))
                            .into_any_element(),
                    ))
                })
        } else {
            v_flex()
                .gap(super::tokens::SPACE_12)
                .child(field(
                    t!("automation_allowed_paths").to_string(),
                    v_flex()
                        .w_full()
                        .min_w_0()
                        .gap(px(8.))
                        .child(
                            h_flex().w_full().child(
                                form_button("automation-directory-pick".into())
                                    .label(t!("automation_add_directory").to_string())
                                    .on_click({
                                        let paths = allowed_paths_state.clone();
                                        move |_, window, cx| {
                                            pick_directories(paths.clone(), window, cx);
                                        }
                                    }),
                            ),
                        )
                        .when(!allowed_paths_state.borrow().is_empty(), |column| {
                            let mut tags = h_flex().w_full().flex_wrap().gap(px(4.));
                            for path in allowed_paths_state.borrow().clone() {
                                let remove_paths = allowed_paths_state.clone();
                                let path_for_remove = path.clone();
                                tags = tags.child(
                                    h_flex()
                                        .items_center()
                                        .gap(px(2.))
                                        .px_2()
                                        .py(px(1.))
                                        .rounded(super::tokens::RADIUS_SMALL)
                                        .bg(cx.theme().muted)
                                        .text_size(px(11.))
                                        .child(path.clone())
                                        .child(
                                            div()
                                                .id(SharedString::from(format!(
                                                    "automation-path-remove-{path}"
                                                )))
                                                .px(px(2.))
                                                .cursor_pointer()
                                                .hover(|style| {
                                                    style.text_color(cx.theme().danger)
                                                })
                                                .child("×")
                                                .on_click(move |_, window, _| {
                                                    remove_paths
                                                        .borrow_mut()
                                                        .retain(|item| item != &path_for_remove);
                                                    window.refresh();
                                                }),
                                        ),
                                );
                            }
                            column.child(tags)
                        })
                        .into_any_element(),
                ))
                .child(field(
                    t!("automation_shell_command").to_string(),
                    v_flex()
                        .w_full()
                        .min_w_0()
                        .gap(px(8.))
                        .child(
                            Textarea::new(&shell)
                                .w_full()
                                .min_h(px(72.))
                                .bg(crate::ui::field_bg(cx))
                                .h(px(72.)),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(muted)
                                .child(t!("automation_shell_command_hint").to_string()),
                        )
                        .into_any_element(),
                ))
                .when(frequency_state.get() != 2, |form| form.child(field(
                    t!("automation_continuous_context").to_string(),
                    v_flex()
                        .w_full()
                        .min_w_0()
                        .gap(px(8.))
                        .child(
                            Switch::new("automation-continuous")
                                .checked(continuous_state.get())
                                .small()
                                .flex().items_center().h(px(32.))
                                .on_click(move |checked, window, _| {
                                    continuous_state.set(*checked);
                                    window.refresh();
                                }),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(muted)
                                .child(t!("automation_continuous_context_hint").to_string()),
                        )
                        .into_any_element(),
                )))
                .child(field(
                    t!("automation_self_review").to_string(),
                    Switch::new("automation-self-review")
                        .checked(self_review_state.get())
                        .small()
                        .flex().items_center().h(px(32.))
                        .on_click(move |checked, window, _| {
                            self_review_state.set(*checked);
                            window.refresh();
                        })
                        .into_any_element(),
                ))
                .child(field(
                    t!("automation_start").to_string(),
                    Switch::new("automation-enabled")
                        .checked(enabled_state.get())
                        .small()
                        .flex().items_center().h(px(32.))
                        .on_click(move |checked, window, _| {
                            enabled_state.set(*checked);
                            window.refresh();
                        })
                        .into_any_element(),
                ))
        };
        let max_width = (window.viewport_size().width - px(32.)).max(px(360.));
        dialog
            .title(t!("create_automation").to_string())
            .w(px(600.))
            .max_w(max_width)
            .overlay_closable(false)
            .keyboard(false)
            .child(v_flex().gap(px(16.)).child(tabs).child(body))
            .footer(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        form_button("create-automation-cancel".into())
                            .label(t!("action_cancel").to_string())
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(submit),
            )
    });
}

pub(super) fn edit_automation(
    observer: &mut Observer,
    id: String,
    window: &mut Window,
    cx: &mut Context<Observer>,
) {
    use chatspeed_gpui::types::Command;
    use gpui_kit::component::{
        button::{Button, ButtonVariants},
        input::{Input, InputState},
    };
    use chatspeed_gpui::components::dialog::WindowExt;
    let (reply, response) = tokio::sync::oneshot::channel();
    if observer
        .worker
        .commands
        .try_send(Command::ReadAutomation {
            id: id.clone(),
            reply,
        })
        .is_err()
    {
        observer.show_toast(
            crate::ToastKind::Error,
            t!("automation_action_failed").to_string(),
            cx,
        );
        return;
    }
    cx.spawn_in(window, async move |owner, cx| {
        let result = response
            .await
            .unwrap_or_else(|_| Err(t!("automation_action_failed").to_string()));
        let _ = owner.update_in(cx, |this, window, cx| {
            let view = match result {
                Ok(view) => view,
                Err(error) => {
                    this.show_toast(crate::ToastKind::Error, error, cx);
                    return;
                }
            };
            if window.has_active_dialog(cx) {
                return;
            }
            let input = cx.new(|cx| {
                let mut input =
                    InputState::new(window, cx).placeholder(t!("automation_name").to_string());
                input.set_value(view["title"].as_str().unwrap_or_default(), window, cx);
                input
            });
            let prompt = cx.new(|cx| {
                let mut input = gpui_kit::component::input::TextareaState::new(window, cx);
                input.set_value(view["prompt"].as_str().unwrap_or_default(), window, cx);
                input
            });
            let current_schedule = automation_schedule_input(&view);
            let schedule = cx.new(|cx| {
                let mut input = InputState::new(window, cx)
                    .placeholder(t!("automation_schedule_input").to_string());
                input.set_value(current_schedule.as_str(), window, cx);
                input
            });
            let owner = cx.entity().downgrade();
            let field_bg = crate::ui::field_bg(cx);
            window.open_dialog(cx, move |dialog, _, _| {
                let (input_state, owner, id, view) =
                    (input.clone(), owner.clone(), id.clone(), view.clone());
                let (prompt_state, schedule_state) = (prompt.clone(), schedule.clone());
                dialog
                    .title(t!("automation_edit").to_string())
                    .child(
                        v_flex()
                            .gap_2()
                            .child(Input::new(&input).bg(field_bg))
                            .when(view["prompt_file_path"].is_null(), |form| {
                                form.child(t!("automation_prompt").to_string()).child(
                                    gpui_kit::component::input::Textarea::new(&prompt).bg(field_bg),
                                )
                            })
                            .child(t!("automation_schedule_input").to_string())
                            .child(Input::new(&schedule).bg(field_bg)),
                    )
                    .footer(
                        h_flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                Button::new("rename-automation-cancel")
                                    .label(t!("action_cancel").to_string())
                                    .on_click(|_, window, cx| window.close_dialog(cx)),
                            )
                            .child(
                                Button::new("rename-automation-save")
                                    .primary()
                                    .label(t!("session_save").to_string())
                                    .on_click(move |_, window, cx| {
                                        let title = input_state.read(cx).value();
                                        let saved =
                                            owner
                                                .update(cx, |this, cx| {
                                                    let body = automation_title_update(
                                                        &view,
                                                        title.as_ref(),
                                                    )
                                                    .and_then(|mut body| {
                                                        let prompt = prompt_state.read(cx).value();
                                                        let prompt = prompt.trim();
                                                        if view["prompt_file_path"].is_null() {
                                                            if prompt.is_empty() {
                                                                return None;
                                                            }
                                                            if view["prompt"]
                                                                .as_str()
                                                                .map(str::trim)
                                                                != Some(prompt)
                                                            {
                                                                body["spec"]["prompt"] =
                                                                    prompt.into();
                                                            }
                                                        }
                                                        let schedule =
                                                            schedule_state.read(cx).value();
                                                        let schedule = schedule.trim();
                                                        if schedule.is_empty()
                                                            && !automation_schedule_input(&view)
                                                                .is_empty()
                                                        {
                                                            return None;
                                                        }
                                                        if !schedule.is_empty()
                                                            && schedule
                                                                != automation_schedule_input(&view)
                                                        {
                                                            let (kind, config) =
                                                                parse_automation_schedule(
                                                                    schedule,
                                                                )?;
                                                            if kind == "once"
                                                                && view["continuous_context"]
                                                                    .as_bool()
                                                                    == Some(true)
                                                            {
                                                                return None;
                                                            }
                                                            let mut merged =
                                                                if view["schedule_kind"].as_str()
                                                                    == Some(kind)
                                                                {
                                                                    view["schedule_config"]
                                                                        .as_object()
                                                                        .cloned()
                                                                        .unwrap_or_default()
                                                                } else {
                                                                    serde_json::Map::new()
                                                                };
                                                            if let Some(fields) = config.as_object()
                                                            {
                                                                merged.extend(fields.clone());
                                                            }
                                                            body["spec"]["schedule_kind"] =
                                                                kind.into();
                                                            body["spec"]["schedule_config"] =
                                                                merged.into();
                                                        }
                                                        Some(body)
                                                    });
                                                    let Some(body) = body else {
                                                        this.show_toast(
                                                            crate::ToastKind::Warning,
                                                            t!("automation_required_fields")
                                                                .to_string(),
                                                            cx,
                                                        );
                                                        return None;
                                                    };
                                                    let (reply, response) =
                                                        tokio::sync::oneshot::channel();
                                                    if this
                                                        .worker
                                                        .commands
                                                        .try_send(Command::DraftAutomation {
                                                            id: id.clone(),
                                                            body,
                                                            reply,
                                                        })
                                                        .is_ok()
                                                    {
                                                        return Some(response);
                                                    }
                                                    this.show_toast(
                                                        crate::ToastKind::Error,
                                                        t!("automation_action_failed").to_string(),
                                                        cx,
                                                    );
                                                    None
                                                })
                                                .ok()
                                                .flatten();
                                        if let Some(response) = saved {
                                            window.close_dialog(cx);
                                            let owner = owner.clone();
                                            window
                                                .spawn(cx, async move |cx| {
                                                    let result =
                                                        response.await.unwrap_or_else(|_| {
                                                            Err(t!("automation_action_failed")
                                                                .to_string())
                                                        });
                                                    let _ =
                                                        owner.update_in(cx, |this, window, cx| {
                                                            match result {
                                                                Ok(plan) => review_automation(
                                                                    plan, window, cx,
                                                                ),
                                                                Err(error) => this.show_toast(
                                                                    crate::ToastKind::Error,
                                                                    error,
                                                                    cx,
                                                                ),
                                                            }
                                                        });
                                                })
                                                .detach();
                                        }
                                    }),
                            ),
                    )
            });
        });
    })
    .detach();
}

fn review_automation(plan: serde_json::Value, window: &mut Window, cx: &mut Context<Observer>) {
    use chatspeed_gpui::types::Command;
    use gpui_kit::component::{
        button::{Button, ButtonVariants},
    };
    use chatspeed_gpui::components::dialog::WindowExt;
    if window.has_active_dialog(cx) {
        return;
    }
    let owner = cx.entity().downgrade();
    window.open_dialog(cx, move |dialog, _, _| {
        let owner = owner.clone();
        let expansion = plan["permission_summary"]["permission_expansion"] == true;
        let ready = plan["status"] == "ready" && plan["plan_hash"].is_string();
        let request = serde_json::json!({"plan": plan, "expected_plan_hash": plan["plan_hash"], "acknowledge_permission_changes": expansion});
        dialog.title(t!("automation_review_plan").to_string())
            .child(div().max_h(px(360.)).id("automation-plan").overflow_y_scroll()
                .child(serde_json::to_string_pretty(&plan).unwrap_or_default()))
            .footer(h_flex().justify_end().gap_2()
                .child(Button::new("cancel-plan").label(t!("action_cancel").to_string()).on_click(|_, window, cx| window.close_dialog(cx)))
                .when(ready, |footer| footer.child(Button::new("apply-plan").primary()
                    .label(if expansion { t!("automation_acknowledge_apply").to_string() } else { t!("session_save").to_string() })
                    .on_click(move |_, window, cx| {
                        let queued = owner.update(cx, |this, cx| {
                            if this.worker.commands.try_send(Command::ApplyAutomation(request.clone())).is_ok() { return true; }
                            this.show_toast(crate::ToastKind::Error, t!("automation_action_failed").to_string(), cx);
                            false
                        }).unwrap_or(false);
                        if queued { window.close_dialog(cx); }
                    }))))
    });
}

fn automation_schedule_input(view: &serde_json::Value) -> String {
    let config = &view["schedule_config"];
    match view["schedule_kind"].as_str() {
        Some("interval") => config["interval_minutes"]
            .as_u64()
            .map(|minutes| format!("{minutes}m"))
            .unwrap_or_default(),
        Some("once") => config["run_at"].as_str().unwrap_or_default().to_owned(),
        Some("daily") => config["times"]
            .as_array()
            .filter(|times| !times.is_empty())
            .map(|times| {
                times
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_else(|| config["time"].as_str().unwrap_or_default().to_owned()),
        _ => String::new(),
    }
}

fn automation_title_update(view: &serde_json::Value, title: &str) -> Option<serde_json::Value> {
    let title = title.trim();
    if title.is_empty() {
        return None;
    }
    let revision = view.get("revision")?.as_u64()?;
    let mut spec = serde_json::Map::new();
    for field in [
        "prompt",
        "prompt_file_path",
        "agent_id",
        "agent_config",
        "allowed_paths",
        "shell_config",
        "schedule_kind",
        "schedule_config",
        "continuous_context",
        "self_review",
        "enabled",
    ] {
        spec.insert(field.to_owned(), view.get(field)?.clone());
    }
    spec.insert("title".into(), title.into());
    Some(serde_json::json!({"spec": spec, "expected_revision": revision}))
}

#[cfg(test)]
#[::core::prelude::v1::test]
fn automation_rename_preserves_spec_and_revision() {
    let view = serde_json::json!({
        "automation_id": "a", "revision": 4, "title": "Old",
        "prompt": null, "prompt_file_path": "/project/task.md", "agent_id": "agent",
        "agent_config": {"custom": true}, "allowed_paths": ["/project"],
        "shell_config": null, "schedule_kind": "daily", "schedule_config": {"time": "09:00"},
        "continuous_context": true, "self_review": true, "enabled": false,
    });
    let mut expected = view.clone();
    let fields = expected.as_object_mut().unwrap();
    fields.remove("automation_id");
    fields.remove("revision");
    expected["title"] = "New".into();
    assert_eq!(
        automation_title_update(&view, " New "),
        Some(serde_json::json!({"spec": expected, "expected_revision": 4}))
    );
    assert_eq!(automation_schedule_input(&view), "09:00");
    for (kind, config, expected) in [
        (
            "daily",
            serde_json::json!({"times": ["09:00", "18:00"]}),
            "09:00, 18:00",
        ),
        (
            "interval",
            serde_json::json!({"interval_minutes": 30}),
            "30m",
        ),
        (
            "once",
            serde_json::json!({"run_at": "2030-01-02 09:30"}),
            "2030-01-02 09:30",
        ),
    ] {
        let view = serde_json::json!({"schedule_kind": kind, "schedule_config": config});
        let input = automation_schedule_input(&view);
        assert_eq!(input, expected);
        assert_eq!(
            parse_automation_schedule(&input).map(|(kind, _)| kind),
            Some(kind)
        );
    }
    assert!(automation_title_update(&view, " ").is_none());
    assert!(automation_title_update(&serde_json::json!({"revision": 4}), "New").is_none());
}

fn valid_automation_date_range<T: Ord>(start: Option<T>, end: Option<T>) -> bool {
    match (start, end) {
        (None, None) => true,
        (Some(start), Some(end)) => start <= end,
        _ => false,
    }
}

#[cfg(test)]
#[core::prelude::v1::test]
fn automation_date_range_requires_ordered_endpoints_or_no_range() {
    assert!(valid_automation_date_range::<u8>(None, None));
    assert!(valid_automation_date_range(Some(1), Some(2)));
    assert!(valid_automation_date_range(Some(1), Some(1)));
    assert!(!valid_automation_date_range(Some(2), Some(1)));
    assert!(!valid_automation_date_range(Some(1), None));
    assert!(!valid_automation_date_range(None, Some(1)));
}

fn parse_automation_schedule(value: &str) -> Option<(&'static str, serde_json::Value)> {
    let value = value.trim();
    if let Some(minutes) = value.strip_suffix('m') {
        let minutes = minutes.trim().parse::<u32>().ok().filter(|n| *n >= 5)?;
        return Some(("interval", serde_json::json!({"interval_minutes": minutes})));
    }
    if let Some((date, time)) = value.split_once(' ') {
        let parts: Vec<_> = date.split('-').collect();
        if parts.len() == 3
            && parts[0].len() == 4
            && parts[1].len() == 2
            && parts[2].len() == 2
            && parts
                .iter()
                .all(|part| part.bytes().all(|b| b.is_ascii_digit()))
            && valid_daily_time(time)
        {
            return Some(("once", serde_json::json!({"run_at": value})));
        }
    }
    parse_daily_schedule(value).map(|schedule| ("daily", schedule))
}

fn parse_daily_schedule(value: &str) -> Option<serde_json::Value> {
    let mut times: Vec<_> = value.split([',', '，']).map(str::trim).collect();
    if !times.iter().all(|time| valid_daily_time(time)) {
        return None;
    }
    times.sort_unstable();
    times.dedup();
    Some(serde_json::json!({"time": times.first()?, "times": times}))
}

#[::core::prelude::v1::test]
fn daily_schedule_normalizes_times() {
    assert_eq!(
        parse_daily_schedule("18:00，09:00, 09:00"),
        Some(serde_json::json!({"time":"09:00","times":["09:00","18:00"]}))
    );
    assert!(parse_daily_schedule("09:00, 24:00").is_none());
    assert!(parse_daily_schedule("").is_none());
    assert_eq!(
        parse_automation_schedule("30m"),
        Some(("interval", serde_json::json!({"interval_minutes":30})))
    );
    assert!(parse_automation_schedule("4m").is_none());
    assert_eq!(
        parse_automation_schedule("2030-01-02 09:30"),
        Some(("once", serde_json::json!({"run_at":"2030-01-02 09:30"})))
    );
    assert!(parse_automation_schedule("2030-01-02 29:30").is_none());
}

fn valid_daily_time(time: &str) -> bool {
    let Some((hour, minute)) = time.split_once(':') else {
        return false;
    };
    hour.len() == 2
        && minute.len() == 2
        && hour
            .bytes()
            .chain(minute.bytes())
            .all(|b| b.is_ascii_digit())
        && hour.parse::<u8>().is_ok_and(|v| v < 24)
        && minute.parse::<u8>().is_ok_and(|v| v < 60)
}

#[cfg(not(feature = "chat-hub"))]
pub(super) fn toggle_chat_hub(
    this: &mut Observer,
    _window: &mut Window,
    cx: &mut Context<Observer>,
) {
    this.not_implemented(t!("chat_hub"), cx);
}

/// Renders the 40px titlebar above the sidebar and transcript.
pub fn titlebar(observer: &Observer, window: &Window, cx: &mut Context<Observer>) -> AnyElement {
    let skin = surfaces(observer.appearance_dark);
    let secondary = cx.theme().muted_foreground;
    let foreground = cx.theme().foreground;
    let accent = cx.theme().primary;
    let collapsed = observer.sidebar_collapsed;
    let radii = crate::ui::frame::inner_corner_radii(window, cx);

    // Vue spaces titlebar buttons with `.left/.right { gap: 4px }` plus
    // `.icon-btn { margin-right: 4px }`, so an 8px gap matches the web bar.
    let mut left = h_flex().flex_none().items_center().gap(px(8.));
    if observer.show_menu_button {
        left = left.child(menu(observer, secondary, cx));
    }
    left = left.child(icon_button(
        "toggle-sidebar",
        "sidebar",
        if collapsed { secondary } else { accent },
        skin.hover,
        Some(
            if collapsed {
                t!("expand_sidebar")
            } else {
                t!("collapse_sidebar")
            }
            .to_string(),
        ),
        cx.listener(|this, _, _, cx| this.toggle_sidebar(cx)),
    ));
    let pending = observer.pending_approvals();
    if !pending.is_empty() {
        left = left.child(approval_queue(pending, secondary, accent, skin.hover, cx));
    }
    left = left.child(sound_menu(observer, secondary, skin.hover, cx));

    let center = h_flex()
        .flex_1()
        .min_w_0()
        .justify_center()
        .gap(px(32.))
        .when_some(observer.roots().first().cloned(), |row, root| {
            row.child(
                h_flex()
                    .id("titlebar-path")
                    .flex_none()
                    .min_w_0()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py(px(2.))
                    .rounded(super::tokens::RADIUS_MEDIUM)
                    .cursor_pointer()
                    .hover(move |style| style.bg(skin.hover))
                    .child(Icon::new("ext-folder").size(12.).color(secondary))
                    .child(
                        div()
                            .max_w(px(320.))
                            .min_w_0()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(foreground)
                            .text_ellipsis()
                            .child(chatspeed_gpui::types::root_name(&root).to_owned()),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.open_files_tab(cx))),
            )
        })
        .when(observer.show_today_cost, |row| {
            row.child(
                h_flex()
                    .flex_none()
                    .gap_1()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(accent)
                    .child(Icon::new("money").size(12.).color(accent))
                    .child(super::chrome_label(chatspeed_gpui::native::compact_cost(
                        observer.today_cost,
                    ))),
            )
        });

    // The four workflow actions keep Vue's order: automation, new task, undo,
    // pin. The window controls follow them behind a wider gap, and the group
    // ends at the same edge inset as the left one.
    let mut right = h_flex()
        .flex_none()
        .items_center()
        .gap(px(8.))
        .child(icon_button(
            "create-automation",
            "add-automation",
            secondary,
            skin.hover,
            Some(t!("create_automation").to_string()),
            cx.listener(|this, _, window, cx| open_automation(this, window, cx)),
        ))
        .child(icon_button(
            "new-session",
            "edit",
            secondary,
            skin.hover,
            Some(t!("new_task").to_string()),
            cx.listener(|this, _, _, cx| this.open_new_task_dialog(cx)),
        ))
        .child(icon_button(
            "rewind",
            "undo",
            if observer.can_rewind() {
                secondary
            } else {
                secondary.opacity(0.45)
            },
            skin.hover,
            Some(t!("delete_last_message").to_string()),
            cx.listener(|this, _, window, cx| this.confirm_rewind(window, cx)),
        ))
        .when(super::window_pin::supported(window), |row| {
            row.child(super::icon_button_element(
                "pin-window",
                Icon::new("pin")
                    .size(18.)
                    .color(if observer.pinned {
                        cx.theme().primary
                    } else {
                        secondary
                    })
                    .rotate(if observer.pinned { 0. } else { 45. }),
                skin.hover,
                Some(
                    if observer.pinned {
                        t!("unpin")
                    } else {
                        t!("pin")
                    }
                    .to_string(),
                ),
                cx.listener(|this, _, window, cx| this.toggle_pin(window, cx)),
            ))
        });
    if let Some(controls) = window_controls(window, cx) {
        right = right.child(div().flex_none().w(px(16.))).child(controls);
    }

    h_flex()
        .id("title-bar")
        .w_full()
        .h(px(TITLEBAR_HEIGHT))
        .flex_none()
        .items_center()
        .gap_2()
        .pl(px(if cfg!(target_os = "macos") {
            MACOS_TRAFFIC_LIGHTS_INSET
        } else {
            SIDE_INSET
        }))
        .pr(px(SIDE_INSET))
        // The window clips its children to a rectangular mask, so the bar has to
        // round its own top corners; the inner radius keeps its arc concentric
        // with the kit's outline instead of stepping against it.
        .rounded_tl(radii.top_left)
        .rounded_tr(radii.top_right)
        .border_b_1()
        .border_color(skin.titlebar_border)
        // Vue's bar is a lighter strip over the body surface with a hairline
        // border; the soft shadow below it is painted by the frame.
        .bg(skin.titlebar)
        // A press that a control did not take starts a native window move once
        // the pointer travels, so a plain click never moves the window.
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, window, _| {
                this.title_drag = !window.default_prevented();
            }),
        )
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|this, _, _, _| this.title_drag = false),
        )
        .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
            if event.pressed_button != Some(MouseButton::Left) {
                this.title_drag = false;
                return;
            }
            if !this.title_drag {
                return;
            }
            this.title_drag = false;
            cx.stop_active_drag(window);
            window.start_window_move();
        }))
        .on_double_click(cx.listener(|_, _, window, _| window.zoom_window()))
        .on_mouse_down(MouseButton::Right, |event: &MouseDownEvent, window, _| {
            window.show_window_menu(event.position);
        })
        .child(left)
        .child(center)
        .child(right)
        .into_any_element()
}

/// Menu groups and order match `src/components/window/Titlebar.vue`.
fn menu(observer: &Observer, secondary: Hsla, cx: &mut Context<Observer>) -> AnyElement {
    let skin = surfaces(observer.appearance_dark);
    let hover = skin.hover;
    let border = cx.theme().border;
    let foreground = cx.theme().foreground;
    let weak = cx.entity().downgrade();
    let groups: [&[(&str, &str, &str, Action)]; 4] = [
        &[
            ("menu-chat", "skill-chat-square", "menu_chat", Action::Chat),
            (
                "menu-assistant",
                "assistant",
                "menu_assistant",
                Action::Assistant,
            ),
            (
                "menu-workflow",
                "skill-plan3",
                "menu_workflow",
                Action::Workflow,
            ),
            ("menu-note", "note", "menu_note", Action::Note),
            (
                "menu-proxy-switcher",
                "switch",
                "menu_proxy_switcher",
                Action::ProxySwitcher,
            ),
        ],
        &[
            (
                "menu-settings",
                "setting",
                "menu_settings",
                Action::Settings,
            ),
            ("menu-model", "model", "menu_model", Action::Model),
            ("menu-agent", "agent", "menu_agent", Action::Agent),
            ("menu-proxy", "proxy", "menu_proxy", Action::Proxy),
            (
                "menu-chat-hub",
                "connected",
                "menu_chat_hub",
                Action::ChatHub,
            ),
            ("menu-mcp", "mcp", "menu_mcp", Action::Mcp),
            ("menu-skill", "skill", "menu_skill", Action::Skill),
        ],
        &[(
            "menu-scraper",
            "extract",
            "menu_scraper",
            Action::ScraperTest,
        )],
        &[
            ("menu-about", "about", "menu_about", Action::About),
            ("menu-quit", "quit", "menu_quit", Action::Quit),
        ],
    ];

    Popover::new("titlebar-menu")
        .anchor(Anchor::TopLeft)
        .offset(px(6.))
        .trigger(trigger(flat_button(
            "menu-button",
            "menu",
            secondary,
            hover,
            None,
        )))
        .content(move |_, window, cx| {
            let popover = cx.entity();
            let mut list = v_flex()
                .id("titlebar-menu-list")
                .w(px(196.))
                .max_h((window.viewport_size().height - px(TITLEBAR_HEIGHT + 28.)).max(px(40.)))
                .overflow_y_scroll()
                .gap(px(2.));
            for (index, group) in groups.iter().enumerate() {
                if index > 0 {
                    list = list.child(div().flex_none().mx_2().my_1().h(px(1.)).bg(border));
                }
                for &(id, icon, label, action) in *group {
                    let popover = popover.clone();
                    let weak = weak.clone();
                    list = list.child(menu_row(
                        id,
                        icon,
                        t!(label).to_string(),
                        foreground,
                        hover,
                        move |_, window, cx| {
                            let _ = popover.update(cx, |state, cx| state.dismiss(window, cx));
                            let _ =
                                weak.update(cx, |this, cx| this.desktop_menu_action(action, cx));
                        },
                    ));
                }
            }
            list
        })
        .into_any_element()
}

/// The approval queue: only rendered while the runtime waits on a decision.
fn approval_queue(
    pending: Vec<(PendingDecision, String, String)>,
    secondary: Hsla,
    accent: Hsla,
    hover: Hsla,
    cx: &mut Context<Observer>,
) -> AnyElement {
    let count = if pending.len() > 9 {
        "9+".to_owned()
    } else {
        pending.len().to_string()
    };
    let badge = crate::success();
    let foreground = cx.theme().foreground;
    let weak = cx.entity().downgrade();

    let button = div()
        .relative()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .w(px(26.))
        .h(px(22.))
        // Vue's `.approval-queue-btn` keeps `--cs-space` on the right; with the
        // 8px group gap this comes to the same 20px before the sound button.
        .mr(px(12.))
        .rounded(super::tokens::RADIUS_MEDIUM)
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(Icon::new("approval").size(18.).color(secondary))
        .child(
            div()
                .absolute()
                .top(px(-5.))
                .right(px(-5.))
                .w(px(18.))
                .h(px(18.))
                .rounded_full()
                .bg(badge)
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(10.))
                .text_color(rgb(0xffffff))
                .child(count),
        );

    Popover::new("titlebar-approvals")
        .anchor(Anchor::TopLeft)
        .offset(px(6.))
        .trigger(trigger(button))
        .content(move |_, window, cx| {
            let popover = cx.entity();
            let mut list = v_flex()
                .id("decision-list")
                .w(px(240.))
                .gap(px(2.))
                .max_h((window.viewport_size().height - px(TITLEBAR_HEIGHT + 48.)).max(px(0.)))
                .overflow_y_scroll();
            for (index, (entry, owner, title)) in pending.iter().enumerate() {
                let owner = owner.clone();
                let weak = weak.clone();
                let popover = popover.clone();
                let target = entry.target.clone();
                let (icon, label) = match &entry.kind {
                    DecisionKind::Question => ("ask_user", t!("status_awaiting_user").to_string()),
                    DecisionKind::Approval(name) if !name.is_empty() => ("approval", name.clone()),
                    DecisionKind::Approval(_) => {
                        ("approval", t!("status_awaiting_approval").to_string())
                    }
                };
                list = list.child(
                    v_flex()
                        .id(("approval", index))
                        .w_full()
                        .gap(px(2.))
                        .px_2()
                        .py(px(4.))
                        .rounded(super::tokens::RADIUS_MEDIUM)
                        .cursor_pointer()
                        .hover(move |style| style.bg(hover))
                        .child(
                            h_flex()
                                .items_center()
                                .gap_2()
                                .child(Icon::new(icon).size(14.).color(accent))
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .text_color(foreground)
                                        .child(super::chrome_label(label)),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(secondary)
                                .child(title.clone()),
                        )
                        .on_click(move |_, window, cx| {
                            let _ = popover.update(cx, |state, cx| state.dismiss(window, cx));
                            let _ = weak.update(cx, |this, cx| {
                                this.open_pending_decision(owner.clone(), target.clone(), cx);
                            });
                        }),
                );
            }
            list
        })
        .into_any_element()
}

/// Approval and completion chimes, as the two switches Vue nests in this menu.
fn sound_menu(
    observer: &Observer,
    secondary: Hsla,
    hover: Hsla,
    cx: &mut Context<Observer>,
) -> AnyElement {
    let approval_on = observer.sound_is_enabled(Chime::Approval);
    let completion_on = observer.sound_is_enabled(Chime::Completion);
    let icon = if approval_on || completion_on {
        "unmute"
    } else {
        "mute"
    };
    let accent = cx.theme().primary;
    let foreground = cx.theme().foreground;
    let weak = cx.entity().downgrade();

    Popover::new("titlebar-sound")
        .anchor(Anchor::TopLeft)
        .offset(px(6.))
        .trigger(trigger(flat_button(
            "sound-button",
            icon,
            secondary,
            hover,
            Some(t!("notification_sound").to_string()),
        )))
        .content(move |_, _, cx| {
            let popover = cx.entity();
            let approval = {
                let popover = popover.clone();
                let weak = weak.clone();
                switch_row(
                    "sound-approval",
                    t!("sound_approval").to_string(),
                    approval_on,
                    accent,
                    foreground,
                    secondary,
                    hover,
                    move |window, cx| {
                        let _ = popover.update(cx, |state, cx| state.dismiss(window, cx));
                        let _ = weak.update(cx, |this, cx| {
                            this.toggle_sound(Chime::Approval, cx);
                        });
                    },
                )
            };
            let completion = {
                let popover = popover.clone();
                let weak = weak.clone();
                switch_row(
                    "sound-completion",
                    t!("sound_completion").to_string(),
                    completion_on,
                    accent,
                    foreground,
                    secondary,
                    hover,
                    move |window, cx| {
                        let _ = popover.update(cx, |state, cx| state.dismiss(window, cx));
                        let _ = weak.update(cx, |this, cx| {
                            this.toggle_sound(Chime::Completion, cx);
                        });
                    },
                )
            };
            v_flex()
                .w(px(180.))
                .gap(px(2.))
                .child(approval)
                .child(completion)
        })
        .into_any_element()
}

/// A flat 26x22 trigger surface: 18px glyph, Vue's `.icon-btn` box.
fn flat_button(
    id: &'static str,
    icon: &'static str,
    color: Hsla,
    hover: Hsla,
    hint: Option<String>,
) -> impl IntoElement {
    h_flex()
        .id(id)
        .flex_none()
        .items_center()
        .justify_center()
        .w(px(26.))
        .h(px(22.))
        .rounded(super::tokens::RADIUS_MEDIUM)
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(Icon::new(icon).size(18.).color(color))
        .when_some(hint, |button, text| {
            button
                .tooltip(crate::ui::tip(text))
                .tooltip_show_delay(crate::ui::TOOLTIP_DELAY)
        })
}

fn menu_row(
    id: &'static str,
    icon: &'static str,
    label: String,
    text: Hsla,
    hover: Hsla,
    run: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    h_flex()
        .id(id)
        .w_full()
        .items_center()
        .gap_2()
        .px_2()
        .py(px(4.))
        .rounded(super::tokens::RADIUS_MEDIUM)
        .cursor_pointer()
        .text_color(text)
        .hover(move |style| style.bg(hover))
        .child(Icon::new(icon).size(14.).color(text))
        .child(div().text_size(px(13.)).child(super::chrome_label(label)))
        .on_click(run)
        .into_any_element()
}

/// One switch row: a check glyph instead of a checkbox, matching the Vue menu.
fn switch_row(
    id: &'static str,
    label: String,
    enabled: bool,
    accent: Hsla,
    foreground: Hsla,
    secondary: Hsla,
    hover: Hsla,
    run: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    h_flex()
        .id(id)
        .w_full()
        .items_center()
        .gap_2()
        .px_2()
        .py(px(4.))
        .rounded(super::tokens::RADIUS_MEDIUM)
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(
            Icon::new(if enabled { "check" } else { "uncheck" })
                .size(14.)
                .color(if enabled { accent } else { secondary }),
        )
        .child(
            div()
                .text_size(px(13.))
                .text_color(foreground)
                .child(super::chrome_label(label)),
        )
        .on_click(move |_, window, cx| run(window, cx))
        .into_any_element()
}

/// Window controls for client-decorated windows; `None` when the window manager
/// draws its own, which is also the case on macOS with its native traffic
/// lights.
fn window_controls(window: &Window, cx: &mut Context<Observer>) -> Option<AnyElement> {
    if cfg!(target_os = "macos") {
        return None;
    }
    if !matches!(window.window_decorations(), Decorations::Client { .. }) {
        return None;
    }
    let supported = window.window_controls();

    let mut row = h_flex()
        .id("window-controls")
        .flex_none()
        .items_center()
        .h_full();
    if supported.minimize {
        row = row.child(control(
            "window-minimize",
            IconName::WindowMinimize,
            cx,
            |window, _| window.minimize_window(),
        ));
    }
    if supported.maximize {
        let icon = if window.is_maximized() {
            IconName::WindowRestore
        } else {
            IconName::WindowMaximize
        };
        row = row.child(control("window-maximize", icon, cx, |window, _| {
            window.zoom_window()
        }));
    }
    Some(
        row.child(control(
            "window-close",
            IconName::WindowClose,
            cx,
            |window, _| window.remove_window(),
        ))
        .into_any_element(),
    )
}

fn control(
    id: &'static str,
    icon: IconName,
    cx: &App,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let is_close = id == "window-close";
    let theme = cx.theme();
    let (hover_bg, active_bg, hover_fg) = if is_close {
        (theme.danger, theme.danger_active, theme.danger_foreground)
    } else {
        (
            theme.secondary_hover,
            theme.secondary_active,
            theme.secondary_foreground,
        )
    };
    let foreground = theme.foreground;

    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .w(px(CONTROL_WIDTH))
        .h_full()
        .text_color(foreground)
        .hover(|style| style.bg(hover_bg).text_color(hover_fg))
        .active(|style| style.bg(active_bg).text_color(hover_fg))
        // The press must not reach the titlebar, which would start a move.
        .on_mouse_down(MouseButton::Left, |_, window, cx| {
            window.prevent_default();
            cx.stop_propagation();
        })
        .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            on_click(window, cx);
        })
        .child(KitIcon::new(icon).small())
        .into_any_element()
}
