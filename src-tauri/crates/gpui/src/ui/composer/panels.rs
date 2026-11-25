//! Configuration dialogs opened from the composer's `+` menu.
//!
//! Vue implements the model, skills, MCP and tool configuration as popovers or
//! dialogs anchored in the web page. The native window draws them as one modal
//! over the workflow frame, which keeps a long option list scrollable and lets
//! every switch repaint from the same live state the composer reads.

use gpui_kit::component::{
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    switch::Switch,
    v_flex, ActiveTheme, Disableable, Sizable,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use rust_i18n::t;
use serde_json::{json, Value};

use crate::ui::Icon;
use crate::{Observer, QuickPanel, ToolDraft};
use chatspeed_gpui::types::Command;

/// The centered modal for the active configuration panel.
pub fn overlay(observer: &Observer, window: &mut Window, cx: &mut Context<Observer>) -> AnyElement {
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
        QuickPanel::Skills => skills_panel(observer, window, cx),
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
        // The docked ChatHub page is a native view above GPUI; keep it clear.
        .pr(observer.dialog_reserved_right())
        .bg(black().opacity(0.32))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| this.close_quick_panel(cx)),
        )
        .child(
            v_flex()
                .id("composer-config-panel")
                .w(px(if matches!(panel, QuickPanel::Models) {
                    480.
                } else {
                    600.
                }))
                // Shrink inside the area left of a docked ChatHub page.
                .max_w(relative(0.92))
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
                                .rounded(crate::ui::tokens::RADIUS_MEDIUM)
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

pub(crate) fn empty_note(text: String, cx: &App) -> AnyElement {
    div()
        .w_full()
        .py_6()
        .text_size(px(12.))
        .text_color(cx.theme().muted_foreground)
        .child(text)
        .into_any_element()
}

/// A check row used for the read-only lists and the multi-select panels.
pub(crate) fn check_row(
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
        .rounded(crate::ui::tokens::RADIUS_MEDIUM)
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

/// Opens the shared model configuration card for the composer's draft.
fn model_panel(observer: &Observer, cx: &mut Context<Observer>) -> AnyElement {
    let Some(state) = observer.model_config.clone() else {
        return div().into_any_element();
    };
    let observer_entity = cx.entity();
    let save_state = state.clone();
    let save_observer = observer_entity.clone();
    let cancel_observer = observer_entity;
    let footer = super::model_config::model_config_footer(
        "model-config",
        observer.models_pending.is_none(),
        move |_, _, cx| {
            cancel_observer.update(cx, |this, cx| this.close_quick_panel(cx));
        },
        move |_, _, cx| {
            let configs = save_state.read(cx).configs.clone();
            save_observer.update(cx, |this, cx| {
                this.save_models(configs, cx);
                this.close_quick_panel(cx);
            });
        },
        cx,
    );
    super::model_config::model_config_panel(&state, footer, cx)
}

pub(crate) fn model_select_trigger(
    id: &'static str,
    label: String,
    enabled: bool,
    width: f32,
    cx: &App,
) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let foreground = cx.theme().foreground;
    h_flex()
        .id(id)
        .when(width > 0., |field| field.w(px(width)).flex_none())
        .when(width <= 0., |field| field.flex_1())
        .min_w_0()
        .items_center()
        .justify_between()
        .gap(px(4.))
        .h(px(28.))
        .px(px(8.))
        .rounded(px(4.))
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().background)
        .when(enabled, |field| field.cursor_pointer())
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .text_size(px(12.))
                .text_color(if enabled { foreground } else { muted })
                .child(label),
        )
        .child(Icon::new("caret-down").size(10.).color(muted))
        .into_any_element()
}

pub(crate) fn param_label(label: String, cx: &App) -> AnyElement {
    div()
        .flex_none()
        .text_size(px(11.))
        .text_color(cx.theme().muted_foreground)
        .child(label)
        .into_any_element()
}

