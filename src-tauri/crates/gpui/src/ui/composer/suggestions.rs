//! Caret-scoped file and slash completion for the composer.

use std::ops::Range;

use gpui_kit::component::{h_flex, v_flex, ActiveTheme, ThemeStyled};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use serde_json::Value;

use crate::ui::Icon;
use crate::Observer;
use chatspeed_gpui::types::Command;

/// What the draft is currently completing.
#[derive(Clone, PartialEq, Eq)]
pub enum Trigger {
    Skill { query: String, range: Range<usize> },
    File { query: String, range: Range<usize> },
}

impl Trigger {
    fn range(&self) -> Range<usize> {
        match self {
            Self::Skill { range, .. } | Self::File { range, .. } => range.clone(),
        }
    }
}

/// Complete the token under the caret, never the surrounding draft.
pub fn trigger(text: &str, caret: usize) -> Option<Trigger> {
    let head = text.get(..caret)?;
    let tail = text.get(caret..)?;
    let end = caret + tail.find(char::is_whitespace).unwrap_or(tail.len());
    if let Some(rest) = head.strip_prefix('/') {
        if !rest.chars().any(char::is_whitespace) && end == text.len() {
            return Some(Trigger::Skill {
                query: rest.to_owned(),
                range: 0..end,
            });
        }
    }
    if let Some(index) = head.rmatch_indices("@\"").find_map(|(index, _)| {
        (index == 0 || head[..index].ends_with(char::is_whitespace)).then_some(index)
    }) {
        let query_start = index + 2;
        let mut escaped = false;
        let closing = text[query_start..].char_indices().find_map(|(offset, ch)| {
            if escaped {
                escaped = false;
                return None;
            }
            escaped = ch == '\\';
            (ch == '"').then_some(query_start + offset)
        });
        if closing.is_none_or(|closing| caret <= closing) {
            return Some(Trigger::File {
                query: head[query_start..].replace("\\\"", "\""),
                range: index..closing.map_or(caret, |closing| closing + 1),
            });
        }
    }
    let index = head.rfind('@')?;
    if head[..index]
        .chars()
        .next_back()
        .is_some_and(|c| !c.is_whitespace())
    {
        return None;
    }
    let query = &head[index + 1..];
    if query.chars().any(char::is_whitespace) || query.starts_with('"') {
        return None;
    }
    Some(Trigger::File {
        query: query.to_owned(),
        range: index..end,
    })
}

#[derive(Default)]
pub struct State {
    current: Option<Trigger>,
    dismissed: bool,
    selected: usize,
    removing_directory: bool,
    scroll: ScrollHandle,
    pub(crate) file_request: Option<(u64, String, Vec<String>)>,
    request_id: u64,
}

impl State {
    pub fn close_directory_panel(&mut self) {
        self.removing_directory = false;
    }
}

fn active(observer: &Observer, cx: &App) -> Option<Trigger> {
    let input = observer.composer.read(cx);
    if !input.selected_range().is_empty() {
        return None;
    }
    trigger(&input.value(), input.cursor())
}

fn rows(observer: &Observer, trigger: &Trigger) -> Vec<Row> {
    match trigger {
        Trigger::Skill { query, .. } => skill_rows(observer, query),
        Trigger::File { query, .. } => file_rows(observer, query),
    }
}

