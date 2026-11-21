//! Workflow sidebar: the navigation rail, the tabbed session panel and the
//! compact column it collapses into.
//!
//! Mirrors `src/components/workflow/WorkflowSidebar.vue` and the rail in
//! `src/views/Workflow.vue`: the rail keeps the tasks and automations at the top
//! and plugins, ChatHub and the terminal at the bottom; the expanded panel starts
//! with the tasks/automation and authorized-paths tabs; the collapsed column
//! moves the active and recent conversations between those two icon groups.

use gpui_kit::component::input::Input;
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use rust_i18n::t;

use crate::ui::trigger;
use crate::ui::Icon;
use crate::ui::{surfaces, Surfaces, RAIL_WIDTH, SIDEBAR_COLLAPSED_WIDTH};
use crate::{accent, danger, info, success, violet, warning, Observer};
use chatspeed_gpui::types::{root_name, Session};

/// Which list the rail selects; Vue also hides the other tab while it is active.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RailTab {
    Tasks,
    Automation,
}

/// Tab shown in the expanded sidebar (`workflowSidebarActiveTab`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SidebarTab {
    Tasks,
    Automation,
    Files,
}

/// Session status groups, from Vue's `getWorkflowStatusClass`.
fn status_class(status: &str) -> &'static str {
    match status.to_lowercase().as_str() {
        "thinking" | "executing" | "auditing" | "running" | "stopping" => "running",
        "paused"
        | "awaiting_user"
        | "awaiting_approval"
        | "awaiting_auto_approval"
        | "awaiting_sub_agent" => "waiting",
        "error" | "failed" => "failed",
        "cancelled" | "interrupted" => "cancelled",
        "completed" => "completed",
        _ => "pending",
    }
}

fn status_color(class: &str) -> Rgba {
    match class {
        "running" => warning(),
        "waiting" => accent(),
        "completed" => success(),
        "failed" => danger(),
        "cancelled" => violet(),
        _ => info(),
    }
}

fn status_label(status: &str) -> String {
    let key = match status.to_lowercase().as_str() {
        "thinking" => "status_thinking",
        "executing" => "status_executing",
        "auditing" => "status_auditing",
        "running" => "status_running",
        "stopping" => "status_stopping",
        "paused" => "status_paused",
        "awaiting_user" => "status_awaiting_user",
        "awaiting_approval" | "awaiting_auto_approval" => "status_awaiting_approval",
        "awaiting_sub_agent" => "status_awaiting_sub_agent",
        "completed" => "status_completed",
        "error" => "status_error",
        "failed" => "status_failed",
        "cancelled" => "status_cancelled",
        "interrupted" => "status_interrupted",
        _ => "status_pending",
    };
    t!(key).to_string()
}

/// Vue keeps every non-terminal workflow in the active group.
fn is_active(session: &Session) -> bool {
    !matches!(
        session.status.to_lowercase().as_str(),
        "completed" | "error" | "failed" | "cancelled" | "interrupted"
    )
}

/// Badge text: the first two characters of the primary authorized folder.
fn initials(session: &Session) -> String {
    let Some(root) = session.primary_root() else {
        return "—".into();
    };
    let name = root_name(&root);
    let text: String = name.chars().take(2).collect();
    if text.is_empty() {
        "—".into()
    } else {
        text.to_uppercase()
    }
}

fn heading(session: &Session) -> String {
    let heading = session.heading();
    if heading.is_empty() {
        t!("untitled").to_string()
    } else {
        heading
    }
}

fn root_label(session: &Session) -> Option<String> {
    session
        .primary_root()
        .map(|root| root_name(&root).to_owned())
}

/// Vue filters on the title, then the user query, then the untitled label, and
/// narrows the list to one primary root before the text search applies.
fn matches_filters(session: &Session, query: &str, root: Option<&str>) -> bool {
    if let Some(root) = root {
        if root_label(session).as_deref() != Some(root) {
            return false;
        }
    }
    if query.is_empty() {
        return true;
    }
    if session.title.to_lowercase().contains(query)
        || session.user_query.to_lowercase().contains(query)
    {
        return true;
    }
    session.title.trim().is_empty()
        && session.user_query.trim().is_empty()
        && t!("untitled").to_lowercase().contains(query)
}

