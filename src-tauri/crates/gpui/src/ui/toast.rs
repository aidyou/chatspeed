//! Transient window feedback for user actions, separate from transcript errors.

use std::time::{Duration, Instant};

use gpui_kit::component::{h_flex, ActiveTheme};
use gpui_kit::*;

use super::icon::Icon;

#[derive(Clone, Copy)]
pub enum ToastKind {
    Error,
    Warning,
    Success,
    Info,
}

impl ToastKind {
    pub fn duration(self) -> Duration {
        Duration::from_secs(match self {
            Self::Error => 5,
            Self::Warning => 4,
            Self::Success | Self::Info => 3,
        })
    }
}

pub struct Toast {
    kind: ToastKind,
    message: String,
    expires: Instant,
}

impl Toast {
    pub fn new(kind: ToastKind, message: String, now: Instant, duration: Duration) -> Self {
        Self {
            kind,
            message,
            expires: now + duration,
        }
    }

    pub fn expired(&self, now: Instant) -> bool {
        now >= self.expires
    }

    pub fn render(
        &self,
        top: Pixels,
        on_close: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
        cx: &App,
    ) -> AnyElement {
        let (icon, color) = match self.kind {
            ToastKind::Error => ("check-x", cx.theme().danger),
            ToastKind::Warning => ("warning", cx.theme().warning),
            ToastKind::Success => ("check-circle", cx.theme().success),
            ToastKind::Info => ("ask_user", cx.theme().primary),
        };
        h_flex()
            .absolute()
            .top(top)
            .left_0()
            .right_0()
            .px_3()
            .justify_center()
            .child(
                h_flex()
                    .id("window-toast")
                    .block_mouse_except_scroll()
                    .min_w_0()
                    .max_w(px(520.))
                    .items_center()
                    .gap_2()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(color.opacity(0.55))
                    .bg(cx.theme().background)
                    .shadow_md()
                    .px_3()
                    .py_2()
                    .child(Icon::new(icon).size(16.).color(color))
                    .child(
                        div()
                            .min_w_0()
                            .text_size(px(13.))
                            .child(self.message.clone()),
                    )
                    .child(
                        div()
                            .id("close-toast")
                            .flex_none()
                            .p_1()
                            .rounded(px(4.))
                            .cursor_pointer()
                            .hover(|style| style.bg(cx.theme().muted))
                            .child(
                                Icon::new("close")
                                    .size(12.)
                                    .color(cx.theme().muted_foreground),
                            )
                            .on_click(on_close),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{Duration, Instant, Toast, ToastKind};

    #[test]
    fn replacement_toast_has_its_own_deadline() {
        let now = Instant::now();
        let first = Toast::new(ToastKind::Info, "first".into(), now, Duration::from_secs(1));
        let second = Toast::new(
            ToastKind::Error,
            "second".into(),
            now,
            Duration::from_secs(3),
        );
        assert!(!first.expired(now));
        assert!(first.expired(now + Duration::from_secs(1)));
        assert!(!second.expired(now + Duration::from_secs(1)));
        assert!(second.expired(now + Duration::from_secs(3)));
    }
}
