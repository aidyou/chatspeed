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

use chatspeed_gpui_observer::chimes::Chime;
use chatspeed_runtime_client::desktop_menu::Action;
use gpui_kit::base::InteractiveElementExt;
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme, Icon as KitIcon, IconName, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use rust_i18n::t;

use crate::ui::{icon_button, surfaces, trigger, Icon, TITLEBAR_HEIGHT};
use crate::Observer;

/// Width of a window control button, matching the platform convention.
const CONTROL_WIDTH: f32 = 34.;

/// Distance from the window edge to the first and last titlebar control.
///
/// Both groups keep that same inset, so the bar looks the same whether the
/// window controls sit on the right or the platform keeps them on the left
/// (macOS traffic lights).
const SIDE_INSET: f32 = 20.;
/// macOS hands the leading edge to the native traffic lights, so the first icon
/// clears them instead of sitting at [`SIDE_INSET`].
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
    let mut left = h_flex().flex_none().items_center().gap(px(8.));
    if observer.show_menu_button {
        left = left.child(menu(observer, secondary, cx));
    }
    left = left.child(icon_button(
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
                    .child(Icon::new("ext-folder").size(12.).color(secondary))
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
    // pin. The window controls follow them behind a wider gap, and the group
    // ends at the same edge inset as the left one.
    let mut right = h_flex()
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
        // Vue tilts the pin 45° while the window is not pinned. GPUI only
        // transforms SVG elements, so the glyph cannot carry that tilt; the
        // button still reports the state it toggles.
        .child(icon_button(
            "pin-window",
            "pin",
            if observer.pinned { accent } else { secondary },
            skin.hover,
            cx.listener(|this, _, _, cx| this.toggle_pin(cx)),
        ));
    if let Some(controls) = window_controls(window, cx) {
        right = right.child(div().flex_none().w(px(16.))).child(controls);
    }

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
            SIDE_INSET
        }))
        .pr(px(SIDE_INSET))
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

/// Menu groups and order match `src/components/window/Titlebar.vue`.
fn menu(observer: &Observer, secondary: Hsla, cx: &mut Context<Observer>) -> AnyElement {
    let skin = surfaces(observer.appearance_dark);
    let hover = skin.hover;
    let border = cx.theme().border;
    let foreground = cx.theme().foreground;
    let weak = cx.entity().downgrade();
    let groups: [&[(&str, &str, &str, Action)]; 4] = [
        &[
            ("menu-chat", "skill-chat-square", "menu_chat", Action::Chat),
            (
                "menu-assistant",
                "assistant",
                "menu_assistant",
                Action::Assistant,
            ),
            (
                "menu-workflow",
                "skill-plan3",
                "menu_workflow",
                Action::Workflow,
            ),
            ("menu-note", "note", "menu_note", Action::Note),
            (
                "menu-proxy-switcher",
                "switch",
                "menu_proxy_switcher",
                Action::ProxySwitcher,
            ),
        ],
        &[
            (
                "menu-settings",
                "setting",
                "menu_settings",
                Action::Settings,
            ),
            ("menu-model", "model", "menu_model", Action::Model),
            ("menu-agent", "agent", "menu_agent", Action::Agent),
            ("menu-proxy", "proxy", "menu_proxy", Action::Proxy),
            (
                "menu-chat-hub",
                "connected",
                "menu_chat_hub",
                Action::ChatHub,
            ),
            ("menu-mcp", "mcp", "menu_mcp", Action::Mcp),
            ("menu-skill", "skill", "menu_skill", Action::Skill),
        ],
        &[(
            "menu-scraper",
            "extract",
            "menu_scraper",
            Action::ScraperTest,
        )],
        &[
            ("menu-about", "about", "menu_about", Action::About),
            ("menu-quit", "quit", "menu_quit", Action::Quit),
        ],
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
        .content(move |_, window, cx| {
            let popover = cx.entity();
            let mut list = v_flex()
                .id("titlebar-menu-list")
                .w(px(196.))
                .max_h((window.viewport_size().height - px(TITLEBAR_HEIGHT + 28.)).max(px(40.)))
                .overflow_y_scroll()
                .gap(px(2.));
            for (index, group) in groups.iter().enumerate() {
                if index > 0 {
                    list = list.child(div().flex_none().mx_2().my_1().h(px(1.)).bg(border));
                }
                for &(id, icon, label, action) in *group {
                    let popover = popover.clone();
                    let weak = weak.clone();
                    list = list.child(menu_row(
                        id,
                        icon,
                        t!(label).to_string(),
                        foreground,
                        hover,
                        move |_, window, cx| {
                            let _ = popover.update(cx, |state, cx| state.dismiss(window, cx));
                            let _ =
                                weak.update(cx, |this, cx| this.desktop_menu_action(action, cx));
                        },
                    ));
                }
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
        .child(Icon::new("approval").size(18.).color(secondary))
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
                                .child(Icon::new("approval").size(14.).color(accent))
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
    let approval_on = observer.sound_is_enabled(Chime::Approval);
    let completion_on = observer.sound_is_enabled(Chime::Completion);
    let icon = if approval_on || completion_on {
        "unmute"
    } else {
        "mute"
    };
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
                            this.toggle_sound(Chime::Approval, cx);
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
                            this.toggle_sound(Chime::Completion, cx);
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
        .child(Icon::new(icon).size(18.).color(color))
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
        .child(Icon::new(icon).size(14.).color(text))
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
        .child(
            Icon::new(if enabled { "check" } else { "uncheck" })
                .size(14.)
                .color(if enabled { accent } else { secondary }),
        )
        .child(div().text_size(px(13.)).text_color(foreground).child(label))
        .on_click(move |_, window, cx| run(window, cx))
        .into_any_element()
}

/// Window controls for client-decorated windows; `None` when the window manager
/// draws its own, which is also the case on macOS with its native traffic
/// lights.
fn window_controls(window: &Window, cx: &mut Context<Observer>) -> Option<AnyElement> {
    if cfg!(target_os = "macos") {
        return None;
    }
    if !matches!(window.window_decorations(), Decorations::Client { .. }) {
        return None;
    }
    let supported = window.window_controls();

    let mut row = h_flex()
        .id("window-controls")
        .flex_none()
        .items_center()
        .h_full();
    if supported.minimize {
        row = row.child(control(
            "window-minimize",
            IconName::WindowMinimize,
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
        row = row.child(control("window-maximize", icon, cx, |window, _| {
            window.zoom_window()
        }));
    }
    Some(
        row.child(control(
            "window-close",
            IconName::WindowClose,
            cx,
            |window, _| window.remove_window(),
        ))
        .into_any_element(),
    )
}

fn control(
    id: &'static str,
    icon: IconName,
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
        .child(KitIcon::new(icon).small())
        .into_any_element()
}
