//! Element Plus underlined tabs, shared by the native configuration dialogs.

use std::rc::Rc;

use gpui_kit::component::{h_flex, ActiveTheme};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

/// Renders Vue's default `el-tabs` header: bottom hairline, primary-colored
/// active label, and a 2px primary underline. The caller owns the active index
/// and the body below the header.
pub(crate) fn underline_tabs(
    id_prefix: &'static str,
    labels: Vec<String>,
    active: usize,
    on_select: impl Fn(usize, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let border = cx.theme().border;
    let primary = cx.theme().primary;
    let muted = cx.theme().muted_foreground;
    let on_select = Rc::new(on_select);
    let mut tabs = h_flex()
        .w_full()
        .min_w_0()
        .items_end()
        .gap(px(24.))
        .border_b_1()
        .border_color(border);
    for (index, label) in labels.into_iter().enumerate() {
        let selected = index == active;
        let callback = on_select.clone();
        tabs = tabs.child(
            div()
                .id(SharedString::from(format!("{id_prefix}-{index}")))
                .flex_none()
                .pb(px(8.))
                .cursor_pointer()
                .text_size(px(13.))
                .text_color(if selected { primary } else { muted })
                .when(selected, |tab| tab.border_b_2().border_color(primary))
                .child(label)
                .on_click(move |_, window, cx| callback(index, window, cx)),
        );
    }
    tabs.into_any_element()
}
