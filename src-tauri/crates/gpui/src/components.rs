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