/// Renders the sidebar for the current collapse state.
pub fn sidebar(observer: &Observer, window: &Window, cx: &mut Context<Observer>) -> AnyElement {
    let skin = surfaces(observer.appearance_dark);
    // The sidebar is flush with the window's bottom-left corner, so it rounds
    // that corner itself: GPUI clips children to a rectangular mask.
    let radii = crate::ui::frame::inner_corner_radii(window, cx);
    if observer.sidebar_collapsed {
        compact(observer, &skin, radii.bottom_left, cx)
    } else {
        h_flex()
            .h_full()
            .flex_none()
            .child(rail(observer, &skin, radii.bottom_left, cx))
            .child(panel(observer, &skin, cx))
            .into_any_element()
    }
}

/// The 50px navigation rail shown next to the expanded panel.
fn rail(
    observer: &Observer,
    skin: &Surfaces,
    radius: Pixels,
    cx: &mut Context<Observer>,
) -> AnyElement {
    let border = cx.theme().border;
    v_flex()
        .id("rail")
        .w(px(RAIL_WIDTH))
        .h_full()
        .flex_none()
        .items_center()
        .gap(px(4.))
        .py_2()
        .bg(skin.deep)
        .border_r_1()
        .border_color(border)
        .rounded_bl(radius)
        .child(rail_tab_button(
            "rail-tasks",
            "skill-plan3",
            observer.rail_tab == RailTab::Tasks,
            true,
            skin,
            cx,
            |this, cx| this.select_rail_tab(RailTab::Tasks, cx),
        ))
        .child(rail_tab_button(
            "rail-automation",
            "clock",
            observer.rail_tab == RailTab::Automation,
            true,
            skin,
            cx,
            |this, cx| this.select_rail_tab(RailTab::Automation, cx),
        ))
        .child(
            v_flex()
                .mt_auto()
                .items_center()
                .gap(px(4.))
                .child(rail_action(
                    "rail-plugin",
                    "plugin",
                    skin,
                    cx,
                    |this, cx| this.not_implemented(t!("plugin_panel"), cx),
                ))
                .child(rail_action(
                    "rail-chathub",
                    "connected",
                    skin,
                    cx,
                    |this, cx| this.not_implemented(t!("chat_hub"), cx),
                ))
                // Vue keeps the terminal entry separately so the docked ChatHub
                // page is only opened by its own entry.
                .child(rail_action(
                    "rail-terminal",
                    "bash",
                    skin,
                    cx,
                    super::terminal::toggle,
                )),
        )
        .into_any_element()
}

/// A rail-sized icon button. Vue only draws the leading selection bar in the
/// expanded rail; the compact column marks its conversation badges instead.
fn rail_tab_button(
    id: &'static str,
    icon: &'static str,
    active: bool,
    bar: bool,
    skin: &Surfaces,
    cx: &mut Context<Observer>,
    run: impl Fn(&mut Observer, &mut Context<Observer>) + 'static,
) -> AnyElement {
    let hover = skin.hover;
    let active_bg = skin.active;
    let secondary = cx.theme().muted_foreground;
    let primary = cx.theme().primary;
    div()
        .id(id)
        .relative()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .size(px(36.))
        .rounded(px(8.))
        .cursor_pointer()
        .when(active, |view| view.bg(active_bg))
        .when(!active, |view| view.hover(move |style| style.bg(hover)))
        .child(
            Icon::new(icon)
                .size(18.)
                .color(if active { primary } else { secondary }),
        )
        // Vue draws the selection bar on the rail's leading edge.
        .when(active && bar, |view| {
            view.child(
                div()
                    .absolute()
                    .left(px(-7.))
                    .top(px(10.))
                    .w(px(3.))
                    .h(px(16.))
                    .rounded_r(px(4.))
                    .bg(primary),
            )
        })
        .on_click(cx.listener(move |this, _, _, cx| run(this, cx)))
        .into_any_element()
}