/// The panel above the box, or `None` when nothing matches.
pub fn panel(observer: &Observer, cx: &mut Context<Observer>) -> Option<AnyElement> {
    if observer.suggestions.removing_directory {
        return Some(directory_panel(observer, cx));
    }
    let trigger = active(observer, cx)?;
    if !observer.composer_focused
        || observer.suggestions.dismissed && observer.suggestions.current.as_ref() == Some(&trigger)
    {
        return None;
    }
    let rows = rows(observer, &trigger);
    if rows.is_empty() {
        return None;
    }
    let primary = cx.theme().primary;
    let muted = cx.theme().muted_foreground;
    let hover = cx.theme().muted;
    let file_panel = matches!(trigger, Trigger::File { .. });
    let mut group = None;
    let mut list = v_flex()
        .id("composer-suggestions")
        .w_full()
        .gap(px(1.))
        .track_scroll(&observer.suggestions.scroll)
        .max_h(px(292.))
        .overflow_y_scroll();
    for (index, row) in rows.into_iter().enumerate() {
        if row.group != group {
            group = row.group;
            if let Some(label) = group {
                list = list.child(
                    div()
                        .px_2()
                        .pt_2()
                        .text_size(px(11.))
                        .text_color(muted)
                        .child(label.title()),
                );
            }
        }
        let expected = trigger.clone();
        let completion = row.completion.clone();
        let selected = index == observer.suggestions.selected;
        let line = if file_panel {
            // Vue's file panel is one compact line with an icon and the path.
            h_flex()
                .w_full()
                .min_w_0()
                .items_center()
                .gap_2()
                .px_2()
                .py(px(4.))
                .child(Icon::new(row.icon).size(14.).color(muted))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_size(px(12.))
                        .text_color(cx.theme().foreground)
                        .child(row.label),
                )
                .into_any_element()
        } else {
            // Vue's skill rows stack the command name over its description.
            v_flex()
                .w_full()
                .min_w_0()
                .gap(px(2.))
                .px_2()
                .py(px(4.))
                .child(
                    div()
                        .w_full()
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(primary)
                        .child(row.label),
                )
                .when(!row.detail.is_empty(), |column| {
                    column.child(
                        div()
                            .w_full()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .text_size(px(11.))
                            .text_color(cx.theme().foreground.opacity(0.6))
                            .child(row.detail),
                    )
                })
                .into_any_element()
        };
        list = list.child(
            div()
                .id(SharedString::from(row.id))
                .w_full()
                .min_w_0()
                .rounded(px(4.))
                .cursor_pointer()
                .when(selected, |line| line.bg(primary.opacity(0.14)))
                .hover(move |style| style.bg(hover))
                .child(line)
                .on_click(cx.listener(move |observer, _, window, cx| {
                    apply(observer, &expected, &completion, window, cx);
                })),
        );
    }
    Some(
        v_flex()
            .occlude()
            .w_full()
            .min_w_0()
            .max_h(px(300.))
            .popover_style(cx)
            .rounded(px(8.))
            .p_1()
            .child(list)
            .into_any_element(),
    )
}

fn directory_panel(observer: &Observer, cx: &mut Context<Observer>) -> AnyElement {
    use rust_i18n::t;
    let muted = cx.theme().muted_foreground;
    let hover = cx.theme().muted;
    let mut list = v_flex().id("remove-directory-list").max_h(px(240.)).overflow_y_scroll();
    if observer.roots().is_empty() {
        list = list.child(div().px_2().py_1().text_color(muted).child(t!("paths_empty").to_string()));
    }
    let pending = observer.selected.as_ref().is_some_and(|id| {
        observer.config_pending.contains(&(id.clone(), "allowed-paths"))
    });
    for path in observer.roots() {
        let path = path.clone();
        list = list.child(
            h_flex().gap_2().px_2().py_1().min_w_0().items_center()
                .child(Icon::new("ext-folder").size(14.).color(muted))
                .child(div().flex_1().min_w_0().text_ellipsis().child(path.clone()))
                .child(
                    div().id(SharedString::from(path.clone())).p_1().rounded_sm()
                        .child(Icon::new("trash").size(14.).color(muted))
                        .when(!pending, |button| button.cursor_pointer()
                            .hover(move |style| style.bg(hover))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.remove_authorized_root(&path, cx);
                            }))),
                ),
        );
    }
    v_flex().id("remove-directory-panel").occlude().w_full().popover_style(cx).p_2()
        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
            this.suggestions.removing_directory = false;
            cx.notify();
        }))
        .child(h_flex().justify_between().items_center().px_2().py_1()
            .child(t!("paths_title").to_string())
            .child(div().id("dismiss-remove-directory").p_1().cursor_pointer()
                .child(Icon::new("close").size(14.).color(muted))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.suggestions.removing_directory = false;
                    cx.notify();
                }))))
        .child(list)
        .into_any_element()
}