pub(crate) use crate::ui::tabs::capsule_segments;

pub(crate) fn field_step(
    id: impl Into<SharedString>,
    label: &'static str,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    div()
        .id(id.into())
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .size(px(16.))
        .rounded(px(3.))
        .cursor_pointer()
        .text_size(px(10.))
        .text_color(cx.theme().muted_foreground)
        .hover(|style| style.bg(cx.theme().muted))
        .child(label)
        .on_click(on_click)
        .into_any_element()
}

/// Vue's skills dialog: an enable switch and the search field share one row.
fn skills_panel(
    observer: &Observer,
    window: &mut Window,
    cx: &mut Context<Observer>,
) -> AnyElement {
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
                .rounded(crate::ui::tokens::RADIUS_MEDIUM)
                .border_1()
                // The library adds a wide halo and a thicker edge on focus.
                // Vue only recolors the existing hairline with the primary
                // color, so the ring is disabled and the edge is ours.
                .border_color(
                    if observer
                        .skill_search
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window)
                    {
                        primary
                    } else {
                        border
                    },
                )
                .bg(crate::ui::field_bg(cx))
                .child(
                    gpui_kit::component::input::Input::new(&observer.skill_search)
                        .bordered(false)
                        .focus_bordered(false),
                ),
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
        .px(px(16.))
        .py(px(4.))
        .rounded(crate::ui::tokens::RADIUS_MEDIUM)
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
        .px(px(16.))
        .py(px(4.))
        .rounded(crate::ui::tokens::RADIUS_MEDIUM)
        .bg(primary)
        .cursor_pointer()
        .text_size(px(14.))
        .text_color(cx.theme().primary_foreground)
        .hover(|style| style.opacity(0.9))
        .child(t!("session_save").to_string())
        .on_click(cx.listener(|this, _, _, cx| {
            let (Some(session), Some(draft)) = (this.selected.clone(), this.skills_draft.clone())
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

const SHELL_RULES_KEY: &str = "shell-rules";
const SHELL_PAGE_SIZE: usize = 10;
const SHELL_RULE_FONT_SIZE: f32 = 12.;

/// Input and pagination state scoped to one opening of the tool dialog.
pub(crate) struct ShellRulesPanel {
    search: Entity<InputState>,
    pattern: Entity<InputState>,
    decision: &'static str,
    page: usize,
    scroll: ScrollHandle,
}

impl ShellRulesPanel {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Observer>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(t!("shell_rules_search").to_string()));
        let pattern = cx.new(|cx| InputState::new(window, cx).placeholder(t!("shell_rules_pattern").to_string()));
        cx.subscribe_in(&search, window, |this, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                this.set_shell_rules_page(0, cx);
            }
        }).detach();
        cx.subscribe_in(&pattern, window, |this, _, event: &InputEvent, window, cx| match event {
            InputEvent::Change => cx.notify(),
            InputEvent::PressEnter { secondary: false, shift: false } => this.add_shell_rule(window, cx),
            _ => {}
        }).detach();
        Self { search, pattern, decision: "review", page: 0, scroll: ScrollHandle::new() }
    }
}

fn policy_field(config: &Value) -> &Value {
    config.get("shellPolicy").unwrap_or(&config["shell_policy"])
}

fn default_shell_rules(agent: &Value) -> Option<Vec<Value>> {
    match policy_field(agent) {
        Value::Array(rules) => Some(rules.clone()),
        Value::String(rules) => serde_json::from_str(rules).ok(),
        Value::Null => Some(Vec::new()),
        _ => None,
    }
}

fn shell_rule_pattern(rule: &Value) -> Option<&str> {
    rule["pattern"].as_str().filter(|pattern| !pattern.trim().is_empty())
}

fn shell_rule_description(rule: &Value) -> &str {
    rule["description"].as_str().unwrap_or_default()
}

