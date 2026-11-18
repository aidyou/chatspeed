//! Custom titlebar: session identity, shell actions and the window controls.
//!
//! Mirrors `src/components/window/Titlebar.vue` in the parts the workflow
//! window uses: the sidebar toggle on the left, the session title in the
//! middle, and the workflow actions on the right.
//!
//! The bar is drawn here rather than by `gpui_kit::component::TitleBar` because
//! that component always appends its own window controls: their hover fill is a
//! rectangle the full height of the bar, which paints over the frame's rounded
//! top-right corner.

use gpui_kit::base::InteractiveElementExt;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme, Icon, IconName, Sizable};
use gpui_kit::*;
use rust_i18n::t;

use crate::{font_icon, ui::frame, Observer};

/// Width of a window control button, matching the platform convention.
const CONTROL_WIDTH: f32 = 34.;

/// Renders the 40px titlebar above the sidebar and transcript.
pub fn titlebar(observer: &Observer, window: &Window, cx: &mut Context<Observer>) -> AnyElement {
    let border = cx.theme().border;
    let foreground = cx.theme().foreground;
    let muted = cx.theme().muted_foreground;
    let hover = cx.theme().muted;
    let accent = cx.theme().primary;
    let collapsed = observer.sidebar_collapsed;

    let mut bar = h_flex()
        .id("title-bar")
        .w_full()
        .h(px(crate::ui::TITLEBAR_HEIGHT))
        .flex_none()
        .items_center()
        .gap_2()
        .pl_3()
        .border_b_1()
        .border_color(border)
        // A press that a control did not take starts a native window move once
        // the pointer travels, so a plain click never moves the window.
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, window, _| {
                this.title_drag = !window.default_prevented();
            }),
        )
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|this, _, _, _| this.title_drag = false),
        )
        .on_mouse_move(cx.listener(
            |this, event: &MouseMoveEvent, window, cx| {
                if event.pressed_button != Some(MouseButton::Left) {
                    this.title_drag = false;
                    return;
                }
                if !this.title_drag {
                    return;
                }
                this.title_drag = false;
                cx.stop_active_drag(window);
                window.start_window_move();
            },
        ))
        .on_double_click(cx.listener(|_, _, window, _| window.zoom_window()))
        .on_mouse_down(MouseButton::Right, |event: &MouseDownEvent, window, _| {
            window.show_window_menu(event.position);
        })
        .child(crate::ui::icon_button(
            "toggle-sidebar",
            "sidebar",
            if collapsed { muted } else { accent },
            hover,
            cx.listener(|this, _, _, cx| this.toggle_sidebar(cx)),
        ))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(1.))
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(foreground)
                        .child(observer.window_title()),
                )
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .text_size(px(10.))
                        .text_color(muted)
                        .child(observer.status_line()),
                ),
        );

    // A paused tail follow is the only state the titlebar has to surface: the
    // transcript itself explains why it stopped moving.
    if !observer.follow {
        bar = bar.child(
            h_flex()
                .id("follow-titlebar")
                .flex_none()
                .items_center()
                .gap_1()
                .px_2()
                .py(px(3.))
                .rounded(px(6.))
                .cursor_pointer()
                .text_size(px(11.))
                .text_color(accent)
                .hover(|style| style.bg(hover))
                .child(font_icon("double-arrow-down", 12., accent))
                .child(t!("follow").to_string())
                .on_click(cx.listener(|this, _, _, cx| this.resume_follow(cx))),
        );
    }

    bar = bar.child(crate::ui::icon_button(
        "new-session",
        "new-chat",
        muted,
        hover,
        cx.listener(|this, _, window, cx| this.start_new_session(window, cx)),
    ));
    bar = bar.child(window_controls(window, cx));
    bar.into_any_element()
}

/// Window controls for client-decorated windows; the window manager draws its
/// own under server-side decorations.
fn window_controls(window: &Window, cx: &mut Context<Observer>) -> AnyElement {
    if !matches!(window.window_decorations(), Decorations::Client { .. }) {
        return div().into_any_element();
    }
    let supported = window.window_controls();
    let radii = frame::inner_corner_radii(window, cx);

    let mut row = h_flex()
        .id("window-controls")
        .flex_none()
        .items_center()
        .h_full();
    if supported.minimize {
        row = row.child(control(
            "window-minimize",
            IconName::WindowMinimize,
            px(0.),
            cx,
            |window, _| window.minimize_window(),
        ));
    }
    if supported.maximize {
        let icon = if window.is_maximized() {
            IconName::WindowRestore
        } else {
            IconName::WindowMaximize
        };
        row = row.child(control(
            "window-maximize",
            icon,
            px(0.),
            cx,
            |window, _| window.zoom_window(),
        ));
    }
    // The close button sits in the frame's top-right corner, so it rounds that
    // corner instead of filling it with a red square.
    row.child(control(
        "window-close",
        IconName::WindowClose,
        radii.top_right,
        cx,
        |window, _| window.remove_window(),
    ))
    .into_any_element()
}

fn control(
    id: &'static str,
    icon: IconName,
    radius: Pixels,
    cx: &App,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let is_close = id == "window-close";
    let theme = cx.theme();
    let (hover_bg, active_bg, hover_fg) = if is_close {
        (theme.danger, theme.danger_active, theme.danger_foreground)
    } else {
        (
            theme.secondary_hover,
            theme.secondary_active,
            theme.secondary_foreground,
        )
    };
    let foreground = theme.foreground;

    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .w(px(CONTROL_WIDTH))
        .h_full()
        .rounded_tr(radius)
        .text_color(foreground)
        .hover(|style| style.bg(hover_bg).text_color(hover_fg))
        .active(|style| style.bg(active_bg).text_color(hover_fg))
        // The press must not reach the titlebar, which would start a move.
        .on_mouse_down(MouseButton::Left, |_, window, cx| {
            window.prevent_default();
            cx.stop_propagation();
        })
        .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            on_click(window, cx);
        })
        .child(Icon::new(icon).small())
        .into_any_element()
}
