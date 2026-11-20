//! Pending user messages, separate from durable transcript rows.

use chatspeed_gpui::model::QueuedMessage;
use gpui_kit::component::{scroll::ScrollableElement as _, v_flex, ActiveTheme};
use gpui_kit::*;
use rust_i18n::t;

pub(super) fn render(items: &[QueuedMessage], cx: &App) -> AnyElement {
    v_flex()
        .id("queued-user-messages")
        .w_full()
        .max_w(px(780.))
        .gap_1()
        .child(div().text_size(px(11.)).text_color(cx.theme().muted_foreground)
            .child(t!("queued_messages").to_string()))
        .child(v_flex().w_full().max_h(px(96.)).gap_1()
            .children(items.iter().map(|item| {
                div().id(SharedString::from(item.id.clone())).w_full().min_w_0()
                    .px_2().py_1().rounded(px(6.)).bg(cx.theme().muted)
                    .text_size(px(12.)).overflow_hidden().whitespace_nowrap().text_ellipsis()
                    .child(if item.content.trim().is_empty() { t!("queued_message_empty").to_string() } else { item.content.clone() })
            })).overflow_y_scrollbar().id("queued-user-messages-scroll"))
        .into_any_element()
}
