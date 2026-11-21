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

fn open_automation(observer: &mut Observer, window: &mut Window, cx: &mut Context<Observer>) {
    use gpui_kit::component::{
        button::{Button, ButtonVariants},
        input::{Input, InputState, Textarea, TextareaState},
        WindowExt,
    };
    if observer.automation_pending || window.has_active_dialog(cx) {
        return;
    }
    let Some(agent) = observer.current_agent_id().or_else(|| {
        observer
            .agents
            .iter()
            .find_map(|agent| agent["id"].as_str().map(str::to_owned))
    }) else {
        observer.show_toast(
            crate::ToastKind::Warning,
            t!("automation_agent_required").to_string(),
            cx,
        );
        return;
    };
    let agent_label = observer
        .agents
        .iter()
        .find(|entry| entry["id"].as_str() == Some(agent.as_str()))
        .and_then(|entry| entry["name"].as_str())
        .unwrap_or(&agent)
        .to_owned();
    let title =
        cx.new(|cx| InputState::new(window, cx).placeholder(t!("automation_name").to_string()));
    let prompt = cx
        .new(|cx| TextareaState::new(window, cx).placeholder(t!("automation_prompt").to_string()));
    let time = cx.new(|cx| {
        let mut input = InputState::new(window, cx).placeholder("09:00");
        input.set_value("09:00", window, cx);
        input
    });
    let owner = cx.entity().downgrade();
    window.open_dialog(cx, move |dialog, _, cx| {
        let (title_state, prompt_state, time_state, agent, owner) = (title.clone(), prompt.clone(), time.clone(), agent.clone(), owner.clone());
        let submit = Button::new("create-automation-submit").primary().label(t!("create_automation").to_string())
            .on_click(move |_, window, cx| {
                let name = title_state.read(cx).value().trim().to_owned();
                let prompt = prompt_state.read(cx).value().trim().to_owned();
                let schedule = parse_automation_schedule(time_state.read(cx).value().as_ref());
                let accepted = owner.update(cx, |this, cx| {
                    if this.automation_pending { return false; }
                    let Some((kind, schedule)) = schedule.filter(|_| !name.is_empty() && !prompt.is_empty()) else {
                        this.show_toast(crate::ToastKind::Warning, t!("automation_required_fields").to_string(), cx);
                        return false;
                    };
                    let spec = serde_json::json!({
                        "title": name, "prompt": prompt, "agent_id": agent,
                        "allowed_paths": [], "schedule_kind": kind, "schedule_config": schedule,
                        "continuous_context": false, "self_review": false, "enabled": true,
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
        dialog.title(t!("create_automation").to_string()).child(v_flex().gap_2()
            .child(div().text_size(px(12.)).text_color(cx.theme().muted_foreground).child(t!("automation_agent_label", agent = agent_label.as_str()).to_string()))
            .child(Input::new(&title)).child(Textarea::new(&prompt))
            .child(t!("automation_schedule_input").to_string()).child(Input::new(&time)))
            .footer(h_flex().justify_end().gap_2()
                .child(Button::new("create-automation-cancel").label(t!("action_cancel").to_string()).on_click(|_, window, cx| window.close_dialog(cx)))
                .child(submit))
    });
}

pub(super) fn edit_automation(observer: &mut Observer, id: String, window: &mut Window, cx: &mut Context<Observer>) {
    use chatspeed_gpui::types::Command;
    use gpui_kit::component::{button::{Button, ButtonVariants}, input::{Input, InputState}, WindowExt};
    let (reply, response) = tokio::sync::oneshot::channel();
    if observer.worker.commands.try_send(Command::ReadAutomation { id: id.clone(), reply }).is_err() {
        observer.show_toast(crate::ToastKind::Error, t!("automation_action_failed").to_string(), cx);
        return;
    }
    cx.spawn_in(window, async move |owner, cx| {
        let result = response.await.unwrap_or_else(|_| Err(t!("automation_action_failed").to_string()));
        let _ = owner.update_in(cx, |this, window, cx| {
            let view = match result {
                Ok(view) => view,
                Err(error) => { this.show_toast(crate::ToastKind::Error, error, cx); return; }
            };
            if window.has_active_dialog(cx) { return; }
            let input = cx.new(|cx| {
                let mut input = InputState::new(window, cx).placeholder(t!("automation_name").to_string());
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
                let mut input = InputState::new(window, cx).placeholder(t!("automation_schedule_input").to_string());
                input.set_value(current_schedule.as_str(), window, cx);
                input
            });
            let owner = cx.entity().downgrade();
            window.open_dialog(cx, move |dialog, _, _| {
                let (input_state, owner, id, view) = (input.clone(), owner.clone(), id.clone(), view.clone());
                let (prompt_state, schedule_state) = (prompt.clone(), schedule.clone());
                dialog.title(t!("automation_edit").to_string()).child(v_flex().gap_2()
                    .child(Input::new(&input))
                    .when(view["prompt_file_path"].is_null(), |form| form.child(t!("automation_prompt").to_string()).child(gpui_kit::component::input::Textarea::new(&prompt)))
                    .child(t!("automation_schedule_input").to_string()).child(Input::new(&schedule)))
                    .footer(h_flex().justify_end().gap_2()
                        .child(Button::new("rename-automation-cancel").label(t!("action_cancel").to_string()).on_click(|_, window, cx| window.close_dialog(cx)))
                        .child(Button::new("rename-automation-save").primary().label(t!("session_save").to_string()).on_click(move |_, window, cx| {
                            let title = input_state.read(cx).value();
                            let saved = owner.update(cx, |this, cx| {
                                let body = automation_title_update(&view, title.as_ref()).and_then(|mut body| {
                                    let prompt = prompt_state.read(cx).value();
                                    let prompt = prompt.trim();
                                    if view["prompt_file_path"].is_null() {
                                        if prompt.is_empty() {
                                            return None;
                                        }
                                        if view["prompt"].as_str().map(str::trim) != Some(prompt) {
                                            body["spec"]["prompt"] = prompt.into();
                                        }
                                    }
                                    let schedule = schedule_state.read(cx).value();
                                    let schedule = schedule.trim();
                                    if schedule.is_empty() && !automation_schedule_input(&view).is_empty() {
                                        return None;
                                    }
                                    if !schedule.is_empty() && schedule != automation_schedule_input(&view) {
                                        let (kind, config) = parse_automation_schedule(schedule)?;
                                        if kind == "once" && view["continuous_context"].as_bool() == Some(true) { return None; }
                                        let mut merged = if view["schedule_kind"].as_str() == Some(kind) {
                                            view["schedule_config"].as_object().cloned().unwrap_or_default()
                                        } else {
                                            serde_json::Map::new()
                                        };
                                        if let Some(fields) = config.as_object() {
                                            merged.extend(fields.clone());
                                        }
                                        body["spec"]["schedule_kind"] = kind.into();
                                        body["spec"]["schedule_config"] = merged.into();
                                    }
                                    Some(body)
                                });
                                let Some(body) = body else {
                                    this.show_toast(crate::ToastKind::Warning, t!("automation_required_fields").to_string(), cx);
                                    return None;
                                };
                                let (reply, response) = tokio::sync::oneshot::channel();
                                if this.worker.commands.try_send(Command::DraftAutomation { id: id.clone(), body, reply }).is_ok() {
                                    return Some(response);
                                }
                                this.show_toast(crate::ToastKind::Error, t!("automation_action_failed").to_string(), cx);
                                None
                            }).ok().flatten();
                            if let Some(response) = saved {
                                window.close_dialog(cx);
                                let owner = owner.clone();
                                window.spawn(cx, async move |cx| {
                                    let result = response.await.unwrap_or_else(|_| Err(t!("automation_action_failed").to_string()));
                                    let _ = owner.update_in(cx, |this, window, cx| match result {
                                        Ok(plan) => review_automation(plan, window, cx),
                                        Err(error) => this.show_toast(crate::ToastKind::Error, error, cx),
                                    });
                                }).detach();
                            }
                        })))
            });
        });
    }).detach();
}

fn review_automation(plan: serde_json::Value, window: &mut Window, cx: &mut Context<Observer>) {
    use chatspeed_gpui::types::Command;
    use gpui_kit::component::{button::{Button, ButtonVariants}, WindowExt};
    if window.has_active_dialog(cx) { return; }
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
        Some("interval") => config["interval_minutes"].as_u64().map(|minutes| format!("{minutes}m")).unwrap_or_default(),
        Some("once") => config["run_at"].as_str().unwrap_or_default().to_owned(),
        Some("daily") => config["times"].as_array()
            .filter(|times| !times.is_empty())
            .map(|times| times.iter().filter_map(serde_json::Value::as_str).collect::<Vec<_>>().join(", "))
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
        "prompt", "prompt_file_path", "agent_id", "agent_config", "allowed_paths",
        "shell_config", "schedule_kind", "schedule_config", "continuous_context",
        "self_review", "enabled",
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
    assert_eq!(automation_title_update(&view, " New "), Some(serde_json::json!({"spec": expected, "expected_revision": 4})));
    assert_eq!(automation_schedule_input(&view), "09:00");
    for (kind, config, expected) in [
        ("daily", serde_json::json!({"times": ["09:00", "18:00"]}), "09:00, 18:00"),
        ("interval", serde_json::json!({"interval_minutes": 30}), "30m"),
        ("once", serde_json::json!({"run_at": "2030-01-02 09:30"}), "2030-01-02 09:30"),
    ] {
        let view = serde_json::json!({"schedule_kind": kind, "schedule_config": config});
        let input = automation_schedule_input(&view);
        assert_eq!(input, expected);
        assert_eq!(parse_automation_schedule(&input).map(|(kind, _)| kind), Some(kind));
    }
    assert!(automation_title_update(&view, " ").is_none());
    assert!(automation_title_update(&serde_json::json!({"revision": 4}), "New").is_none());
}

fn parse_automation_schedule(value: &str) -> Option<(&'static str, serde_json::Value)> {
    let value = value.trim();
    if let Some(minutes) = value.strip_suffix('m') {
        let minutes = minutes.trim().parse::<u32>().ok().filter(|n| *n >= 5)?;
        return Some(("interval", serde_json::json!({"interval_minutes": minutes})));
    }
    if let Some((date, time)) = value.split_once(' ') {
        let parts: Vec<_> = date.split('-').collect();
        if parts.len() == 3 && parts[0].len() == 4 && parts[1].len() == 2 && parts[2].len() == 2
            && parts.iter().all(|part| part.bytes().all(|b| b.is_ascii_digit())) && valid_daily_time(time)
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
    assert_eq!(parse_daily_schedule("18:00，09:00, 09:00"), Some(serde_json::json!({"time":"09:00","times":["09:00","18:00"]})));
    assert!(parse_daily_schedule("09:00, 24:00").is_none());
    assert!(parse_daily_schedule("").is_none());
    assert_eq!(parse_automation_schedule("30m"), Some(("interval", serde_json::json!({"interval_minutes":30}))));
    assert!(parse_automation_schedule("4m").is_none());
    assert_eq!(parse_automation_schedule("2030-01-02 09:30"), Some(("once", serde_json::json!({"run_at":"2030-01-02 09:30"}))));
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

pub(super) fn toggle_chat_hub(this: &mut Observer, window: &mut Window, cx: &mut Context<Observer>) {
    #[cfg(not(feature = "chat-hub"))]
    {
        let _ = window;
        this.not_implemented(t!("chat_hub"), cx);
    }
    #[cfg(feature = "chat-hub")]
    {
        if let Some(hub) = this.chat_hub.take() {
            hub.close(window);
            cx.notify();
            return;
        }
        let (reply, response) = tokio::sync::oneshot::channel();
        if this.worker.commands.try_send(chatspeed_gpui::types::Command::ListChatHubs(reply)).is_err() {
            return;
        }
        cx.spawn_in(window, async move |owner, cx| {
            let result = response.await;
            let _ = owner.update_in(cx, |this, window, cx| match result {
                Ok(Ok(sites)) => crate::chat_hub::select_site(sites, window, cx),
                Ok(Err(error)) => this.show_toast(crate::ToastKind::Error, error, cx),
                Err(_) => {}
            });
        }).detach();
    }
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
        cx.listener(|this, _, _, cx| this.toggle_sidebar(cx)),
    ));
    let pending = observer.pending_approvals();
    if !pending.is_empty() {
        left = left.child(approval_queue(pending, secondary, accent, skin.hover, cx));
    }
    left = left.child(sound_menu(observer, secondary, skin.hover, cx));
    #[cfg(feature = "chat-hub")]
    {
        left = left.child(div().id("chat-hub-toggle").cursor_pointer().px_2().child("ChatHub")
            .on_click(cx.listener(|this, _, window, cx| toggle_chat_hub(this, window, cx))));
    }

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
                    .rounded(px(6.))
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
            "clock",
            secondary,
            skin.hover,
            cx.listener(|this, _, window, cx| open_automation(this, window, cx)),
        ))
        .child(icon_button(
            "new-session",
            "new-chat",
            secondary,
            skin.hover,
            cx.listener(|this, _, window, cx| this.start_new_session(window, cx)),
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
            cx.listener(|this, _, window, cx| this.confirm_rewind(window, cx)),
        ))
        .when(super::window_pin::supported(window), |row| {
            row.child(super::icon_button_element(
                "pin-window",
                Icon::new("pin")
                    .size(18.)
                    .color(if observer.pinned {
                        crate::success().into()
                    } else {
                        secondary
                    })
                    .rotate(if observer.pinned { 0. } else { 45. }),
                skin.hover,
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
        .rounded(px(8.))
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
                        .py(px(6.))
                        .rounded(px(6.))
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
        .trigger(trigger(flat_button("sound-button", icon, secondary, hover)))
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
fn flat_button(id: &'static str, icon: &'static str, color: Hsla, hover: Hsla) -> impl IntoElement {
    h_flex()
        .id(id)
        .flex_none()
        .items_center()
        .justify_center()
        .w(px(26.))
        .h(px(22.))
        .rounded(px(8.))
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(Icon::new(icon).size(18.).color(color))
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
        .py(px(6.))
        .rounded(px(6.))
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
        .py(px(6.))
        .rounded(px(6.))
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