fn shell_rule_page<'a>(rules: &'a [Value], query: &str, requested: usize) -> (Vec<&'a Value>, usize, usize) {
    let query = query.trim().to_lowercase();
    let mut matching: Vec<_> = rules.iter().filter_map(|rule| {
        let pattern = shell_rule_pattern(rule)?;
        (pattern.to_lowercase().contains(&query) || shell_rule_description(rule).to_lowercase().contains(&query))
            .then_some((pattern, rule))
    }).collect();
    matching.sort_by(|a, b| a.0.cmp(b.0));
    let pages = matching.len().div_ceil(SHELL_PAGE_SIZE).max(1);
    let page = requested.min(pages - 1);
    let visible = matching.into_iter().skip(page * SHELL_PAGE_SIZE).take(SHELL_PAGE_SIZE)
        .map(|(_, rule)| rule).collect();
    (visible, page, pages)
}

fn append_shell_rule(rules: &mut Vec<Value>, pattern: &str, decision: &str) -> bool {
    let pattern = pattern.trim();
    if pattern.is_empty() || rules.iter().any(|rule| shell_rule_pattern(rule) == Some(pattern)) {
        return false;
    }
    rules.push(json!({"pattern": pattern, "decision": decision}));
    true
}

impl Observer {
    fn shell_rules(&self) -> Vec<Value> {
        policy_field(&self.agent_config()).as_array().cloned().unwrap_or_default()
    }

    fn can_save_shell_rules(&self) -> bool {
        !self.options.demo && self.quick_panel == Some(QuickPanel::Tools)
            && self.tool_panel_tab == 1 && self.confirm.is_none()
            && self.selected.as_ref().is_some_and(|session| {
                !self.config_pending.contains(&(session.clone(), SHELL_RULES_KEY))
            })
    }

    fn save_shell_rules(&mut self, rules: Vec<Value>, cx: &mut Context<Self>) -> bool {
        let Some(session) = self.selected.clone().filter(|_| self.can_save_shell_rules()) else {
            return false;
        };
        let mut config = self.agent_config();
        if let Some(fields) = config.as_object_mut() {
            fields.remove("shell_policy");
        }
        config["shellPolicy"] = Value::Array(rules);
        let command = Command::SetShellRules { session: session.clone(), config };
        if !self.submit_config(&session, SHELL_RULES_KEY, command, cx) {
            self.show_toast(crate::ToastKind::Error, t!("shell_rules_save_failed").to_string(), cx);
            return false;
        }
        true
    }

    fn set_shell_rules_page(&mut self, page: usize, cx: &mut Context<Self>) {
        if let Some(panel) = self.shell_rules_panel.as_mut() {
            panel.page = page;
            panel.scroll.set_offset(point(px(0.), px(0.)));
            cx.notify();
        }
    }

    fn add_shell_rule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_save_shell_rules() { return; }
        let Some(panel) = &self.shell_rules_panel else { return; };
        let input = panel.pattern.clone();
        let pattern = input.read(cx).value().trim().to_owned();
        if pattern.is_empty() { return; }
        let mut rules = self.shell_rules();
        if !append_shell_rule(&mut rules, &pattern, panel.decision) {
            self.show_toast(crate::ToastKind::Info, t!("shell_rules_duplicate").to_string(), cx);
            return;
        }
        if self.save_shell_rules(rules, cx) {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
    }

    fn confirm_shell_rules(&mut self, import: bool, cx: &mut Context<Self>) {
        let Some(session) = self.selected.clone().filter(|_| self.can_save_shell_rules()) else { return; };
        self.confirm = Some(crate::ui::confirm::ConfirmRequest {
            kind: crate::ui::confirm::ConfirmKind::ReplaceShellRules { session, import },
            title: if import { t!("shell_rules_import") } else { t!("shell_rules_clear") }.to_string(),
            message: if import { t!("shell_rules_import_confirm") } else { t!("shell_rules_clear_confirm") }.to_string(),
            confirm_label: t!("confirm").to_string(),
            danger: !import,
        });
        cx.notify();
    }

