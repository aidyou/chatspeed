//! `/` skill and `@` file completion for the composer.
//!
//! Mirrors the suggestion panels of `src/components/workflow/WorkflowInputArea.vue`:
//! a leading `/` lists the installed skills (`get_system_skills`) and a trailing
//! `@` lists files from the authorized roots (`search_workspace_files`). Enter
//! applies the first row, which is the same "first match" behaviour Vue gets from
//! its highlighted index, and clicking a row applies it directly.

use gpui_kit::component::{h_flex, v_flex, ActiveTheme};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use serde_json::Value;

use crate::ui::Icon;
use crate::Observer;
use chatspeed_gpui_observer::types::Command;

/// What the draft is currently completing.
pub enum Trigger {
    Skill { query: String },
    File { query: String },
}

/// The completion context of `text`, or `None` when no panel should show.
///
/// Vue triggers on the caret; the native composer completes at the end of the
/// draft, which is where typing continues.
pub fn trigger(text: &str) -> Option<Trigger> {
    let trimmed = text.trim_start();
    if let Some(rest) = trimmed.strip_prefix('/') {
        if !rest.chars().any(char::is_whitespace) {
            return Some(Trigger::Skill {
                query: rest.to_owned(),
            });
        }
    }
    // A mention closes as soon as the draft moves past the token, which is what
    // the space appended after an insertion does.
    let head = text;
    let index = head.rfind('@')?;
    // An `@` inside a word (`user@host`) is not a mention.
    let before = head[..index].chars().next_back();
    if before.is_some_and(|c| !c.is_whitespace()) {
        return None;
    }
    let query = &head[index + 1..];
    if query.chars().any(char::is_whitespace) {
        return None;
    }
    Some(Trigger::File {
        query: query.to_owned(),
    })
}

/// The panel above the box, or `None` when nothing matches.
pub fn panel(
    observer: &Observer,
    trigger: &Trigger,
    cx: &mut Context<Observer>,
) -> Option<AnyElement> {
    let rows = match trigger {
        Trigger::Skill { query } => skill_rows(observer, query),
        Trigger::File { query } => file_rows(observer, query),
    };
    if rows.is_empty() {
        return None;
    }
    let border = cx.theme().border;
    let background = cx.theme().background;
    let primary = cx.theme().primary;
    let muted = cx.theme().muted_foreground;
    let hover = cx.theme().muted;
    let composer = observer.composer.clone();
    let file_panel = matches!(trigger, Trigger::File { .. });
    let mut list = v_flex().id("composer-suggestions").w_full().gap(px(1.));
    for row in rows {
        let text = row.insert.clone();
        let composer = composer.clone();
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
                .hover(move |style| style.bg(hover))
                .child(line)
                .on_click(move |_, window, cx: &mut App| {
                    let text = text.clone();
                    composer.update(cx, |state, cx| state.set_value(text, window, cx));
                }),
        );
    }
    Some(
        v_flex()
            .w_full()
            .max_w(px(780.))
            .max_h(px(300.))
            .rounded(px(10.))
            .border_1()
            .border_color(border)
            .bg(background)
            .p_1()
            .shadow(vec![gpui::BoxShadow {
                offset: gpui::point(px(0.), px(6.)),
                blur_radius: px(20.),
                spread_radius: px(0.),
                inset: false,
                color: cx.theme().foreground.opacity(0.12),
            }])
            .child(
                v_flex()
                    .id("composer-suggestion-scroll")
                    .max_h(px(292.))
                    .overflow_y_scroll()
                    .child(list),
            )
            .into_any_element(),
    )
}

struct Row {
    id: String,
    label: String,
    detail: String,
    icon: &'static str,
    /// The text the row writes into the composer.
    insert: String,
}

fn skill_rows(observer: &Observer, query: &str) -> Vec<Row> {
    let query = query.to_lowercase();
    observer
        .skills
        .iter()
        .filter_map(|skill| {
            let name = skill["name"].as_str()?;
            if !query.is_empty() && !name.to_lowercase().contains(&query) {
                return None;
            }
            Some(Row {
                id: format!("composer-skill-{name}"),
                label: format!("/{name}"),
                detail: skill["description"].as_str().unwrap_or_default().to_owned(),
                icon: "skill",
                insert: format!("/{name} "),
            })
        })
        .collect()
}

fn file_rows(observer: &Observer, query: &str) -> Vec<Row> {
    // A stale list must never answer a newer query.
    if observer.file_query != query {
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
                insert: format!("{} ", format_reference(&display)),
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
fn format_reference(path: &str) -> String {
    if !path
        .chars()
        .any(|c| c.is_whitespace() || c == '"' || c == '\\')
    {
        return format!("@{path}");
    }
    let escaped = path.replace('\\', "\\\\").replace('"', "\\\"");
    format!("@\"{escaped}\"")
}

/// Applies the first row of the active panel; `false` lets Enter send instead.
pub fn apply_first(
    observer: &mut Observer,
    window: &mut Window,
    cx: &mut Context<Observer>,
) -> bool {
    let text = observer.composer.read(cx).value().to_string();
    let Some(trigger) = trigger(&text) else {
        return false;
    };
    let rows = match &trigger {
        Trigger::Skill { query } => skill_rows(observer, query),
        Trigger::File { query } => file_rows(observer, query),
    };
    let Some(row) = rows.into_iter().next() else {
        return false;
    };
    let next = match trigger {
        Trigger::Skill { .. } => row.insert.clone(),
        Trigger::File { .. } => {
            // Replace the `@token` being completed, keeping the text before it.
            let head = text.trim_end();
            let start = head.rfind('@').unwrap_or(head.len());
            format!("{}{}", &text[..start], row.insert)
        }
    };
    observer
        .composer
        .update(cx, |state, cx| state.set_value(next, window, cx));
    true
}

/// Sends the `@` search the draft now needs, replacing any older one.
pub fn refresh(observer: &mut Observer, cx: &mut Context<Observer>) {
    let text = observer.composer.read(cx).value().to_string();
    match trigger(&text) {
        Some(Trigger::File { query }) if query != observer.file_query => {
            observer.file_query = query.clone();
            observer.file_results.clear();
            let _ = observer.worker.commands.try_send(Command::SearchFiles {
                paths: observer.model.roots.clone(),
                query,
            });
        }
        Some(_) => {}
        None => {
            observer.file_query.clear();
            observer.file_results.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{format_reference, trigger, Trigger};

    #[test]
    fn leading_slash_completes_skills_only_before_whitespace() {
        assert!(matches!(trigger("/ski"), Some(Trigger::Skill { query }) if query == "ski"));
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
    fn references_are_quoted_when_needed() {
        assert_eq!(format_reference("src/main.rs"), "@src/main.rs");
        assert_eq!(format_reference("src/my file.rs"), "@\"src/my file.rs\"");
    }
}