struct Row {
    id: String,
    label: String,
    detail: String,
    icon: &'static str,
    completion: Completion,
    group: Option<Group>,
}

#[derive(Clone)]
enum Completion {
    Insert(String),
    Run(&'static str),
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Group {
    Installed,
    Bundled,
    Commands,
}

impl Group {
    fn title(self) -> String {
        match self {
            Self::Installed => rust_i18n::t!("suggestion_installed"),
            Self::Bundled => rust_i18n::t!("suggestion_bundled"),
            Self::Commands => rust_i18n::t!("suggestion_commands"),
        }
        .into_owned()
    }
}

fn builtin_commands(observer: &Observer) -> [(&'static str, std::borrow::Cow<'static, str>, bool); 15] {
    use rust_i18n::t;
    [
        ("settings", t!("menu_settings"), true),
        ("models", t!("model_config"), true),
        ("skills-config", t!("skills_config"), true),
        ("mcp", t!("mcp_config"), true),
        ("proxy", t!("menu_proxy"), true),
        ("agent", t!("menu_agent"), true),
        ("about", t!("menu_about"), true),
        ("new", t!("new_task"), true),
        ("audit", t!("final_audit_label"), true),
        ("compress", t!("manual_compress"), true),
        ("clear", t!("clear_context"), observer.can_clear_context()),
        ("plan", t!("planning_mode"), observer.can_toggle_planning()),
        ("attach", t!("add_attachment"), observer.can_attach_images()),
        ("add-dir", t!("suggestion_add_directory"), true),
        ("remove-dir", t!("suggestion_remove_directory"), true),
    ]
}

fn skill_rows(observer: &Observer, query: &str) -> Vec<Row> {
    let mut candidates = Vec::new();
    for (name, detail, available) in builtin_commands(observer) {
        if available {
            candidates.push(Row {
                id: format!("command-{name}"),
                label: format!("/{name}"),
                detail: detail.into_owned(),
                icon: "setting",
                completion: if name == "add-dir" {
                    Completion::Insert("/add-dir ".into())
                } else {
                    Completion::Run(name)
                },
                group: Some(Group::Commands),
            });
        }
    }
    let workflow = observer.agent_config();
    let config = if observer.selected.is_some() && !observer.model.workflow.is_null() {
        &workflow
    } else if let Some(agent) = observer.composer_agent() {
        agent
    } else {
        return rank_rows(candidates, query);
    };
    let disabled = config["skillEnabled"]
        .as_bool()
        .or_else(|| config["skill_enabled"].as_bool())
        == Some(false);
    let selected = config.get("selectedSkills").or_else(|| config.get("selected_skills"))
        .and_then(Value::as_array);
    if !disabled {
        for skill in &observer.skills {
            let Some(name) = skill["name"].as_str() else {
                continue;
            };
            if !skill_allowed(selected, name) {
                continue;
            }
            candidates.push(Row {
                id: format!("composer-skill-{name}"),
                label: format!("/{name}"),
                detail: skill["description"].as_str().unwrap_or_default().to_owned(),
                icon: "skill",
                completion: Completion::Insert(format!("/{name} ")),
                group: Some(if skill["source"].as_str() == Some("user") {
                    Group::Installed
                } else {
                    Group::Bundled
                }),
            });
        }
    }
    rank_rows(candidates, query)
}

fn skill_allowed(selected: Option<&Vec<Value>>, name: &str) -> bool {
    name == "help" || selected.is_none_or(|names| names.iter().any(|value| value.as_str() == Some(name)))
}

fn rank_rows(rows: Vec<Row>, query: &str) -> Vec<Row> {
    let query = query.to_lowercase();
    let mut rows: Vec<_> = rows
        .into_iter()
        .filter_map(|row| {
            let name = row.label[1..].to_lowercase();
            let detail = row.detail.to_lowercase();
            let rank = if name == query {
                0
            } else if name.starts_with(&query) {
                1
            } else if name.contains(&query) {
                2
            } else if detail.starts_with(&query) {
                3
            } else if detail.contains(&query) {
                4
            } else {
                return None;
            };
            Some((rank, name, row))
        })
        .collect();
    if rows.iter().any(|(rank, _, _)| *rank < 3) {
        rows.retain(|(rank, _, _)| *rank < 3);
    }
    // Put the strongest matching group first so its best result is highlighted.
    let best = [Group::Installed, Group::Bundled, Group::Commands].map(|group| {
        rows.iter()
            .filter(|(_, _, row)| row.group == Some(group))
            .map(|(rank, _, _)| *rank)
            .min()
            .unwrap_or(5)
    });
    rows.sort_by(|(rank, name, row), (other_rank, other_name, other)| {
        let group = row.group.unwrap_or(Group::Commands);
        let other_group = other.group.unwrap_or(Group::Commands);
        (best[group as usize], group, rank, name).cmp(&(
            best[other_group as usize],
            other_group,
            other_rank,
            other_name,
        ))
    });
    rows.into_iter().map(|(_, _, row)| row).collect()
}

fn file_rows(observer: &Observer, query: &str) -> Vec<Row> {
    // A stale list must never answer a newer query.
    if !observer
        .suggestions
        .file_request
        .as_ref()
        .is_some_and(|(_, previous, roots)| previous == query && roots == observer.roots())
    {
        return Vec::new();
    }
    let primary_root = observer.roots().first();
    observer
        .file_results
        .iter()
        .filter_map(|file| {
            let path = file["path"].as_str()?;
            let display = reference_path(file, primary_root);
            if display.is_empty() {
                return None;
            }
            let is_directory = file["is_directory"].as_bool().unwrap_or(false);
            Some(Row {
                id: format!("composer-file-{path}"),
                label: display.clone(),
                detail: String::new(),
                icon: if is_directory {
                    "ext-folder"
                } else {
                    "ext-file"
                },
                group: None,
                completion: Completion::Insert(format!("{} ", format_reference(&display))),
            })
        })
        .collect()
}

/// Vue shows the path relative to the primary root and the absolute one for
/// files that live in another authorized root.
fn reference_path(file: &Value, primary_root: Option<&String>) -> String {
    let path = file["path"].as_str().unwrap_or_default();
    let root = file["root_path"].as_str().unwrap_or_default();
    let relative = file["relative_path"].as_str().unwrap_or_default();
    if relative.is_empty() {
        return path.to_owned();
    }
    match primary_root {
        Some(primary) if primary == root => relative.to_owned(),
        Some(_) => path.to_owned(),
        None => relative.to_owned(),
    }
}

/// Vue quotes paths that contain whitespace, a quote or a backslash.
pub(in crate::ui) fn format_reference(path: &str) -> String {
    if !path
        .chars()
        .any(|c| c.is_whitespace() || c == '"' || c == '\\')
    {
        return format!("@{path}");
    }
    let escaped = path.replace('\\', "\\\\").replace('"', "\\\"");
    format!("@\"{escaped}\"")
}

fn apply(
    observer: &mut Observer,
    expected: &Trigger,
    completion: &Completion,
    window: &mut Window,
    cx: &mut Context<Observer>,
) {
    if active(observer, cx).as_ref() != Some(expected) {
        return;
    }
    let input = observer.composer.clone();
    let value = input.read(cx).value();
    let range = expected.range();
    if let Completion::Run(name) = completion {
        if can_run(observer, name) {
            input.update(cx, |state, cx| state.set_value("", window, cx));
            execute(observer, name, "", window, cx);
        }
        return;
    }
    let Completion::Insert(text) = completion else { return; };
    let replacement = text.trim_end();
    let append_space = !value[range.end..].starts_with(char::is_whitespace);
    let replacement = if append_space {
        format!("{replacement} ")
    } else {
        replacement.to_owned()
    };
    let utf16 =
        value[..range.start].encode_utf16().count()..value[..range.end].encode_utf16().count();
    input.update(cx, |state, cx| {
        EntityInputHandler::replace_text_in_range(state, Some(utf16), &replacement, window, cx);
        state.focus_handle(cx).focus(window, cx);
    });
    refresh(observer, cx);
    observer.suggestions.dismissed = true;
    cx.notify();
}

pub fn handle_key(
    observer: &mut Observer,
    key: &str,
    window: &mut Window,
    cx: &mut Context<Observer>,
) -> bool {
    if observer.suggestions.removing_directory {
        if key == "escape" {
            observer.suggestions.removing_directory = false;
            cx.notify();
            return true;
        }
        return false;
    }
    refresh(observer, cx);
    let Some(trigger) = observer.suggestions.current.clone() else {
        return false;
    };
    if observer.suggestions.dismissed {
        return false;
    }
    let rows = rows(observer, &trigger);
    if rows.is_empty() {
        return false;
    }
    let state = &mut observer.suggestions;
    match key {
        "escape" => state.dismissed = true,
        "up" | "down" => {
            if !rows.is_empty() {
                state.selected = advance(state.selected, rows.len(), key == "down");
                let headings = rows
                    .iter()
                    .take(state.selected + 1)
                    .enumerate()
                    .filter(|(i, row)| {
                        row.group.is_some() && (*i == 0 || row.group != rows[i - 1].group)
                    })
                    .count();
                state.scroll.scroll_to_item(state.selected + headings);
            }
        }
        "enter" | "tab" => {
            if let Some(row) = rows.get(state.selected).or_else(|| rows.first()) {
                apply(observer, &trigger, &row.completion, window, cx);
            }
        }
        _ => return false,
    }
    cx.notify();
    true
}

fn advance(selected: usize, len: usize, down: bool) -> usize {
    if down {
        (selected + 1) % len
    } else {
        (selected + len - 1) % len
    }
}

pub fn refresh(observer: &mut Observer, cx: &mut Context<Observer>) -> bool {
    let trigger = active(observer, cx);
    let paths = observer.roots().to_vec();
    let state = &mut observer.suggestions;
    let changed = state.current != trigger;
    if changed {
        state.current = trigger.clone();
        state.selected = 0;
        state.dismissed = false;
        state.scroll = ScrollHandle::new();
    }
    let Some(Trigger::File { query, .. }) = trigger else {
        state.file_request = None;
        observer.file_results.clear();
        return changed;
    };
    if !state
        .file_request
        .as_ref()
        .is_some_and(|(_, previous, roots)| *previous == query && *roots == paths)
    {
        observer.file_results.clear();
        state.request_id = state.request_id.wrapping_add(1);
        let request = state.request_id;
        if observer
            .worker
            .commands
            .try_send(Command::SearchFiles {
                request,
                paths: paths.clone(),
                query: query.clone(),
            })
            .is_ok()
        {
            state.file_request = Some((request, query, paths));
        }
    }
    changed
}

fn parse_command(text: &str) -> Option<(String, &str)> {
    let text = text.trim().strip_prefix('/')?;
    let boundary = text.find(char::is_whitespace).unwrap_or(text.len());
    let (name, argument) = text.split_at(boundary);
    (!name.is_empty()).then(|| (name.to_ascii_lowercase(), argument.trim()))
}

fn directory_argument(argument: &str) -> String {
    let mut chars = argument.chars();
    let Some(quote @ ('\'' | '"')) = chars.next() else { return argument.to_owned(); };
    if chars.next_back() != Some(quote) {
        return argument.to_owned();
    }
    let mut inner = chars.peekable();
    let mut path = String::new();
    while let Some(ch) = inner.next() {
        if ch == '\\' && inner.peek().is_some_and(|ch| matches!(ch, '\\' | '\'' | '"')) {
            path.extend(inner.next());
        } else {
            path.push(ch);
        }
    }
    path.trim().to_owned()
}

fn expand_directory(path: &str, home: Option<&std::path::Path>) -> Option<std::path::PathBuf> {
    match path.strip_prefix('~') {
        Some("") => home.map(std::path::Path::to_path_buf),
        Some(rest) if rest.starts_with(std::path::is_separator) => {
            Some(home?.join(rest.trim_start_matches(std::path::is_separator)))
        }
        _ => Some(path.into()),
    }
}

fn can_run(observer: &Observer, name: &str) -> bool {
    match name {
        "clear" => observer.can_clear_context(),
        "plan" => observer.can_toggle_planning(),
        "attach" => observer.can_attach_images(),
        "audit" => observer.can_toggle_final_audit(),
        _ => true,
    }
}

pub fn run_typed(observer: &mut Observer, text: &str, window: &mut Window, cx: &mut Context<Observer>) -> bool {
    let Some((name, argument)) = parse_command(text) else { return false; };
    if !builtin_commands(observer).iter().any(|(command, _, _)| *command == name)
        || (!argument.is_empty() && !matches!(name.as_str(), "add-dir" | "remove-dir"))
        || !can_run(observer, &name)
    {
        return false;
    }
    // Preserve an incomplete command so the user can correct the path.
    if name != "add-dir" {
        observer.composer.update(cx, |state, cx| state.set_value("", window, cx));
    }
    execute(observer, &name, argument, window, cx);
    true
}

fn execute(observer: &mut Observer, name: &str, argument: &str, window: &mut Window, cx: &mut Context<Observer>) {
    use crate::QuickPanel;
    use chatspeed_runtime_client::desktop_menu::Action;
    match name {
        "settings" => observer.desktop_menu_action(Action::Settings, cx),
        "mcp" => observer.desktop_menu_action(Action::Mcp, cx),
        "proxy" => observer.desktop_menu_action(Action::Proxy, cx),
        "agent" => observer.desktop_menu_action(Action::Agent, cx),
        "about" => observer.desktop_menu_action(Action::About, cx),
        "models" => observer.open_quick_panel(QuickPanel::Models, window, cx),
        "skills-config" => observer.open_quick_panel(QuickPanel::Skills, window, cx),
        "new" => observer.open_new_task_dialog(cx),
        "clear" => observer.clear_context_frame(cx),
        "compress" if observer.selected.is_some() => observer.manual_compress(cx),
        "compress" => observer.show_toast(crate::ToastKind::Warning, rust_i18n::t!("suggestion_compress_no_session").to_string(), cx),
        "attach" => observer.pick_attachments(cx),
        "plan" => observer.toggle_planning(cx),
        "audit" => observer.toggle_runtime_preference("finalAudit", cx),
        "add-dir" => {
            let path = directory_argument(argument);
            let home = std::env::home_dir();
            if let Some(path) = expand_directory(&path, home.as_deref()).filter(|path| !path.as_os_str().is_empty()) {
                if observer.add_authorized_root(path, cx) {
                    observer.composer.update(cx, |state, cx| state.set_value("", window, cx));
                }
            } else {
                observer.show_toast(crate::ToastKind::Warning, rust_i18n::t!("paths_invalid_directory").to_string(), cx);
            }
        }
        "remove-dir" if argument.is_empty() => observer.suggestions.removing_directory = true,
        "remove-dir" => observer.show_toast(crate::ToastKind::Warning, rust_i18n::t!("paths_remove_no_args").to_string(), cx),
        _ => {}
    }
    cx.notify();
}

#[cfg(test)]
mod tests {
    use super::{advance, format_reference, trigger as at_caret, Trigger};

    fn trigger(text: &str) -> Option<Trigger> {
        at_caret(text, text.len())
    }

    #[test]
    fn directory_commands_parse_quoted_paths_and_home_without_shell_expansion() {
        use super::{directory_argument, expand_directory, parse_command};
        use std::path::Path;

        let (name, argument) = parse_command("  /ADD-DIR  \"~/My Projects/中文\" ").unwrap();
        assert_eq!(name, "add-dir");
        assert_eq!(directory_argument(argument), "~/My Projects/中文");
        assert_eq!(directory_argument(r#"'/work/it\'s here'"#), "/work/it's here");
        assert_eq!(directory_argument(r#"C:\My Projects\app"#), r#"C:\My Projects\app"#);
        let home = Path::new("home");
        assert_eq!(expand_directory("~/work", Some(home)), Some(home.join("work")));
        assert_eq!(expand_directory("~", None), None);
        assert_eq!(parse_command("/remove-dir"), Some(("remove-dir".into(), "")));
        assert_eq!(parse_command("tell me /add-dir"), None);
    }

    #[test]
    fn explicitly_empty_skill_config_keeps_only_help() {
        use super::skill_allowed;
        use serde_json::json;

        assert!(skill_allowed(None, "custom"));
        assert!(!skill_allowed(Some(&vec![]), "custom"));
        let selected = vec![json!("custom")];
        assert!(skill_allowed(Some(&selected), "custom"));
        assert!(skill_allowed(Some(&selected), "help"));
        assert!(!skill_allowed(Some(&selected), "other"));
    }

    #[test]
    fn completion_targets_only_the_token_even_after_unicode_and_before_suffix() {
        let draft = "🦀 中文 @srcremainder then continue";
        let caret = "🦀 中文 @src".len();
        let result = at_caret(draft, caret).expect("file completion");
        let range = result.range();
        assert_eq!(&draft[range.clone()], "@srcremainder");
        assert_eq!(&draft[..range.start], "🦀 中文 ");
        assert_eq!(&draft[range.end..], " then continue");
        assert_eq!(advance(0, 3, false), 2);
        assert_eq!(advance(2, 3, true), 0);
    }

    #[test]
    fn leading_slash_completes_skills_only_before_whitespace() {
        assert!(matches!(trigger("/ski"), Some(Trigger::Skill { query, .. }) if query == "ski"));
        assert!(trigger("/skill now").is_none());
        assert!(trigger("hello /skill").is_none());
    }

    #[test]
    fn trailing_at_completes_files_and_ignores_emails() {
        assert!(matches!(
            trigger("look at @src/ma"),
            Some(Trigger::File { .. })
        ));
        assert!(matches!(trigger("open @src/main.rs "), None));
        assert!(trigger("user@host").is_none());
    }

    #[test]
    fn quoted_completion_replaces_the_whole_reference_but_not_the_suffix() {
        let text = r#"open @"src/my file.rs" then continue"#;
        let caret = "open @\"src/my fi".len();
        let Some(Trigger::File { query, range }) = at_caret(text, caret) else {
            panic!("expected quoted path completion");
        };
        assert_eq!(query, "src/my fi");
        assert_eq!(&text[range], r#"@"src/my file.rs""#);
        assert!(trigger(r#"open @"src/my file.rs""#).is_none());
        assert!(matches!(trigger("open @\"src/my fi"), Some(Trigger::File { query, .. }) if query == "src/my fi"));
    }

    #[test]
    fn references_are_quoted_when_needed() {
        assert_eq!(format_reference("src/main.rs"), "@src/main.rs");
        assert_eq!(format_reference("src/my file.rs"), "@\"src/my file.rs\"");
    }
}
