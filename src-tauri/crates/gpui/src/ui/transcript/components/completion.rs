//! Snapshot and completion cards; expansion state belongs to the transcript.

use gpui_kit::component::{h_flex, v_flex, ActiveTheme};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::{
    accent, danger, success,
    ui::{file_editor, Icon},
    warning, Observer,
};
use chatspeed_gpui::presentation::{
    file_changes_title, ActivityView, FinishTaskView, SnapshotView, TaskFileKind,
};

use super::cost_card;

pub(crate) fn snapshot_card(
    snapshot: &SnapshotView,
    markdown: Option<AnyElement>,
    on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let open = markdown.is_some();
    let muted = cx.theme().muted_foreground;
    let mut body = v_flex()
        .w_full()
        .min_w_0()
        .mb_2()
        .rounded(px(8.))
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().muted)
        .overflow_hidden()
        .child(
            h_flex()
                .id(SharedString::from(snapshot.key.clone()))
                .w_full()
                .min_w_0()
                .items_center()
                .gap_2()
                .px_3()
                .py_2()
                .cursor_pointer()
                .hover(|style| style.bg(cx.theme().background))
                .child(Icon::new("compress").size(14.).color(accent()))
                .child(
                    div()
                        .flex_none()
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(cx.theme().foreground)
                        .child(snapshot.title.clone()),
                )
                .when(!snapshot.preview.is_empty(), |view| {
                    view.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(10.))
                            .text_color(muted)
                            .child(snapshot.preview.clone()),
                    )
                })
                .when(snapshot.preview.is_empty(), |view| {
                    view.child(div().flex_1())
                })
                .child(crate::ui::rotating_icon(
                    snapshot.key.clone(),
                    "double-arrow-down",
                    14.,
                    muted,
                    if open { 180. } else { 0. },
                ))
                .on_click(on_toggle),
        );
    if let Some(markdown) = markdown {
        body = body.child(
            div()
                .w_full()
                .min_w_0()
                .px_3()
                .py_2()
                .border_t_1()
                .border_color(cx.theme().border)
                .child(markdown),
        );
    }
    body.into_any_element()
}

fn task_files_card(
    view: &FinishTaskView,
    open: bool,
    on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &mut Context<Observer>,
) -> AnyElement {
    let mut counts = [0; 3];
    for file in &view.files {
        counts[match file.kind {
            TaskFileKind::Added => 0,
            TaskFileKind::Modified => 1,
            TaskFileKind::Deleted => 2,
        }] += 1;
    }
    let group = ActivityView {
        key: format!("{}:files", view.key),
        icon: "edit",
        summary: file_changes_title(counts, &rust_i18n::locale()),
        thought_count: 0,
        error_count: 0,
        ongoing: false,
        items: Vec::new(),
    };
    let hover = cx.theme().muted;
    let files = open.then(|| {
        view.files
            .iter()
            .enumerate()
            .map(|(index, file)| {
                let color = match file.kind {
                    TaskFileKind::Added => success(),
                    TaskFileKind::Modified => warning(),
                    TaskFileKind::Deleted => danger(),
                };
                let row = h_flex()
                    .w_full()
                    .min_w_0()
                    .gap_2()
                    .text_size(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_color(color)
                            .child(file.path.clone()),
                    )
                    .when_some(file.lines, |row, (added, removed)| {
                        row.child(
                            div()
                                .flex_none()
                                .text_color(danger())
                                .child(format!("+{added}")),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_color(success())
                                .child(format!("--{removed}")),
                        )
                    });
                if let Some(path) = file.editor_path().and_then(file_editor::text_action_path) {
                    let path = path.to_path_buf();
                    row.id(format!("{}:{index}", group.key))
                        .cursor_pointer()
                        .hover(move |style| style.bg(hover))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            file_editor::open_text(this, path.clone(), window, cx);
                        }))
                        .into_any_element()
                } else {
                    row.into_any_element()
                }
            })
            .collect()
    });
    super::super::rows::activity_group(&group, false, files, on_toggle, cx)
}

pub(crate) fn finish_task_card(
    view: &FinishTaskView,
    open: bool,
    on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    files_open: bool,
    on_toggle_files: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &mut Context<Observer>,
) -> AnyElement {
    let (icon, color) = if view.failed {
        ("check-x", danger())
    } else {
        ("check-circle", success())
    };
    let mut label = h_flex()
        .id(SharedString::from(view.key.clone()))
        .group("completion-header")
        .flex_none()
        .max_w_full()
        .min_w_0()
        .items_center()
        .gap_2()
        .px(px(2.))
        .child(Icon::new(icon).size(15.).color(color))
        .child(
            div()
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(color)
                .child(view.label.clone()),
        );
    if view.cost.is_some() {
        label = label
            .child(crate::ui::disclosure_caret(
                "completion-header",
                open,
                cx.theme().muted_foreground,
            ))
            .cursor_pointer()
            .on_click(on_toggle);
    }
    // A column's default cross-axis stretch would make the press target span the
    // full row. Keep the header in a row so only its icon and label are hit.
    let mut body = v_flex()
        .w_full()
        .min_w_0()
        .gap_2()
        .when(!view.files.is_empty(), |body| {
            body.child(task_files_card(view, files_open, on_toggle_files, cx))
        })
        .child(h_flex().w_full().min_w_0().child(label));
    if open {
        if let Some(cost) = &view.cost {
            body = body.child(cost_card(cost, cx));
        }
    }
    body.into_any_element()
}