fn rail_action(
    id: &'static str,
    icon: &'static str,
    skin: &Surfaces,
    cx: &mut Context<Observer>,
    run: impl Fn(&mut Observer, &mut Context<Observer>) + 'static,
) -> AnyElement {
    let hover = skin.hover;
    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .size(px(36.))
        .rounded(px(8.))
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(Icon::new(icon).size(18.).color(cx.theme().muted_foreground))
        .on_click(cx.listener(move |this, _, _, cx| run(this, cx)))
        .into_any_element()
}

/// The expanded panel: tab strip first, then the selected tab's content.
fn panel(observer: &Observer, skin: &Surfaces, cx: &mut Context<Observer>) -> AnyElement {
    let border = cx.theme().border;
    let background = cx.theme().background;

    let tasks_tab = SidebarTab::Tasks;
    let automation_tab = SidebarTab::Automation;
    let active = observer.visible_sidebar_tab();
    let list_tab = match observer.rail_tab {
        RailTab::Tasks => (tasks_tab, t!("tab_tasks").to_string()),
        RailTab::Automation => (automation_tab, t!("tab_automation").to_string()),
    };

    let strip = h_flex()
        .id("sidebar-tabs")
        .w_full()
        .h(px(40.))
        .flex_none()
        .items_center()
        .gap_2()
        .px(px(16.))
        .border_b_1()
        .border_color(border)
        .bg(background)
        .child(tab_button(
            "tab-list",
            list_tab.1,
            active == list_tab.0,
            cx,
            move |this, cx| this.select_sidebar_tab(list_tab.0, cx),
        ))
        .child(tab_button(
            "tab-files",
            t!("tab_paths").to_string(),
            active == SidebarTab::Files,
            cx,
            |this, cx| this.select_sidebar_tab(SidebarTab::Files, cx),
        ));

    let body = match active {
        SidebarTab::Files => crate::ui::file_tree::tree(observer, cx),
        _ => match observer.rail_tab {
            RailTab::Automation => automation_list(observer, cx),
            RailTab::Tasks => session_list(observer, skin, cx),
        },
    };

    v_flex()
        .id("sidebar-panel")
        .w(px(observer.sidebar_width))
        .h_full()
        .flex_none()
        .min_w_0()
        .border_r_1()
        .border_color(border)
        .bg(background)
        .child(strip)
        .when(active != SidebarTab::Files, |panel| {
            panel.child(search_header(observer, cx))
        })
        .child(body)
        .into_any_element()
}

/// Vue's round search field with the primary-root filter in its suffix.
fn search_header(observer: &Observer, cx: &mut Context<Observer>) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    h_flex()
        .w_full()
        .flex_none()
        .px(px(16.))
        .py_2()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .rounded_full()
                .bg(cx.theme().muted)
                .child(
                    Input::new(&observer.session_search)
                        .appearance(false)
                        .bordered(false)
                        .h(px(30.))
                        .prefix(Icon::new("search").size(13.).color(muted))
                        .suffix(root_filter(observer, cx)),
                ),
        )
        .into_any_element()
}

