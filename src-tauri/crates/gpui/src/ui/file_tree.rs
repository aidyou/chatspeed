//! Authorized-path tree for the sidebar's files tab.
//!
//! Mirrors `src/components/workflow/FileTree.vue` and `TreeNode.vue`: each
//! authorized root expands into its children, folders carry the primary-colored
//! folder glyph, files carry the extension glyph, and every level hangs off a
//! dashed guide with a short connector per row.

use gpui_kit::component::{h_flex, tooltip::Tooltip, v_flex, ActiveTheme};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use rust_i18n::t;
use std::path::Path;

use crate::icons::file_icon;
use crate::ui::Icon;
use crate::Observer;
use chatspeed_gpui::types::{root_name, DirEntry};

/// Vue indents the root's children by `--cs-space` and every deeper level by 21px.
const ROOT_INDENT: f32 = 16.;
const NESTED_INDENT: f32 = 21.;
/// Cap visual indentation so actions still fit in a narrow sidebar.
const MAX_INDENT_DEPTH: usize = 4;
/// Guards against a symlink cycle turning the recursive walk into a hang.
const MAX_DEPTH: usize = 32;

/// Renders the authorized paths and, for expanded ones, their children.
pub fn tree(observer: &Observer, cx: &mut Context<Observer>) -> AnyElement {
    let foreground = cx.theme().foreground;
    let secondary = cx.theme().muted_foreground;
    let mut list = v_flex()
        .id("authorized-paths")
        .flex_1()
        .min_h_0()
        .w_full()
        .overflow_y_scroll()
        .py_2();

    for root in observer.roots() {
        list = list.child(root_row(observer, root, foreground, secondary, cx));
        if observer.expanded_dirs.contains(root) {
            let rows = children(observer, root, 1, cx);
            if !rows.is_empty() {
                list = list.child(level(ROOT_INDENT, rows, cx));
            }
        }
    }

    if observer.roots().is_empty() {
        list = list.child(
            div()
                .w_full()
                .px_3()
                .py_3()
                .text_size(px(11.))
                .text_color(secondary)
                .child(t!("paths_empty").to_string()),
        );
    }
    v_flex()
        .flex_1()
        .min_h_0()
        .w_full()
        .child(
            h_flex()
                .flex_none()
                .w_full()
                .justify_between()
                .items_center()
                .px_4()
                .py_2()
                .border_b_1()
                .border_color(cx.theme().border)
                .child(div().text_xs().font_weight(FontWeight::BOLD).text_color(secondary)
                    .child(t!("paths_title").to_string()))
                .child(h_flex().gap_2()
                    .child(header_action("add", t!("suggestion_add_directory").to_string(), Observer::pick_authorized_root, cx))
                    .child(header_action("refresh", t!("status_panel_refresh").to_string(), Observer::refresh_authorized_dirs, cx))),
        )
        .child(list)
        .into_any_element()
}

fn header_action(
    icon: &'static str,
    label: String,
    action: fn(&mut Observer, &mut Context<Observer>),
    cx: &mut Context<Observer>,
) -> impl IntoElement {
    let muted = cx.theme().muted_foreground;
    let primary = cx.theme().primary;
    div().id(icon).p_1().rounded_sm().cursor_pointer().text_color(muted)
        .hover(move |style| style.text_color(primary))
        .child(Icon::new(icon).size(14.))
        .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
        .on_click(cx.listener(move |this, _, _, cx| action(this, cx)))
}

#[derive(Clone)]
struct RootDrag {
    path: String,
    generation: u64,
}

impl Render for RootDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().px_2().py_1().rounded_sm().bg(cx.theme().background)
            .child(root_name(&self.path).to_owned())
    }
}

fn reordered_roots(roots: &[String], source: &str, target: &str) -> Option<Vec<String>> {
    let from = roots.iter().position(|root| root == source)?;
    let to = roots.iter().position(|root| root == target)?;
    if from == to {
        return None;
    }
    let mut roots = roots.to_vec();
    let root = roots.remove(from);
    roots.insert(to, root);
    Some(roots)
}

