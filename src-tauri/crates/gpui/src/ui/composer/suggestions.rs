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
    scroll: ScrollHandle,
    pub(crate) file_request: Option<(u64, String, Vec<String>)>,
    request_id: u64,
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
                .rounded(px(6.))
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
            .rounded(px(10.))
            .p_1()
            .child(list)
            .into_any_element(),
    )
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

fn skill_rows(observer: &Observer, query: &str) -> Vec<Row> {
    use rust_i18n::t;
    let mut candidates = Vec::new();
    let existing = observer.selected.is_some();
    for (name, detail, available) in [
        ("settings", t!("menu_settings"), true),
        ("models", t!("model_config"), true),
        ("skills-config", t!("skills_config"), true),
        ("mcp", t!("mcp_config"), true),
        ("proxy", t!("menu_proxy"), true),
        ("agent", t!("menu_agent"), true),
        ("about", t!("menu_about"), true),
        ("new", t!("new_task"), true),
        ("audit", t!("final_audit_label"), existing),
        ("compress", t!("manual_compress"), existing),
        (
            "clear",
            t!("clear_context"),
            existing && !observer.workflow_running(),
        ),
        ("plan", t!("planning_mode"), existing),
        ("attach", t!("add_attachment"), true),
    ] {
        if available {
            candidates.push(Row {
                id: format!("command-{name}"),
                label: format!("/{name}"),
                detail: detail.into_owned(),
                icon: "setting",
                completion: Completion::Run(name),
                group: Some(Group::Commands),
            });
        }
    }
    let config = observer.agent_config();
    let disabled = config["skillEnabled"]
        .as_bool()
        .or_else(|| config["skill_enabled"].as_bool())
        == Some(false);
    let selected = crate::SkillsDraft::from_config(&config).selected;
    if !disabled {
        for skill in &observer.skills {
            let Some(name) = skill["name"].as_str() else {
                continue;
            };
            if !selected.is_empty() && !selected.contains(name) {
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
        .is_some_and(|(_, previous, roots)| previous == query && *roots == observer.model.roots)
    {
        return Vec::new();
    }
    let primary_root = observer.model.roots.first();
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
    let replacement = match completion {
        Completion::Insert(text) => text.trim_end(),
        Completion::Run(_) => "",
    };
    let append_space = matches!(completion, Completion::Insert(_))
        && !value[range.end..].starts_with(char::is_whitespace);
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
    if let Completion::Run(name) = completion {
        run(observer, name, window, cx);
    }
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
    refresh(observer, cx);
    let Some(trigger) = observer.suggestions.current.clone() else {
        return false;
    };
    if observer.suggestions.dismissed {
        return false;
    }
    let rows = rows(observer, &trigger);
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
    let paths = observer.model.roots.clone();
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

fn run(observer: &mut Observer, name: &str, window: &mut Window, cx: &mut Context<Observer>) {
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
        "new" => observer.start_new_session(window, cx),
        "clear" => observer.clear_context_frame(cx),
        "compress" => observer.manual_compress(cx),
        "attach" => observer.pick_attachments(cx),
        "plan" => observer.toggle_planning(cx),
        "audit" => observer.toggle_runtime_preference("finalAudit", cx),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{advance, format_reference, trigger as at_caret, Trigger};

    fn trigger(text: &str) -> Option<Trigger> {
        at_caret(text, text.len())
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