/// The caret-down menu that narrows the list to one primary authorized root.
fn root_filter(observer: &Observer, cx: &mut Context<Observer>) -> AnyElement {
    let selected = observer.session_root_filter.clone();
    let active = selected.is_some();
    let muted = cx.theme().muted_foreground;
    let primary = cx.theme().primary;
    let hover = surfaces(observer.appearance_dark).hover;
    let mut roots: Vec<String> = observer.sessions.iter().filter_map(root_label).collect();
    roots.sort_by_key(|root| root.to_lowercase());
    roots.dedup();

    let entries: Vec<(String, Option<String>)> =
        std::iter::once((t!("root_filter_all").to_string(), None))
            .chain(roots.into_iter().map(|root| (root.clone(), Some(root))))
            .collect();
    let observer_entity = cx.entity();

    Popover::new("session-root-filter")
        .trigger(trigger(
            div()
                .id("session-root-filter-trigger")
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .size(px(20.))
                .rounded_full()
                .cursor_pointer()
                .hover(move |style| style.bg(hover))
                .child(Icon::new("caret-down").size(12.).color(if active {
                    primary
                } else {
                    muted
                })),
        ))
        .content(move |_, window, cx| {
            let popover = cx.entity();
            // The trigger sits near the top of the window, so the menu may only
            // use half of it; longer root lists scroll instead of running off
            // screen, which is what Element Plus does for the Vue dropdown.
            let max_height = (window.viewport_size().height.as_f32() * 0.5).max(160.);
            let mut list = v_flex().id("root-filter-list").w(px(200.)).gap(px(2.));
            for (label, value) in entries.clone() {
                let popover = popover.clone();
                let value = value.clone();
                let selected = selected == value;
                list = list.child(
                    h_flex()
                        .id(SharedString::from(format!("root-filter-{label}")))
                        .w_full()
                        .min_w_0()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .py_1()
                        .rounded(px(4.))
                        .cursor_pointer()
                        .hover(|style| style.bg(cx.theme().muted))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_ellipsis()
                                .text_size(px(12.))
                                .text_color(if selected {
                                    cx.theme().primary
                                } else {
                                    cx.theme().foreground
                                })
                                .child(label),
                        )
                        .when(selected, |row| {
                            row.child(Icon::new("check").size(12.).color(cx.theme().primary))
                        })
                        .on_click({
                            let observer_entity = observer_entity.clone();
                            move |_, window, cx: &mut App| {
                                popover.update(cx, |state, cx| state.dismiss(window, cx));
                                observer_entity.update(cx, |this, cx| {
                                    this.set_session_root_filter(value.clone(), cx)
                                });
                            }
                        }),
                );
            }
            v_flex()
                .id("root-filter-scroll")
                .w(px(200.))
                .max_h(px(max_height))
                .overflow_y_scroll()
                .child(list)
                .into_any_element()
        })
        .into_any_element()
}

/// The 4px band on the panel's trailing edge; the window follows the drag.
///
/// The caller passes whether a drag is running: reading the observer here
/// would re-enter the entity the window is already rendering.
pub fn resize_band(dragging: bool, cx: &mut Context<Observer>) -> AnyElement {
    let accent = cx.theme().primary;
    div()
        .id("sidebar-resize")
        .flex_none()
        .h_full()
        .w(px(4.))
        .cursor_col_resize()
        .when(dragging, |band| band.bg(accent.opacity(0.35)))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, event: &MouseDownEvent, window, cx| {
                this.begin_sidebar_drag(event.position.x, cx);
                window.prevent_default();
            }),
        )
        .into_any_element()
}

fn tab_button(
    id: &'static str,
    label: String,
    active: bool,
    cx: &mut Context<Observer>,
    run: impl Fn(&mut Observer, &mut Context<Observer>) + 'static,
) -> AnyElement {
    let primary = cx.theme().primary;
    let foreground = cx.theme().foreground;
    v_flex()
        .id(id)
        .h_full()
        .flex_none()
        .justify_center()
        .items_center()
        .gap(px(6.))
        .px(px(8.))
        .cursor_pointer()
        .child(
            div()
                .text_size(px(14.))
                .when(active, |view| {
                    view.text_color(primary).font_weight(FontWeight::MEDIUM)
                })
                .when(!active, |view| view.text_color(foreground))
                .child(label),
        )
        .child(
            div()
                .w_full()
                .h(px(2.))
                .rounded_full()
                .when(active, |view| view.bg(primary)),
        )
        .on_click(cx.listener(move |this, _, _, cx| run(this, cx)))
        .into_any_element()
}