fn root_row(
    observer: &Observer,
    root: &str,
    foreground: Hsla,
    secondary: Hsla,
    cx: &mut Context<Observer>,
) -> AnyElement {
    let expanded = observer.expanded_dirs.contains(root);
    let hover = cx.theme().muted;
    let path = root.to_owned();
    h_flex()
        .id(format!("root-{root}"))
        .group("authorized-tree-row")
        .w_full()
        .min_w_0()
        .items_center()
        .gap_2()
        .px(px(16.))
        .py_2()
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(
            Icon::new(if expanded {
                "ext-folder-open"
            } else {
                "ext-folder"
            })
            .size(14.)
            .color(secondary),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(foreground)
                .text_ellipsis()
                .child(root_name(root).to_owned()),
        )
        .child(actions(root, true, true, cx))
        .when(observer.roots().len() > 1, |row| {
            row.on_drag(RootDrag { path: path.clone(), generation: observer.model.generation }, |drag, _, _, cx| {
                cx.new(|_| drag.clone())
            })
            .on_drop(cx.listener({
                let target = path.clone();
                move |this, drag: &RootDrag, _, cx| {
                    if this.model.accepts(drag.generation) {
                        if let Some(roots) = reordered_roots(this.roots(), &drag.path, &target) {
                            this.save_authorized_roots(roots, cx);
                        }
                    }
                }
            }))
        })
        .on_click(cx.listener(move |this, _, _, cx| this.toggle_dir(path.clone(), cx)))
        .into_any_element()
}

/// One dashed level: the guide plus the rows it indents.
fn level(indent: f32, rows: Vec<AnyElement>, cx: &mut Context<Observer>) -> AnyElement {
    v_flex()
        .w_full()
        .pl(px(indent))
        .child(
            v_flex()
                .w_full()
                .border_l_1()
                .border_dashed()
                .border_color(cx.theme().border)
                .children(rows),
        )
        .into_any_element()
}

/// Children of one directory, recursing into the folders that are expanded.
fn children(
    observer: &Observer,
    path: &str,
    depth: usize,
    cx: &mut Context<Observer>,
) -> Vec<AnyElement> {
    if depth > MAX_DEPTH {
        return Vec::new();
    }
    if let Some(error) = observer.dir_errors.get(path) {
        return vec![hint_row(error.clone(), true, cx)];
    }
    let Some(entries) = observer.dir_entries.get(path) else {
        return if observer.dir_pending.contains(path) {
            vec![hint_row(t!("tree_loading").to_string(), false, cx)]
        } else {
            Vec::new()
        };
    };
    let mut rows = Vec::new();
    for entry in entries {
        rows.push(node_row(observer, entry, cx));
        if entry.is_dir && observer.expanded_dirs.contains(&entry.path) {
            let nested = children(observer, &entry.path, depth + 1, cx);
            if depth >= MAX_INDENT_DEPTH {
                rows.extend(nested);
            } else if !nested.is_empty() {
                rows.push(level(NESTED_INDENT, nested, cx));
            }
        }
    }
    rows
}

fn node_row(observer: &Observer, entry: &DirEntry, cx: &mut Context<Observer>) -> AnyElement {
    let border = cx.theme().border;
    let foreground = cx.theme().foreground;
    let secondary = cx.theme().muted_foreground;
    let hover = cx.theme().muted;
    let primary = cx.theme().primary;
    let expanded = observer.expanded_dirs.contains(&entry.path);
    let path = entry.path.clone();
    let name = entry.name.clone();
    let is_dir = entry.is_dir;
    // Match the Vue tree's status colors, but keep the Git code visible so
    // added and modified entries are distinguishable without relying on color.
    let git_status = entry.git_status.as_deref().map(|code| {
        let color = if code.contains('D') {
            cx.theme().danger
        } else if code == "??" || code.contains('A') {
            cx.theme().success
        } else if code.contains(['M', 'R', 'C', 'U', 'T']) {
            cx.theme().warning
        } else {
            secondary
        };
        (code.to_owned(), color)
    });

    h_flex()
        .id(format!("node-{path}"))
        .group("authorized-tree-row")
        .relative()
        .w_full()
        .min_w_0()
        .items_center()
        .gap_2()
        .px(px(16.))
        .py(px(4.))
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        // The connector sits on the guide and stops short of the icon column.
        .child(
            div()
                .absolute()
                .top(px(12.))
                .left(px(-1.))
                .w(px(12.))
                .h(px(1.))
                .border_t_1()
                .border_dashed()
                .border_color(border),
        )
        .child(
            Icon::new(if is_dir {
                if expanded {
                    "ext-folder-open"
                } else {
                    "ext-folder"
                }
            } else {
                file_icon(&name)
            })
            .size(14.)
            .color(if is_dir { primary } else { secondary }),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(13.))
                .text_color(git_status.as_ref().map_or(foreground, |(_, color)| *color))
                .text_ellipsis()
                .child(name),
        )
        .children(git_status.map(|(code, color)| {
            div().flex_shrink_0().text_xs().text_color(color).child(code)
        }))
        .child(actions(&path, is_dir, false, cx))
        .on_click(cx.listener(move |this, _, window, cx| {
            if is_dir {
                this.toggle_dir(path.clone(), cx);
            } else {
                super::file_editor::open(this, path.clone(), window, cx);
            }
        }))
        .into_any_element()
}

