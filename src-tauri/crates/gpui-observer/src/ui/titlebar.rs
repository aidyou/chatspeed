//! Custom titlebar: shell actions, the authorized-path chip and the window
//! controls.
//!
//! Mirrors `src/components/window/Titlebar.vue` plus the slots the workflow view
//! fills in: the menu, the sidebar toggle, the approval queue and the sound menu
//! on the left, the authorized-path chip in the middle, and the workflow actions
//! on the right. The bar carries no session title, matching Vue.
//!
//! The bar is drawn here rather than by `gpui_kit::component::TitleBar` because
//! that component always appends its own window controls: their hover fill is a
//! rectangle the full height of the bar, which paints over the frame's rounded
//! top-right corner.

use gpui_kit::base::InteractiveElementExt;
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme, Icon, IconName, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use rust_i18n::t;

use crate::ui::{icon_button, surfaces, trigger, TITLEBAR_HEIGHT};
use crate::{font_icon, Observer};

/// Width of a window control button, matching the platform convention.
const CONTROL_WIDTH: f32 = 34.;

/// Vue pads the bar with `--cs-space`; macOS hands the leading edge to the
/// native traffic lights, so the first icon clears them.
const LEADING_INSET: f32 = 16.;
const MACOS_TRAFFIC_LIGHTS_INSET: f32 = 78.;

/// Renders the 40px titlebar above the sidebar and transcript.
pub fn titlebar(observer: &Observer, window: &Window, cx: &mut Context<Observer>) -> AnyElement {
    let skin = surfaces(observer.appearance_dark);
    let secondary = cx.theme().muted_foreground;
    let foreground = cx.theme().foreground;
    let accent = cx.theme().primary;
    let collapsed = observer.sidebar_collapsed;
    let radii = crate::ui::frame::inner_corner_radii(window, cx);

    // Vue spaces titlebar buttons with `.left/.right { gap: 4px }` plus
    // `.icon-btn { margin-right: 4px }`, so an 8px gap matches the web bar.
    let mut left = h_flex()
        .flex_none()
        .items_center()
        .gap(px(8.))
        .child(menu(observer, secondary, cx))
        .child(icon_button(
            "toggle-sidebar",
            "sidebar",
            if collapsed { secondary } else { accent },
            skin.hover,
            cx.listener(|this, _, _, cx| this.toggle_sidebar(cx)),
        ));
    let pending = observer.pending_approvals();
    if !pending.is_empty() {
        left = left.child(approval_queue(pending, secondary, accent, skin.hover, cx));
    }
    left = left.child(sound_menu(observer, secondary, skin.hover, cx));

    let center = h_flex().flex_1().min_w_0().justify_center().when_some(
        observer.roots().first().cloned(),
        |row, root| {
            row.child(
                h_flex()
                    .id("titlebar-path")
                    .flex_none()
                    .min_w_0()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py(px(2.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .hover(move |style| style.bg(skin.hover))
                    .child(font_icon("ext-folder", 12., secondary))
                    .child(
                        div()
                            .max_w(px(320.))
                            .min_w_0()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(foreground)
                            .text_ellipsis()
                            .child(chatspeed_gpui_observer::types::root_name(&root).to_owned()),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.open_files_tab(cx))),
            )
        },
    );

    // The four workflow actions keep Vue's order: automation, new task, undo,
    // pin. The window controls follow them, flush with the right edge.
    let right = h_flex()
        .flex_none()
        .items_center()
        .gap(px(8.))
        .child(icon_button(
            "create-automation",
            "clock",
            secondary,
            skin.hover,
            cx.listener(|this, _, _, cx| this.not_implemented(t!("create_automation"), cx)),
        ))
        .child(icon_button(
            "new-session",
            "new-chat",
            secondary,
            skin.hover,
            cx.listener(|this, _, window, cx| this.start_new_session(window, cx)),
        ))
        .child(icon_button(
            "rewind",
            "undo",
            secondary,
            skin.hover,
            cx.listener(|this, _, _, cx| this.not_implemented(t!("delete_last_message"), cx)),
        ))
        .child(icon_button(
            "pin-window",
            "pin",
            if observer.pinned { accent } else { secondary },
            skin.hover,
            cx.listener(|this, _, _, cx| this.toggle_pin(cx)),
        ))
        .child(div().flex_none().w(px(16.)))
        .child(window_controls(window, cx));

    h_flex()
        .id("title-bar")
        .w_full()
        .h(px(TITLEBAR_HEIGHT))
        .flex_none()
        .items_center()
        .gap_2()
        .pl(px(if cfg!(target_os = "macos") {
            MACOS_TRAFFIC_LIGHTS_INSET
        } else {
            LEADING_INSET
        }))
        // The window clips its children to a rectangular mask, so the bar has to
        // round its own top corners; the inner radius keeps its arc concentric
        // with the kit's outline instead of stepping against it.
        .rounded_tl(radii.top_left)
        .rounded_tr(radii.top_right)
        .border_b_1()
        .border_color(skin.titlebar_border)
        // Vue's bar is a lighter strip over the body surface with a hairline
        // border; the soft shadow below it is painted by the frame.
        .bg(skin.titlebar)
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
        .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
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
        }))
        .on_double_click(cx.listener(|_, _, window, _| window.zoom_window()))
        .on_mouse_down(MouseButton::Right, |event: &MouseDownEvent, window, _| {
            window.show_window_menu(event.position);
        })
        .child(left)
        .child(center)
        .child(right)
        .into_any_element()
}