/// Active and recent transcripts, grouped the way Vue lists them.
fn session_list(observer: &Observer, skin: &Surfaces, cx: &mut Context<Observer>) -> AnyElement {
    let query = observer
        .session_search
        .read(cx)
        .value()
        .trim()
        .to_lowercase();
    let filter = observer.session_root_filter.as_deref();
    let active: Vec<&Session> = observer
        .sessions
        .iter()
        .filter(|session| matches_filters(session, &query, filter) && is_active(session))
        .collect();
    let recent: Vec<&Session> = observer
        .sessions
        .iter()
        .filter(|session| matches_filters(session, &query, filter) && !is_active(session))
        .collect();

    let mut list = v_flex()
        .id("session-list")
        .flex_1()
        .min_h_0()
        .w_full()
        .overflow_y_scroll()
        .px(px(16.))
        .py_2()
        .gap(px(2.));

    if !active.is_empty() {
        for session in &active {
            list = list.child(session_row(observer, session, skin, cx));
        }
    }
    if !recent.is_empty() {
        list = list
            .child(div().w_full().h(px(1.)).my_2().bg(cx.theme().border))
            .children(
                recent
                    .iter()
                    .map(|session| session_row(observer, session, skin, cx)),
            );
    }
    if active.is_empty() && recent.is_empty() {
        list = list.child(
            div()
                .px_2()
                .py_1()
                .text_size(px(11.))
                .text_color(cx.theme().muted_foreground)
                .child(t!("no_sessions").to_string()),
        );
    }
    list.into_any_element()
}

fn session_row(
    observer: &Observer,
    session: &Session,
    skin: &Surfaces,
    cx: &mut Context<Observer>,
) -> AnyElement {
    let selected = observer.selected.as_deref() == Some(session.id.as_str());
    let id = session.id.clone();
    let class = status_class(&session.status);
    let dot = status_color(class);
    let hover = skin.hover;
    let active_bg = skin.active;
    let primary = cx.theme().primary;
    let foreground = cx.theme().foreground;
    let secondary = cx.theme().muted_foreground;
    let root = root_label(session);

    v_flex()
        .id(format!("session-{}", session.id))
        .w_full()
        .min_w_0()
        .gap(px(4.))
        .px(px(16.))
        .py_2()
        .mb(px(2.))
        .rounded(px(8.))
        .cursor_pointer()
        .when(selected, |view| view.bg(active_bg))
        .when(!selected, |view| view.hover(move |style| style.bg(hover)))
        .child(
            div()
                .min_w_0()
                .text_size(px(14.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(if selected { primary } else { foreground })
                .text_ellipsis()
                .child(heading(session)),
        )
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .items_center()
                .gap_2()
                .child(
                    h_flex()
                        .flex_none()
                        .items_center()
                        .gap_1()
                        .child(div().w(px(6.)).h(px(6.)).rounded_full().bg(dot))
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(if selected { primary } else { secondary })
                                .child(status_label(&session.status)),
                        ),
                )
                .when_some(root, |row, root| {
                    row.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(10.))
                            .text_color(foreground)
                            .text_align(TextAlign::Right)
                            .text_ellipsis()
                            .child(root),
                    )
                }),
        )
        .child(session_actions(session, cx))
        .on_click(cx.listener(move |this, _, _, cx| this.select(id.clone(), cx)))
        .into_any_element()
}

fn session_actions(session: &Session, cx: &mut Context<Observer>) -> impl IntoElement {
    use gpui_kit::component::{button::Button, input::InputState, WindowExt};
    let rename = session.clone();
    let delete = session.id.clone();
    h_flex().justify_end().gap_1()
        .child(div().id("rename-session").cursor_pointer().child(Icon::new("edit"))
            .on_click(cx.listener(move |_, _, window, cx| {
                cx.stop_propagation();
                if window.has_active_dialog(cx) { return; }
                let input = cx.new(|cx| {
                    let mut input = InputState::new(window, cx);
                    input.set_value(rename.title.clone(), window, cx);
                    input
                });
                let owner = cx.entity().downgrade();
                let id = rename.id.clone();
                window.open_dialog(cx, move |dialog, _, _| {
                    let (field, owner, id) = (input.clone(), owner.clone(), id.clone());
                    dialog.title(t!("session_rename").to_string()).child(Input::new(&input)).footer(
                        Button::new("save-session-title").label(t!("session_save").to_string()).on_click(move |_, window, cx| {
                            let title = field.read(cx).value().trim().to_owned();
                            if owner.update(cx, |this, cx| this.edit_session(id.clone(), Some(title), cx)).unwrap_or(false) {
                                window.close_dialog(cx);
                            }
                        })
                    )
                });
            })))
        .child(div().id("delete-session").cursor_pointer().child(Icon::new("delete"))
            .on_click(cx.listener(move |_, _, window, cx| {
                cx.stop_propagation();
                let id = delete.clone();
                let answer = window.prompt(PromptLevel::Warning, &t!("session_delete"), Some(&t!("session_delete_confirm")), &[
                    PromptButton::cancel(t!("action_cancel").to_string()),
                    PromptButton::new(t!("session_delete").to_string()),
                ], cx);
                cx.spawn(async move |owner, cx| {
                    if answer.await.ok() == Some(1) {
                        let _ = owner.update(cx, |this, cx| this.edit_session(id, None, cx));
                    }
                }).detach();
            })))
}