    pub(crate) fn replace_shell_rules(&mut self, session: &str, import: bool, cx: &mut Context<Self>) {
        if self.selected.as_deref() != Some(session) || !self.can_save_shell_rules() { return; }
        let rules = if import {
            let Some(rules) = self.current_agent().and_then(default_shell_rules) else {
                self.show_toast(crate::ToastKind::Error, t!("shell_rules_defaults_unavailable").to_string(), cx);
                return;
            };
            rules
        } else {
            Vec::new()
        };
        if self.save_shell_rules(rules, cx) {
            self.set_shell_rules_page(0, cx);
        }
    }
}

#[cfg(test)]
mod shell_rules_tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn shell_rule_search_and_pagination_leave_the_full_policy_intact() {
        let mut rules: Vec<_> = (0..23)
            .rev()
            .map(|id| json!({"pattern": format!("Rule-{id:02}")}))
            .collect();
        rules[0]["description"] = json!("Example description");
        let (visible, page, pages) = shell_rule_page(&rules, "", 9);
        assert_eq!((page, pages, visible.len()), (2, 3, 3));
        assert_eq!(visible[0]["pattern"], "Rule-20");
        let (visible, page, pages) = shell_rule_page(&rules, " EXAMPLE ", 2);
        assert_eq!((page, pages), (0, 1));
        assert_eq!(visible, vec![&rules[0]]);
        assert_eq!(shell_rule_page(&rules, "rule-0", 0).0.len(), 10);
        assert_eq!(rules.len(), 23);
    }

    #[::core::prelude::v1::test]
    fn shell_rule_add_trims_and_rejects_empty_or_duplicate_patterns() {
        let mut rules = vec![json!({"pattern": "first", "decision": "deny", "description": "Keep"})];
        assert!(!append_shell_rule(&mut rules, " first ", "allow"));
        assert!(!append_shell_rule(&mut rules, "   ", "allow"));
        assert!(append_shell_rule(&mut rules, " second ", "review"));
        assert_eq!(rules[0]["description"], "Keep");
        assert_eq!(rules[1], json!({"pattern": "second", "decision": "review"}));
    }

    #[::core::prelude::v1::test]
    fn shell_rule_import_reads_agent_defaults_and_distinguishes_invalid_data() {
        let rules = json!([{"pattern": "first", "decision": "deny"}]);
        assert_eq!(default_shell_rules(&json!({"shell_policy": rules.to_string()})), rules.as_array().cloned());
        assert_eq!(default_shell_rules(&json!({"shellPolicy": rules})), rules.as_array().cloned());
        assert_eq!(default_shell_rules(&json!({"shell_policy": null})), Some(vec![]));
        assert_eq!(default_shell_rules(&json!({"shell_policy": "invalid"})), None);
    }
}

