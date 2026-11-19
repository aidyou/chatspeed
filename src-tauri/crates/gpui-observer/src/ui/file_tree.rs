//! Authorized-path tree for the sidebar's files tab.
//!
//! Mirrors `src/components/workflow/FileTree.vue` and `TreeNode.vue`: each
//! authorized root expands into its children, folders carry the primary-colored
//! folder glyph, files carry the extension glyph, and every level hangs off a
//! dashed guide with a short connector per row.

use gpui_kit::component::{h_flex, v_flex, ActiveTheme};
use gpui_kit::*;
use rust_i18n::t;

use crate::icons::file_icon;
use crate::{font_icon, Observer};
use chatspeed_gpui_observer::types::{root_name, DirEntry};

/// Vue indents the root's children by `--cs-space` and every deeper level by 21px.
const ROOT_INDENT: f32 = 16.;
const NESTED_INDENT: f32 = 21.;
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
    list.into_any_element()
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
        .w_full()
        .min_w_0()
        .items_center()
        .gap_2()
        .px(px(16.))
        .py_2()
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(font_icon(
            if expanded {
                "ext-folder-open"
            } else {
                "ext-folder"
            },
            14.,
            secondary,
        ))
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
        .on_click(cx.listener(move |this, _, _, cx| this.toggle_dir(path.clone(), cx)))
        .into_any_element()
}

/// One dashed level: the guide plus the rows it indents.
fn level(indent: f32, rows: Vec<AnyElement>, cx: &mut Context<Observer>) -> AnyElement {
    v_flex()
        .w_full()
        .ml(px(indent))
        .border_l_1()
        .border_dashed()
        .border_color(cx.theme().border)
        .children(rows)
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
            if !nested.is_empty() {
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

    h_flex()
        .id(format!("node-{path}"))
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
        .child(font_icon(
            if is_dir {
                if expanded {
                    "ext-folder-open"
                } else {
                    "ext-folder"
                }
            } else {
                file_icon(&name)
            },
            14.,
            if is_dir { primary } else { secondary },
        ))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(13.))
                .text_color(foreground)
                .text_ellipsis()
                .child(name.clone()),
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            if is_dir {
                this.toggle_dir(path.clone(), cx);
            } else {
                this.not_implemented(format!("{} · {}", t!("open_file"), name), cx);
            }
        }))
        .into_any_element()
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