fn automation_list(observer: &Observer, cx: &mut Context<Observer>) -> AnyElement {
    let entries = match &observer.automations {
        None => return empty_tab("…".to_string(), cx),
        Some(Err(error)) => return empty_tab(error.clone(), cx),
        Some(Ok(entries)) => entries,
    };
    let query = observer.session_search.read(cx).value().trim().to_lowercase();
    let mut rows = v_flex().id("automation-list").flex_1().min_h_0().overflow_y_scroll().px_4();
    let mut found = false;
    for entry in entries.iter().filter(|entry| entry.title.to_lowercase().contains(&query)) {
        found = true;
        rows = rows.child(h_flex().id(entry.automation_id.clone()).gap_2().items_center().py_2()
            .child(div().size(px(6.)).rounded_full().bg(if entry.enabled { Hsla::from(success()) } else { cx.theme().muted_foreground }))
            .child(div().flex_1().min_w_0().text_ellipsis().child(entry.title.clone()))
            .children([
                ("toggle", if entry.enabled { t!("automation_disable") } else { t!("automation_enable") }, if entry.enabled { chatspeed_gpui::types::AutomationAction::Disable } else { chatspeed_gpui::types::AutomationAction::Enable }),
                ("run", t!("automation_run"), chatspeed_gpui::types::AutomationAction::Run),
            ].into_iter().map(|(key, label, action)| {
                let id = entry.automation_id.clone();
                div().id(key).cursor_pointer().text_xs().child(label.to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.worker.commands.try_send(chatspeed_gpui::types::Command::ActOnAutomation { id: id.clone(), action }).is_err() {
                            this.show_toast(crate::ToastKind::Error, t!("automation_action_failed").to_string(), cx);
                        }
                    }))
            }))
            .child(div().id("automation-history").cursor_pointer().text_xs().child(t!("automation_history").to_string())
                .on_click(cx.listener({
                    let id = entry.automation_id.clone();
                    move |this, _, _, cx| {
                        if this.automation_history.as_ref().is_some_and(|(current, _)| *current == id) {
                            this.automation_history = None;
                        } else if this.worker.commands.try_send(chatspeed_gpui::types::Command::ListAutomationRuns(id.clone())).is_ok() {
                            this.automation_history = Some((id.clone(), None));
                        } else {
                            this.show_toast(crate::ToastKind::Error, t!("automation_action_failed").to_string(), cx);
                        }
                        cx.notify();
                    }
                })))
            .child(div().id("delete-automation").cursor_pointer().child(Icon::new("delete"))
                .on_click(cx.listener({
                    let id = entry.automation_id.clone();
                    move |_, _, window, cx| {
                        let id = id.clone();
                        let label = t!("automation_delete");
                        let answer = window.prompt(PromptLevel::Warning, &label, Some(&t!("automation_delete_confirm")), &[
                            PromptButton::cancel(t!("action_cancel").to_string()),
                            PromptButton::new(label.to_string()),
                        ], cx);
                        cx.spawn(async move |owner, cx| {
                            if answer.await.ok() == Some(1) {
                                let _ = owner.update(cx, |this, cx| {
                                    use chatspeed_gpui::types::{AutomationAction, Command};
                                    if this.worker.commands.try_send(Command::ActOnAutomation { id, action: AutomationAction::Delete }).is_err() {
                                        this.show_toast(crate::ToastKind::Error, t!("automation_action_failed").to_string(), cx);
                                    }
                                });
                            }
                        }).detach();
                    }
                }))));
        if let Some((_, runs)) = observer.automation_history.as_ref().filter(|(id, _)| *id == entry.automation_id) {
            let history = match runs {
                None => vec!["…".to_string()],
                Some(Err(error)) => vec![error.clone()],
                Some(Ok(runs)) if runs.is_empty() => vec![t!("automation_history_empty").to_string()],
                Some(Ok(runs)) => runs.iter().map(|run| format!("{} · {}", run.scheduled_for, status_label(&run.status))).collect(),
            };
            rows = rows.child(v_flex().pl_4().text_xs().text_color(cx.theme().muted_foreground).children(history.into_iter().map(|line| div().child(line))));
        }
    }
    if found { rows.into_any_element() } else { empty_tab(t!("automation_empty").to_string(), cx) }
}