/// One command in the native menu.
#[derive(Clone, Copy)]
enum Action {
    NewSession,
    ToggleSidebar,
    TogglePin,
    Close,
}

/// Vue's menu button, limited to the commands the native window can perform.
fn menu(observer: &Observer, secondary: Hsla, cx: &mut Context<Observer>) -> AnyElement {
    let skin = surfaces(observer.appearance_dark);
    let hover = skin.hover;
    let weak = cx.entity().downgrade();
    let entries: Vec<(&'static str, &'static str, String, Action)> = vec![
        (
            "menu-new",
            "new-chat",
            t!("new_task").to_string(),
            Action::NewSession,
        ),
        (
            "menu-sidebar",
            "sidebar",
            if observer.sidebar_collapsed {
                t!("expand_sidebar")
            } else {
                t!("collapse_sidebar")
            }
            .to_string(),
            Action::ToggleSidebar,
        ),
        (
            "menu-pin",
            "pin",
            if observer.pinned {
                t!("unpin")
            } else {
                t!("pin")
            }
            .to_string(),
            Action::TogglePin,
        ),
        (
            "menu-close",
            "close",
            t!("close_window").to_string(),
            Action::Close,
        ),
    ];

    Popover::new("titlebar-menu")
        .anchor(Anchor::TopLeft)
        .offset(px(6.))
        .trigger(trigger(flat_button(
            "menu-button",
            "menu",
            secondary,
            hover,
        )))
        .content(move |_, _, cx| {
            let popover = cx.entity();
            let mut list = v_flex().w(px(196.)).gap(px(2.));
            for (id, icon, label, action) in entries.clone() {
                let popover = popover.clone();
                let weak = weak.clone();
                list = list.child(menu_row(
                    id,
                    icon,
                    label,
                    secondary,
                    hover,
                    move |_, window, cx| {
                        let _ = popover.update(cx, |state, cx| state.dismiss(window, cx));
                        let _ = weak.update(cx, |this, cx| match action {
                            Action::NewSession => this.start_new_session(window, cx),
                            Action::ToggleSidebar => this.toggle_sidebar(cx),
                            Action::TogglePin => this.toggle_pin(cx),
                            Action::Close => window.remove_window(),
                        });
                    },
                ));
            }
            list
        })
        .into_any_element()
}

/// The approval queue: only rendered while the runtime waits on a decision.
fn approval_queue(
    pending: Vec<(String, String)>,
    secondary: Hsla,
    accent: Hsla,
    hover: Hsla,
    cx: &mut Context<Observer>,
) -> AnyElement {
    let count = if pending.len() > 9 {
        "9+".to_owned()
    } else {
        pending.len().to_string()
    };
    let badge = crate::success();
    let foreground = cx.theme().foreground;
    let weak = cx.entity().downgrade();

    let button = div()
        .relative()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .w(px(26.))
        .h(px(22.))
        // Vue's `.approval-queue-btn` keeps `--cs-space` on the right; with the
        // 8px group gap this comes to the same 20px before the sound button.
        .mr(px(12.))
        .rounded(px(8.))
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(font_icon("approval", 18., secondary))
        .child(
            div()
                .absolute()
                .top(px(-5.))
                .right(px(-5.))
                .w(px(18.))
                .h(px(18.))
                .rounded_full()
                .bg(badge)
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(10.))
                .text_color(rgb(0xffffff))
                .child(count),
        );

    Popover::new("titlebar-approvals")
        .anchor(Anchor::TopLeft)
        .offset(px(6.))
        .trigger(trigger(button))
        .content(move |_, _, _| {
            let mut list = v_flex().w(px(240.)).gap(px(2.));
            for (index, (id, name)) in pending.iter().enumerate() {
                let weak = weak.clone();
                list = list.child(
                    v_flex()
                        .id(("approval", index))
                        .w_full()
                        .gap(px(2.))
                        .px_2()
                        .py(px(6.))
                        .rounded(px(6.))
                        .cursor_pointer()
                        .hover(move |style| style.bg(hover))
                        .child(
                            h_flex()
                                .items_center()
                                .gap_2()
                                .child(font_icon("approval", 14., accent))
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .text_color(foreground)
                                        .child(name.clone()),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(secondary)
                                .child(id.clone()),
                        )
                        .on_click(move |_, _, cx| {
                            // The decision card sits inline at the tail of the
                            // transcript: bring the reader back to it.
                            let _ = weak.update(cx, |this, cx| {
                                this.follow = true;
                                this.model.resume_follow();
                                this.scroll_pending = true;
                                cx.notify();
                            });
                        }),
                );
            }
            list
        })
        .into_any_element()
}

