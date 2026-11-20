//! Pending user messages, separate from durable transcript rows.

use gpui_kit::component::{h_flex, v_flex, ActiveTheme};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use rust_i18n::t;

use crate::{
    ui::{surfaces, Icon},
    Observer,
};

pub(crate) fn render(observer: &Observer, cx: &mut Context<Observer>) -> AnyElement {
    let hover = surfaces(observer.appearance_dark).hover;
    let muted = cx.theme().muted_foreground;
    let foreground = cx.theme().foreground;
    v_flex()
        .w_full()
        .gap_1()
        .children(observer.model.queued_messages.iter().map(|item| {
            let id = item.id.clone();
            let generation = observer.model.generation;
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
                            .when(item.provisional, |column| {
                                column.child(
                                    div()
                                        .text_size(px(10.))
                                        .text_color(muted)
                                        .child(t!("queued_message_sending").to_string()),
                                )
                            })
                            .when(
                                item.content.is_empty()
                                    && item.metadata["attachments"]
                                        .as_array()
                                        .is_none_or(Vec::is_empty),
                                |column| column.child(t!("queued_message_empty").to_string()),
                            ),
                    )
                    .child(
                        h_flex()
                            .id("remove")
                            .flex_none()
                            .size(px(24.))
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .cursor_pointer()
                            .group("queue-remove")
                            .hover(|style| style.bg(cx.theme().muted))
                            .child(
                                div()
                                    .id("remove-icon")
                                    .text_color(muted)
                                    .group_hover("queue-remove", |style| {
                                        style.text_color(foreground)
                                    })
                                    .child(Icon::new("close").size(12.)),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.remove_queued_message(generation, &id, cx)
                            })),
                    ),
            )
        }))
        .into_any_element()
}