fn empty_tab(message: String, cx: &mut Context<Observer>) -> AnyElement {
    div()
        .flex_1()
        .min_h_0()
        .px(px(16.))
        .py_2()
        .text_size(px(11.))
        .text_color(cx.theme().muted_foreground)
        .child(message)
        .into_any_element()
}

/// The 50px compact column: tabs, active/recent badges, then the entries.
fn compact(
    observer: &Observer,
    skin: &Surfaces,
    radius: Pixels,
    cx: &mut Context<Observer>,
) -> AnyElement {
    let border = cx.theme().border;

    v_flex()
        .id("compact-sidebar")
        .w(px(SIDEBAR_COLLAPSED_WIDTH))
        .h_full()
        .flex_none()
        .items_center()
        .bg(skin.deep)
        .border_r_1()
        .border_color(border)
        .rounded_bl(radius)
        .child(
            v_flex()
                .w_full()
                .flex_none()
                .items_center()
                .gap(px(4.))
                .py_2()
                .border_b_1()
                .border_color(border)
                .child(rail_tab_button(
                    "compact-tasks",
                    "skill-plan3",
                    observer.rail_tab == RailTab::Tasks,
                    false,
                    skin,
                    cx,
                    |this, cx| this.select_rail_tab(RailTab::Tasks, cx),
                ))
                .child(rail_tab_button(
                    "compact-automation",
                    "clock",
                    observer.rail_tab == RailTab::Automation,
                    false,
                    skin,
                    cx,
                    |this, cx| this.select_rail_tab(RailTab::Automation, cx),
                )),
        )
        .child(compact_list(observer, skin, cx))
        .child(
            v_flex()
                .w_full()
                .flex_none()
                .items_center()
                .mt_auto()
                .child(rail_action(
                    "compact-plugin",
                    "plugin",
                    skin,
                    cx,
                    |this, cx| this.not_implemented(t!("plugin_panel"), cx),
                ))
                .child(compact_entry(
                    "compact-chathub",
                    "connected",
                    40.,
                    skin,
                    cx,
                    |this, cx| this.not_implemented(t!("chat_hub"), cx),
                ))
                // Vue keeps the terminal entry separately so the docked ChatHub
                // page is only opened by its own entry.
                .child(compact_entry(
                    "compact-terminal",
                    "bash",
                    48.,
                    skin,
                    cx,
                    super::terminal::toggle,
                )),
        )
        .into_any_element()
}

/// A full-width compact entry: the ChatHub and terminal rows of the collapsed
/// column, which Vue separates from the icon group with a top border.
fn compact_entry(
    id: &'static str,
    icon: &'static str,
    height: f32,
    skin: &Surfaces,
    cx: &mut Context<Observer>,
    run: impl Fn(&mut Observer, &mut Context<Observer>) + 'static,
) -> AnyElement {
    let hover = skin.hover;
    div()
        .id(id)
        .w_full()
        .min_h(px(height))
        .flex()
        .items_center()
        .justify_center()
        .border_t_1()
        .border_color(cx.theme().border)
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(Icon::new(icon).size(18.).color(cx.theme().muted_foreground))
        .on_click(cx.listener(move |this, _, _, cx| run(this, cx)))
        .into_any_element()
}

