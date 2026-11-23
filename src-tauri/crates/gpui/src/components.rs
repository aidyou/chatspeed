//! Shared ChatSpeed controls. Pages own values and business actions; controls own presentation.

pub mod button {
    pub use gpui_kit::component::button::{Button, ButtonRounded, ButtonVariants};
}
pub mod input {
    pub use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};

    use gpui_kit::{App, Entity, Styled};

    /// Standard form field; callers can still customize size and adornments.
    pub fn field(state: &Entity<InputState>, cx: &App) -> Input {
        Input::new(state).bg(super::tokens::field_bg(cx))
    }
}
pub mod slider {
    pub use gpui_kit::component::slider::{Slider, SliderEvent, SliderState, SliderValue};
}
pub mod switch {
    pub use gpui_kit::component::switch::Switch;
}

#[path = "ui/tokens.rs"]
pub mod tokens;
#[path = "ui/tabs.rs"]
pub mod tabs;
#[path = "ui/menu_popover.rs"]
pub mod select;

pub mod dialog {
    pub use gpui_kit::component::{dialog::*, WindowExt};
}

#[path = "ui/confirm.rs"]
pub mod confirm;
#[path = "ui/icon.rs"]
pub mod icon;
#[path = "ui/toast.rs"]
pub mod toast;

pub mod tooltip {
    use gpui_kit::*;
    pub use gpui_kit::component::tooltip::Tooltip;

    pub const TOOLTIP_DELAY: std::time::Duration = std::time::Duration::from_millis(200);

    /// Compact Element Plus-style hover hint.
    pub fn tip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
        let text = text.into();
        move |window, cx| compact(Tooltip::new(text.clone())).build(window, cx)
    }

    pub fn compact(tooltip: Tooltip) -> Tooltip {
        tooltip.text_size(px(12.))
            .line_height(px(18.))
            .px(px(8.))
            .py(px(4.))
            .rounded(px(4.))
            .border_color(rgb(0x303133))
            .bg(rgb(0x303133))
            .text_color(white())
    }
}

pub mod skeleton {
    use gpui_kit::component::ActiveTheme as _;
    use gpui_kit::{
        div, px, relative, Animation, AnimationExt, AnyElement, App, Div, ElementId, IntoElement,
        Styled,
    };

    /// Animate a group together without changing its layout or delaying content.
    pub fn pulse(id: impl Into<ElementId>, content: Div, cx: &App) -> AnyElement {
        if cx.reduce_motion() {
            return content.into_any_element();
        }
        content
            .with_animation(
                id,
                Animation::new(std::time::Duration::from_millis(1200))
                    .repeat_synced()
                    .with_max_fps(30.),
                |content, phase| {
                    let wave = (phase * std::f32::consts::TAU).cos();
                    content.opacity(0.78 + 0.22 * wave)
                },
            )
            .into_any_element()
    }

    /// A placeholder block sized relative to its container and capped in pixels.
    pub fn block(width: f32, max_width: f32, height: f32, radius: f32, cx: &App) -> Div {
        div()
            .w(relative(width))
            .max_w(px(max_width))
            .h(px(height))
            .rounded(px(radius))
            .bg(cx.theme().muted_foreground.opacity(0.14))
    }
}

use gpui_kit::{AnyElement, IntoElement};

/// An unstyled trigger suitable for popovers in custom chrome.
pub struct Trigger {
    element: AnyElement,
    open: bool,
}

impl gpui_kit::component::Selectable for Trigger {
    fn selected(mut self, selected: bool) -> Self {
        self.open = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.open
    }
}

impl IntoElement for Trigger {
    type Element = AnyElement;

    fn into_element(self) -> Self::Element {
        self.element
    }
}

pub fn trigger(element: impl IntoElement) -> Trigger {
    Trigger { element: element.into_any_element(), open: false }
}
