//! Pending user messages, separate from durable transcript rows.

use gpui_kit::component::{h_flex, v_flex, ActiveTheme};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use rust_i18n::t;

use crate::{
    ui::{surfaces, Icon, Spin},
    Observer,
};

pub(crate) fn render(observer: &Observer, cx: &mut Context<Observer>) -> AnyElement {
    let hover = surfaces(observer.appearance_dark).hover;
    let muted = cx.theme().muted_foreground;
    let foreground = cx.theme().foreground;
    let button_hover = cx.theme().muted;
    v_flex()
        .w_full()
        .gap_1()
        .children(observer.model.queued_messages.iter().map(|item| {
            let id = item.id.clone();
            let generation = observer.model.generation;
            let removing = observer.model.queue_removal_pending(&item.id);
            let attachments = item.metadata["attachments"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|attachment| {
                    let name = attachment["name"].as_str().unwrap_or_default();
                    // Reuse only native image paths, not arbitrary remote URLs from metadata.
                    if let Some(path) = attachment["path"]
                        .as_str()
                        .map(std::path::PathBuf::from)
                        .filter(|path| path.is_absolute() && crate::is_image_path(path))
                    {
                        Some(
                            img(path)
                                .size(px(72.))
                                .rounded(px(8.))
                                .object_fit(ObjectFit::Cover)
                                .into_any_element(),
                        )
                    } else if !name.is_empty() {
                        Some(
                            div()
                                .max_w(px(180.))
                                .text_size(px(10.))
                                .text_color(muted)
                                .child(name.to_owned())
                                .into_any_element(),
                        )
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            h_flex().w_full().justify_end().child(
                h_flex()
                    .id(SharedString::from(item.id.clone()))
                    .max_w(relative(0.8))
                    .min_w_0()
                    .items_start()
                    .gap_1()
                    .px_4()
                    .py_2()
                    .rounded(px(16.))
                    .bg(hover)
                    .child(
                        div()
                            .flex_none()
                            .mt(px(2.))
                            .child(Icon::new("clock").size(12.).color(muted)),
                    )
                    .child(
                        v_flex()
                            .min_w_0()
                            .gap(px(2.))
                            .when(!item.content.is_empty(), |column| {
                                column.child(
                                    div()
                                        .text_size(px(12.))
                                        .line_height(px(18.))
                                        .text_color(foreground)
                                        .child(item.content.clone()),
                                )
                            })
                            .when(!attachments.is_empty(), |column| {
                                column.child(h_flex().flex_wrap().gap_1().children(attachments))
                            })
                            .when(!removing && item.provisional, |column| {
                                column.child(queue_status(
                                    &item.id,
                                    t!("queued_message_sending").to_string(),
                                    muted,
                                ))
                            })
                            .when(removing, |column| {
                                column.child(queue_status(
                                    &item.id,
                                    t!("queued_message_removing").to_string(),
                                    muted,
                                ))
                            })
                            .when(
                                item.content.is_empty()
                                    && item.metadata["attachments"]
                                        .as_array()
                                        .is_none_or(Vec::is_empty),
                                |column| column.child(t!("queued_message_empty").to_string()),
                            ),
                    )
                    .when(!removing, |row| {
                        row.child(
                            h_flex()
                                .flex_none()
                                .items_center()
                                .gap(px(2.))
                                .child(queue_action(
                                    SharedString::from(format!("queue-edit-{id}")),
                                    "queue-edit",
                                    "edit",
                                    muted,
                                    foreground,
                                    button_hover,
                                    {
                                        let id = id.clone();
                                        cx.listener(move |this, _, window, cx| {
                                            this.edit_queued_message(generation, &id, window, cx)
                                        })
                                    },
                                ))
                                .child(queue_action(
                                    SharedString::from(format!("queue-remove-{id}")),
                                    "queue-remove",
                                    "close",
                                    muted,
                                    foreground,
                                    button_hover,
                                    cx.listener(move |this, _, _, cx| {
                                        this.remove_queued_message(generation, &id, cx);
                                    }),
                                )),
                        )
                    }),
            )
        }))
        .into_any_element()
}

/// One waiting label under a queued message.
///
/// Enqueueing and removing are both requests the runtime has to acknowledge, so
/// they share the spinner and the muted type instead of reading as two
/// different kinds of pending work.
fn queue_status(id: &str, label: String, muted: Hsla) -> AnyElement {
    h_flex()
        .items_center()
        .gap(px(4.))
        .text_size(px(10.))
        .text_color(muted)
        .child(Icon::new("loading").size(10.).color(muted).spin(
            SharedString::from(format!("queue-status-{id}")),
            Spin::Accelerate,
        ))
        .child(label)
        .into_any_element()
}

/// One icon-only action beside a queued message.
///
/// Both actions share the same target, glyph size, and hover so the row reads
/// as content plus a pair of peer actions instead of a primary and a dismiss.
fn queue_action(
    id: SharedString,
    group: &'static str,
    icon: &'static str,
    muted: Hsla,
    foreground: Hsla,
    hover: Hsla,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    h_flex()
        .id(id)
        .flex_none()
        .size(px(24.))
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .cursor_pointer()
        .group(group)
        .hover(move |style| style.bg(hover))
        .child(
            div()
                .text_color(muted)
                .group_hover(group, move |style| style.text_color(foreground))
                .child(Icon::new(icon).size(12.)),
        )
        .on_click(on_click)
        .into_any_element()
}