fn shell_decisions(
    id: &str,
    decision: &str,
    on_select: impl Fn(&'static str, &mut Window, &mut App) + 'static,
) -> crate::ui::tabs::CapsuleSegments {
    crate::ui::tabs::capsule_segments_sized(
        id,
        vec![
            ("allow", t!("shell_decision_allow").to_string()),
            ("deny", t!("shell_decision_deny").to_string()),
            ("review", t!("shell_decision_review").to_string()),
        ],
        decision,
        22., // Include the two hairlines in the 24px control height.
        on_select,
    )
    .font_size(SHELL_RULE_FONT_SIZE)
}

fn shell_rules_content(observer: &Observer, rules: &[Value], cx: &mut Context<Observer>) -> AnyElement {
    let Some(panel) = &observer.shell_rules_panel else { return div().into_any_element(); };
    let (visible, page, pages) = shell_rule_page(rules, &panel.search.read(cx).value(), panel.page);
    let writable = observer.can_save_shell_rules();
    let can_add = writable && !panel.pattern.read(cx).value().trim().is_empty();
    let can_import = writable && observer.current_agent().is_some();
    let muted = cx.theme().muted_foreground;
    let mut list = v_flex()
        .id("shell-rule-list")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .track_scroll(&panel.scroll);
    if visible.is_empty() {
        list = list.child(empty_note(t!("shell_rules_empty").to_string(), cx));
    }
    for (index, rule) in visible.into_iter().enumerate() {
        let Some(pattern) = shell_rule_pattern(rule).map(str::to_owned) else { continue; };
        let description = shell_rule_description(rule).to_owned();
        let decision = rule["decision"].as_str().unwrap_or("review");
        let target = pattern.clone();
        let entity = cx.entity();
        list = list.child(
            h_flex()
                .flex_none()
                .min_w_0()
                .items_center()
                .gap_2()
                .px_2()
                .py_1()
                .rounded(crate::ui::tokens::RADIUS_MEDIUM)
                .hover(|style| style.bg(cx.theme().muted))
                .child(v_flex().flex_1().min_w_0().child(
                    div().min_w_0().whitespace_nowrap().text_ellipsis()
                        .text_size(px(SHELL_RULE_FONT_SIZE)).font_family(crate::MONO_FONT).child(pattern.clone()),
                ).when(!description.is_empty(), |element| element.child(
                    div().min_w_0().whitespace_nowrap().text_ellipsis()
                        .text_size(px(10.)).text_color(muted).child(description),
                )))
                .child(div().flex_none().when(!writable, |element| element.opacity(0.55)).child(
                    shell_decisions(&format!("shell-decision-{index}"), decision, move |value, _, cx| {
                        entity.update(cx, |this, cx| {
                            let mut rules = this.shell_rules();
                            let mut changed = false;
                            for rule in &mut rules {
                                if shell_rule_pattern(rule) == Some(target.as_str()) && rule["decision"] != value {
                                    rule["decision"] = Value::from(value);
                                    changed = true;
                                }
                            }
                            if changed { this.save_shell_rules(rules, cx); }
                        });
                    }),
                ))
                .child(Button::new(("shell-rule-remove", index)).small().ghost()
                    .disabled(!writable)
                    .accessibility_label(t!("shell_rules_remove").to_string())
                    .child(Icon::new("trash").size(12.).color(muted))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let mut rules = this.shell_rules();
                        let previous = rules.len();
                        rules.retain(|rule| shell_rule_pattern(rule) != Some(pattern.as_str()));
                        if rules.len() != previous { this.save_shell_rules(rules, cx); }
                    }))),
        );
    }
    let entity = cx.entity();
    v_flex().flex_1().min_h_0().gap_2()
        .child(Input::new(&panel.search).small().cleanable(true))
        .child(h_flex().gap_2().items_center()
            .child(div().flex_1().min_w_0().child(Input::new(&panel.pattern).small().cleanable(true)))
            .child(shell_decisions("shell-new-decision", panel.decision, move |decision, _, cx| {
                entity.update(cx, |this, cx| {
                    if let Some(panel) = &mut this.shell_rules_panel {
                        panel.decision = decision;
                        cx.notify();
                    }
                });
            }))
            .child(Button::new("shell-add").small().primary().label(t!("shell_rules_add").to_string())
                .disabled(!can_add).on_click(cx.listener(|this, _, window, cx| this.add_shell_rule(window, cx)))))
        .child(list)
        .when(pages > 1, |content| content.child(h_flex().flex_none().justify_center().items_center().gap_2()
            .child(Button::new("shell-prev").small().ghost().disabled(page == 0)
                .accessibility_label(t!("shell_rules_previous").to_string()).child(Icon::new("caret-right").size(12.).rotate(180.))
                .on_click(cx.listener(move |this, _, _, cx| this.set_shell_rules_page(page.saturating_sub(1), cx))))
            .child(div().text_size(px(12.)).text_color(muted).child(format!("{} / {pages}", page + 1)))
            .child(Button::new("shell-next").small().ghost().disabled(page + 1 >= pages)
                .accessibility_label(t!("shell_rules_next").to_string()).child(Icon::new("caret-right").size(12.))
                .on_click(cx.listener(move |this, _, _, cx| this.set_shell_rules_page(page + 1, cx))))))
        .child(h_flex().flex_none().justify_end().gap_2().pt_2().border_t_1().border_color(cx.theme().border)
            .child(Button::new("shell-import").small().label(t!("shell_rules_import").to_string())
                .disabled(!can_import).on_click(cx.listener(|this, _, _, cx| this.confirm_shell_rules(true, cx))))
            .child(Button::new("shell-clear").small().danger().label(t!("shell_rules_clear").to_string())
                .disabled(!writable || rules.is_empty()).on_click(cx.listener(|this, _, _, cx| this.confirm_shell_rules(false, cx)))))
        .into_any_element()
}