/// Approval and completion chimes, as the two switches Vue nests in this menu.
fn sound_menu(
    observer: &Observer,
    secondary: Hsla,
    hover: Hsla,
    cx: &mut Context<Observer>,
) -> AnyElement {
    let muted = observer.approval_sound_muted && observer.completion_sound_muted;
    let icon = if muted { "mute" } else { "unmute" };
    let approval_on = !observer.approval_sound_muted;
    let completion_on = !observer.completion_sound_muted;
    let accent = cx.theme().primary;
    let foreground = cx.theme().foreground;
    let weak = cx.entity().downgrade();

    Popover::new("titlebar-sound")
        .anchor(Anchor::TopLeft)
        .offset(px(6.))
        .trigger(trigger(flat_button("sound-button", icon, secondary, hover)))
        .content(move |_, _, cx| {
            let popover = cx.entity();
            let approval = {
                let popover = popover.clone();
                let weak = weak.clone();
                switch_row(
                    "sound-approval",
                    t!("sound_approval").to_string(),
                    approval_on,
                    accent,
                    foreground,
                    secondary,
                    hover,
                    move |window, cx| {
                        let _ = popover.update(cx, |state, cx| state.dismiss(window, cx));
                        let _ = weak.update(cx, |this, cx| {
                            this.approval_sound_muted = !this.approval_sound_muted;
                            cx.notify();
                        });
                    },
                )
            };
            let completion = {
                let popover = popover.clone();
                let weak = weak.clone();
                switch_row(
                    "sound-completion",
                    t!("sound_completion").to_string(),
                    completion_on,
                    accent,
                    foreground,
                    secondary,
                    hover,
                    move |window, cx| {
                        let _ = popover.update(cx, |state, cx| state.dismiss(window, cx));
                        let _ = weak.update(cx, |this, cx| {
                            this.completion_sound_muted = !this.completion_sound_muted;
                            cx.notify();
                        });
                    },
                )
            };
            v_flex()
                .w(px(180.))
                .gap(px(2.))
                .child(approval)
                .child(completion)
        })
        .into_any_element()
}

/// A flat 26x22 trigger surface: 18px glyph, Vue's `.icon-btn` box.
fn flat_button(id: &'static str, icon: &'static str, color: Hsla, hover: Hsla) -> impl IntoElement {
    h_flex()
        .id(id)
        .flex_none()
        .items_center()
        .justify_center()
        .w(px(26.))
        .h(px(22.))
        .rounded(px(8.))
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(font_icon(icon, 18., color))
}

fn menu_row(
    id: &'static str,
    icon: &'static str,
    label: String,
    text: Hsla,
    hover: Hsla,
    run: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    h_flex()
        .id(id)
        .w_full()
        .items_center()
        .gap_2()
        .px_2()
        .py(px(6.))
        .rounded(px(6.))
        .cursor_pointer()
        .text_color(text)
        .hover(move |style| style.bg(hover))
        .child(font_icon(icon, 14., text))
        .child(div().text_size(px(13.)).child(label))
        .on_click(run)
        .into_any_element()
}

/// One switch row: a check glyph instead of a checkbox, matching the Vue menu.
fn switch_row(
    id: &'static str,
    label: String,
    enabled: bool,
    accent: Hsla,
    foreground: Hsla,
    secondary: Hsla,
    hover: Hsla,
    run: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    h_flex()
        .id(id)
        .w_full()
        .items_center()
        .gap_2()
        .px_2()
        .py(px(6.))
        .rounded(px(6.))
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .child(font_icon(
            if enabled { "check" } else { "uncheck" },
            14.,
            if enabled { accent } else { secondary },
        ))
        .child(div().text_size(px(13.)).text_color(foreground).child(label))
        .on_click(move |_, window, cx| run(window, cx))
        .into_any_element()
}

/// Window controls for client-decorated windows; the window manager draws its
/// own under server-side decorations.
fn window_controls(window: &Window, cx: &mut Context<Observer>) -> AnyElement {
    if !matches!(window.window_decorations(), Decorations::Client { .. }) {
        return div().into_any_element();
    }
    let supported = window.window_controls();
    let radii = crate::ui::frame::inner_corner_radii(window, cx);

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
        row = row.child(control("window-maximize", icon, px(0.), cx, |window, _| {
            window.zoom_window()
        }));
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