fn actions(path: &str, directory: bool, root: bool, cx: &mut Context<Observer>) -> AnyElement {
    let mut icons = Vec::new();
    if !root {
        icons.push("at");
    }
    icons.push("copy");
    if directory {
        icons.push("ext-folder-open");
    }
    if root {
        icons.extend(["refresh", "trash"]);
    }
    let muted = cx.theme().muted_foreground;
    let hover = cx.theme().background;
    h_flex()
        .flex_shrink_0()
        .invisible()
        .group_hover("authorized-tree-row", |style| style.visible())
        .children(icons.into_iter().map(|icon| {
            let path = path.to_owned();
            div()
                .id(icon)
                .p(px(3.))
                .rounded_sm()
                .hover(move |style| style.bg(hover))
                .child(Icon::new(icon).size(12.).color(muted))
                .on_mouse_down(MouseButton::Left, |_, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                })
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    match icon {
                        "copy" => {
                            cx.write_to_clipboard(ClipboardItem::new_string(path.clone()));
                            this.show_toast(
                                crate::ToastKind::Success,
                                t!("copied").to_string(),
                                cx,
                            );
                        }
                        "refresh" => {
                            if this.expanded_dirs.contains(&path) {
                                this.load_dir(path.clone(), cx);
                            } else {
                                this.dir_entries.remove(&path);
                            }
                        }
                        "trash" => this.remove_authorized_root(&path, cx),
                        "at" => {
                            let normalized = path.replace('\\', "/");
                            let root = this.roots().first().map(|root| root.replace('\\', "/"));
                            let reference = root
                                .as_ref()
                                .and_then(|root| Path::new(&normalized).strip_prefix(root).ok())
                                .and_then(Path::to_str)
                                .filter(|path| !path.is_empty())
                                .unwrap_or(&normalized);
                            let text = format!(
                                "{} ",
                                super::composer::suggestions::format_reference(reference)
                            );
                            this.composer.update(cx, |state, cx| {
                                EntityInputHandler::replace_text_in_range(
                                    state, None, &text, window, cx,
                                );
                                state.focus_handle(cx).focus(window, cx);
                            });
                        }
                        "ext-folder-open" => {
                            if let Err(error) = open_folder(&path, this.roots()) {
                                this.show_toast(crate::ToastKind::Error, error.to_string(), cx);
                            }
                        }
                        _ => {}
                    }
                }))
        }))
        .into_any_element()
}

fn open_folder(path: &str, roots: &[String]) -> std::io::Result<()> {
    let path = Path::new(path).canonicalize()?;
    if !path.is_dir()
        || !roots.iter().any(|root| {
            Path::new(root)
                .canonicalize()
                .is_ok_and(|root| path.starts_with(root))
        })
    {
        return Err(std::io::ErrorKind::PermissionDenied.into());
    }
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer.exe"
    } else {
        "xdg-open"
    };
    let mut child = std::process::Command::new(program).arg(path).spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::reordered_roots;

    #[test]
    fn dragging_a_root_changes_order_without_changing_membership() {
        let roots = ["/a".into(), "/b".into(), "/c".into()];
        assert_eq!(reordered_roots(&roots, "/a", "/c").unwrap(), ["/b", "/c", "/a"]);
        assert_eq!(reordered_roots(&roots, "/c", "/a").unwrap(), ["/c", "/a", "/b"]);
        assert_eq!(reordered_roots(&roots, "/a", "/a"), None);
        assert_eq!(reordered_roots(&roots, "/old", "/a"), None);
    }
}

/// A loading or error line inside an expanded folder.
fn hint_row(message: String, error: bool, cx: &mut Context<Observer>) -> AnyElement {
    let color = if error {
        cx.theme().danger
    } else {
        cx.theme().muted_foreground
    };
    div()
        .w_full()
        .px(px(16.))
        .py(px(4.))
        .text_size(px(11.))
        .text_color(color)
        .text_ellipsis()
        .child(message)
        .into_any_element()
}