/// Active and recent conversation badges between the tab and entry groups.
fn compact_list(observer: &Observer, skin: &Surfaces, cx: &mut Context<Observer>) -> AnyElement {
    let border = cx.theme().border;
    let active: Vec<&Session> = observer.sessions.iter().filter(|s| is_active(s)).collect();
    let recent: Vec<&Session> = observer
        .sessions
        .iter()
        .filter(|session| !is_active(session))
        .take(5)
        .collect();

    let mut list = v_flex()
        .id("compact-list")
        .w_full()
        .flex_1()
        .min_h_0()
        .items_center()
        .gap(px(4.))
        .py_2()
        .overflow_y_scroll();
    for session in &active {
        list = list.child(compact_badge(observer, session, skin, cx));
    }
    if !active.is_empty() && !recent.is_empty() {
        list = list.child(div().w(px(20.)).h(px(1.)).my_2().bg(border));
    }
    for session in &recent {
        list = list.child(compact_badge(observer, session, skin, cx));
    }
    list.into_any_element()
}

fn compact_badge(
    observer: &Observer,
    session: &Session,
    skin: &Surfaces,
    cx: &mut Context<Observer>,
) -> AnyElement {
    let selected = observer.selected.as_deref() == Some(session.id.as_str());
    let class = status_class(&session.status);
    let dot = status_color(class);
    let id = session.id.clone();
    let hover = skin.hover;
    let primary = cx.theme().primary;
    let badge_text = initials(session);
    let foreground = cx.theme().foreground;

    div()
        .id(format!("compact-{}", session.id))
        .relative()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .size(px(36.))
        .rounded(px(8.))
        .cursor_pointer()
        .when(selected, |view| view.text_color(primary))
        .when(!selected, |view| {
            view.text_color(foreground)
                .hover(move |style| style.bg(hover))
        })
        .child(
            div()
                .text_size(px(10.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(badge_text),
        )
        .child(
            div()
                .absolute()
                .right(px(3.))
                .bottom(px(3.))
                .w(px(8.))
                .h(px(8.))
                .rounded_full()
                .bg(dot),
        )
        .when(selected, |view| {
            view.child(
                div()
                    .absolute()
                    .left(px(-7.))
                    .top(px(10.))
                    .w(px(3.))
                    .h(px(16.))
                    .rounded_r(px(4.))
                    .bg(primary),
            )
        })
        .on_click(cx.listener(move |this, _, _, cx| this.select(id.clone(), cx)))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(status: &str) -> Session {
        Session {
            id: "s".into(),
            status: status.into(),
            ..Session::default()
        }
    }

    // `gpui_kit::test` shadows the built-in attribute through the glob import.
    #[::core::prelude::v1::test]
    fn status_classes_follow_vue_groups() {
        for status in ["thinking", "executing", "auditing", "running", "stopping"] {
            assert_eq!(status_class(status), "running");
            assert!(is_active(&session(status)));
        }
        for status in [
            "paused",
            "awaiting_user",
            "awaiting_approval",
            "awaiting_auto_approval",
            "awaiting_sub_agent",
        ] {
            assert_eq!(status_class(status), "waiting");
            assert!(is_active(&session(status)));
        }
        for status in ["completed", "cancelled", "interrupted", "failed", "error"] {
            assert!(!is_active(&session(status)));
        }
        assert_eq!(status_class("COMPLETED"), "completed");
        assert_eq!(status_class("cancelled"), "cancelled");
        assert_eq!(status_class(""), "pending");
    }

    #[::core::prelude::v1::test]
    fn badges_use_the_primary_root_initials() {
        let session: Session = serde_json::from_value(serde_json::json!({
            "id": "s",
            "agent_config": "{\"allowedPaths\":[\"/work/chatspeed/\"]}",
        }))
        .expect("session");
        assert_eq!(initials(&session), "CH");
        assert_eq!(root_label(&session).as_deref(), Some("chatspeed"));
        assert_eq!(initials(&Session::default()), "—");
    }
}
