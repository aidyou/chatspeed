//! Element Plus underlined tabs, shared by the native configuration dialogs.

use std::rc::Rc;

use gpui_kit::component::{h_flex, ActiveTheme};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

/// Layout spacing is measured in 4px increments; borders and optical offsets are separate.
pub(crate) const SPACE_4: f32 = 4.;
pub(crate) const SPACE_8: f32 = 8.;
pub(crate) const SPACE_16: f32 = 16.;

/// A single-selection capsule shared by configuration forms.
pub(crate) fn capsule_segments(
    id_prefix: &str,
    options: Vec<(&'static str, String)>,
    selected: &str,
    on_select: impl Fn(&'static str, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    use gpui_kit::component::button::{Button, ButtonRounded, ButtonVariants};
    let on_select = Rc::new(on_select);
    h_flex()
        .id(SharedString::from(id_prefix.to_owned()))
        .flex_wrap()
        .gap(px(SPACE_4))
        .p(px(SPACE_4))
        .rounded_full()
        .bg(cx.theme().muted)
        .children(options.into_iter().map(|(value, label)| {
            let callback = on_select.clone();
            Button::new(SharedString::from(value))
                .rounded(ButtonRounded::Size(px(12.)))
                .h(px(24.))
                .px(px(SPACE_8))
                .py(px(0.))
                .label(label)
                .map(|button| if selected == value { button.primary() } else { button.ghost() })
                .on_click(move |_, window, cx| callback(value, window, cx))
        }))
        .into_any_element()
}

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
