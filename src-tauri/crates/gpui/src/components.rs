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
    use gpui_kit::component::ActiveTheme as _;
    use gpui_kit::*;
    pub use gpui_kit::component::tooltip::Tooltip;

    pub const TOOLTIP_DELAY: std::time::Duration = std::time::Duration::from_millis(200);

    /// The tooltip surface and its text color.
    ///
    /// Element Plus hints are inverted against the page: dark on a light page,
    /// light on a dark one, so a hint never blends into the surface behind it.
    pub fn surface(dark: bool) -> (Hsla, Hsla) {
        if dark {
            (rgb(0xf2f2f3).into(), rgb(0x303133).into())
        } else {
            (rgb(0x303133).into(), white())
        }
    }

    /// Secondary text on the tooltip surface, for metadata rows.
    pub fn dim(dark: bool) -> Hsla {
        if dark {
            rgb(0x606266).into()
        } else {
            white().opacity(0.72)
        }
    }

    /// Lifts the surface off the page behind it.
    ///
    /// The light surface sits on a dark page and needs a deeper, wider shadow
    /// than Element Plus's subtle one, or it reads as a flat rectangle with no
    /// edge. The dark surface gets a shadow of its own for the same reason.
    pub fn shadow(dark: bool) -> Vec<BoxShadow> {
        let (offset, blur, alpha) = if dark {
            (6., 24., 0.42)
        } else {
            (8., 24., 0.32)
        };
        vec![BoxShadow {
            offset: point(px(0.), px(offset)),
            blur_radius: px(blur),
            spread_radius: px(0.),
            inset: false,
            color: black().opacity(alpha),
        }]
    }

    /// Compact Element Plus-style hover hint.
    pub fn tip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
        let text = text.into();
        move |window, cx| compact(Tooltip::new(text.clone()), cx).build(window, cx)
    }

    pub fn compact(tooltip: Tooltip, cx: &App) -> Tooltip {
        let dark = cx.theme().is_dark();
        let (background, foreground) = surface(dark);
        tooltip
            .text_size(px(12.))
            .line_height(px(18.))
            .px(px(8.))
            .py(px(4.))
            .rounded(px(4.))
            .border_color(background)
            .bg(background)
            .text_color(foreground)
            .shadow(shadow(dark))
    }
}

pub mod skeleton {
    use gpui_kit::component::ActiveTheme as _;
    use gpui_kit::{
        div, px, relative, AnyElement, App, Div, IntoElement, Styled, Window,
    };

    /// Breathe a group together without changing its layout or delaying content.
    ///
    /// Driven by the same shared clock as the spinners rather than by an
    /// `AnimationElement`: the latter holds every repeating animation at phase
    /// zero under the desktop's reduced-motion preference, which left this
    /// skeleton completely static. A request frame keeps it repainting until it
    /// leaves the tree.
    pub fn pulse(content: Div, window: &mut Window) -> AnyElement {
        window.request_animation_frame();
        let phase = super::icon::spin_phase(std::time::Duration::from_millis(1200));
        let wave = (phase * std::f32::consts::TAU).cos();
        content.opacity(0.78 + 0.22 * wave).into_any_element()
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
