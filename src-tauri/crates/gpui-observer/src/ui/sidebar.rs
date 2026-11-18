//! Compact workflow sidebar: the icon rail and the expanded session list.
//!
//! Mirrors `src/components/workflow/WorkflowSidebar.vue`: a collapsed 50px
//! column of initial badges with a status dot, or the full list where every
//! row carries the title and the backend status.

use gpui_kit::component::{h_flex, v_flex, ActiveTheme};
use gpui_kit::*;
use rust_i18n::t;

use crate::Observer;

/// Renders the sidebar for the current collapse state.
pub fn sidebar(observer: &Observer, window: &Window, cx: &mut Context<Observer>) -> AnyElement {
    // The sidebar is flush with the window's bottom-left corner, so it rounds
    // that corner itself: GPUI clips children to a rectangular mask.
    let radii = crate::ui::frame::inner_corner_radii(window, cx);
    if observer.sidebar_collapsed {
        collapsed(observer, radii, cx)
    } else {
        expanded(observer, radii, cx)
    }
}

fn expanded(observer: &Observer, radii: Corners<Pixels>, cx: &mut Context<Observer>) -> AnyElement {
    let border = cx.theme().border;
    let muted = cx.theme().muted_foreground;
    let surface = cx.theme().muted;

    let mut list = v_flex()
        .id("sessions")
        .w(px(crate::ui::SIDEBAR_WIDTH))
        .h_full()
        .flex_none()
        .overflow_y_scroll()
        .gap_1()
        .p_3()
        .border_r_1()
        .border_color(border)
        .rounded_bl(radii.bottom_left)
        .bg(surface)
        .child(
            h_flex()
                .w_full()
                .items_center()
                .gap_2()
                .pb_2()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(11.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(muted)
                        .child(t!("sessions").to_string()),
                )
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(muted)
                        .child(observer.sessions.len().to_string()),
                ),
        );

    for (index, session) in observer.sessions.iter().enumerate() {
        list = list.child(observer.session_row(index, session, cx));
    }
    if observer.sessions.is_empty() {
        list = list.child(
            div()
                .pt_2()
                .text_size(px(11.))
                .text_color(muted)
                .child(t!("no_sessions").to_string()),
        );
    }
    list.into_any_element()
}

fn collapsed(
    observer: &Observer,
    radii: Corners<Pixels>,
    cx: &mut Context<Observer>,
) -> AnyElement {
    let border = cx.theme().border;
    let surface = cx.theme().muted;
    let mut rail = v_flex()
        .id("sessions-rail")
        .w(px(crate::ui::SIDEBAR_COLLAPSED_WIDTH))
        .h_full()
        .flex_none()
        .items_center()
        .gap_1()
        .py_3()
        .overflow_y_scroll()
        .border_r_1()
        .border_color(border)
        .rounded_bl(radii.bottom_left)
        .bg(surface);
    for (index, session) in observer.sessions.iter().enumerate() {
        rail = rail.child(observer.rail_item(index, session, cx));
    }
    rail.into_any_element()
}