/// Vue's tabs: available tools, auto approval and shell rules.
fn tool_panel(observer: &Observer, cx: &mut Context<Observer>) -> AnyElement {
    const AUTO_APPROVE_WIDTH: Pixels = px(88.);
    let Some(draft) = observer.tool_draft.clone() else {
        return empty_note(t!("tools_empty").to_string(), cx);
    };
    let tab = observer.tool_panel_tab.min(1);
    let ids = agent_tool_ids(observer);
    let rules = observer.shell_rules();
    let muted = cx.theme().muted_foreground;

    let observer_entity = cx.entity();
    let tabs = crate::ui::tabs::underline_tabs(
        "tool-tab",
        vec![
            format!("{} ({})", t!("tool_tab_available"), ids.len()),
            format!("{} ({})", t!("tool_tab_shell"), rules.len()),
        ],
        tab,
        move |index, window, cx| {
            observer_entity.update(cx, |this, cx| {
                this.tool_panel_tab = index;
                cx.notify();
            });
            window.refresh();
        },
        cx,
    );

    let mut body = v_flex()
        .id("tool-panel-body")
        .w_full()
        .min_w_0()
        .flex_1()
        .min_h_0()
        .when(tab == 0, |body| body.overflow_y_scroll())
        .px(crate::ui::tokens::SPACE_16)
        .pt(crate::ui::tokens::SPACE_16)
        .pb_3();
    match tab {
        0 => {
            if ids.is_empty() {
                body = body.child(empty_note(t!("tools_empty").to_string(), cx));
            } else {
                body = body.child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .items_center()
                        .gap(px(8.))
                        .px_2()
                        .py_1()
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
                                .w(AUTO_APPROVE_WIDTH)
                                .flex_none()
                                .whitespace_nowrap()
                                .text_size(px(10.))
                                .text_align(TextAlign::Center)
                                .text_color(muted)
                                .child(t!("mcp_auto_approve").to_string()),
                        ),
                );
            }
            for id in ids {
                let available = draft.available.contains(&id);
                let approved = draft.auto_approve.contains(&id);
                let name = tool_display_name(observer, &id);
                let detail = if name == id { String::new() } else { name };
                let id_for_available = id.clone();
                let id_for_approve = id.clone();
                body = body.child(
                    h_flex()
                        .id(SharedString::from(format!("tool-row-{id}")))
                        .w_full()
                        .min_w_0()
                        .items_center()
                        .gap(px(8.))
                        .px_2()
                        .py_1()
                        .rounded(crate::ui::tokens::RADIUS_MEDIUM)
                        .hover(|style| style.bg(cx.theme().muted.opacity(0.7)))
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
                                        .child(id.clone()),
                                )
                                .when(!detail.is_empty(), |column| {
                                    column.child(
                                        div()
                                            .min_w_0()
                                            .whitespace_nowrap()
                                            .text_ellipsis()
                                            .text_size(px(11.))
                                            .text_color(muted)
                                            .child(detail),
                                    )
                                }),
                        )
                        .child(
                            div().w(px(64.)).flex_none().flex().items_center().justify_center().child(
                                Switch::new(SharedString::from(format!(
                                    "tool-available-{id}"
                                )))
                                .checked(available)
                                .small()
                                .on_click(cx.listener(move |this, checked, _, cx| {
                                    if *checked
                                        != this.tool_draft.as_ref().is_some_and(|draft| {
                                            draft.available.contains(&id_for_available)
                                        })
                                    {
                                        this.toggle_tool(
                                            &id_for_available,
                                            false,
                                            ToolFlag::Available,
                                            cx,
                                        );
                                    }
                                })),
                            ),
                        )
                        .child(
                            div().w(AUTO_APPROVE_WIDTH).flex_none().flex().items_center().justify_center().child(
                                Switch::new(SharedString::from(format!("tool-approve-{id}")))
                                    .checked(approved)
                                    .small()
                                    // Approval is a subset of availability, so the
                                    // grant only makes sense once the tool is on.
                                    .disabled(!approved && !available)
                                    .on_click(cx.listener(move |this, checked, _, cx| {
                                        if *checked
                                            != this.tool_draft.as_ref().is_some_and(|draft| {
                                                draft.auto_approve.contains(&id_for_approve)
                                            })
                                        {
                                            this.toggle_tool(
                                                &id_for_approve,
                                                false,
                                                ToolFlag::AutoApprove,
                                                cx,
                                            );
                                        }
                                    })),
                            ),
                        ),
                );
            }
        }
        _ => body = body.child(shell_rules_content(observer, &rules, cx)),
    }

    v_flex()
        .id("tool-panel")
        .w_full()
        .min_w_0()
        .h(px(460.))
        .child(
            div()
                .w_full()
                .px(crate::ui::tokens::SPACE_16)
                .pt(crate::ui::tokens::SPACE_16)
                .child(tabs),
        )
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
    for id in draft
        .mcp_available
        .iter()
        .chain(draft.mcp_auto_approve.iter())
    {
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
        let mut row = h_flex()
            .id(SharedString::from(format!("mcp-tool-row-{id}")))
            .w_full()
            .min_w_0()
            .items_center()
            .gap(px(8.))
            .px_2()
            .py_1()
            .rounded(crate::ui::tokens::RADIUS_MEDIUM)
            // A faint row wash; `muted` follows the active light/dark theme.
            .hover(|style| style.bg(cx.theme().muted.opacity(0.7)));
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
            // Vue disables the dependent switches until the tool is available;
            // turning one off again stays possible so a stale grant can be
            // revoked even when the server stopped reporting the tool.
            let locked =
                !checked && !available && !matches!(flag, ToolFlag::Available);
            row = row.child(
                div()
                    .w(px(64.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        Switch::new(SharedString::from(format!("mcp-{flag:?}-{id}")))
                            .checked(checked)
                            .small()
                            .disabled(locked)
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
        // Vue only lets an available tool carry the dependent flags, and the
        // runtime enforces the same subset rule. Refuse the grant here so a
        // disabled-looking switch can never send a config that gets normalized
        // away.
        let (available, checked) = if mcp {
            (
                draft.mcp_available.contains(id),
                match flag {
                    ToolFlag::Available => draft.mcp_available.contains(id),
                    ToolFlag::AutoApprove => draft.mcp_auto_approve.contains(id),
                    ToolFlag::AutoExpand => draft.mcp_auto_expand.contains(id),
                },
            )
        } else {
            (
                draft.available.contains(id),
                match flag {
                    ToolFlag::Available => draft.available.contains(id),
                    _ => draft.auto_approve.contains(id),
                },
            )
        };
        if !checked && !available && !matches!(flag, ToolFlag::Available) {
            return;
        }
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
        config["mcpTools"] = crate::mcp_tools_config(
            &draft.mcp_available,
            &draft.mcp_auto_approve,
            &draft.mcp_auto_expand,
        );
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
