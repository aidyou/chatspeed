//! Overlay scrollbar that distinguishes thumb and track input from programmatic scrolling.

use std::{cell::Cell, rc::Rc};

use gpui_kit::component::scroll::{Scrollbar, ScrollbarHandle, ScrollbarMode};
use gpui_kit::*;

#[derive(Clone, Default)]
pub(crate) struct ScrollbarInput(Rc<Cell<bool>>);

impl ScrollbarInput {
    pub(crate) fn take(&self) -> bool {
        self.0.replace(false)
    }

    pub(crate) fn overlay(&self, handle: &ScrollHandle) -> impl IntoElement {
        div().absolute().inset_0().child(
            Scrollbar::vertical(&UserScrollHandle {
                handle: handle.clone(),
                input: self.clone(),
            })
            .id("history-scrollbar")
            .mode(ScrollbarMode::Scrolling)
            .viewport_from_layout(),
        )
    }
}

#[derive(Clone)]
struct UserScrollHandle {
    handle: ScrollHandle,
    input: ScrollbarInput,
}

impl ScrollbarHandle for UserScrollHandle {
    fn viewport_bounds(&self) -> Bounds<Pixels> {
        self.handle.bounds()
    }

    fn offset(&self) -> Point<Pixels> {
        self.handle.offset()
    }

    fn set_offset(&self, offset: Point<Pixels>) {
        let previous = self.handle.offset();
        self.handle.set_offset(offset);
        if self.handle.offset() != previous {
            self.input.0.set(true);
        }
    }

    fn content_size(&self) -> Size<Pixels> {
        ScrollbarHandle::content_size(&self.handle)
    }
}

#[cfg(test)]
mod tests {
    use super::{ScrollbarHandle, ScrollbarInput, UserScrollHandle};
    use gpui_kit::{point, px, ScrollHandle};

    #[test]
    fn only_scrollbar_input_signals_the_reader() {
        let handle = ScrollHandle::new();
        let input = ScrollbarInput::default();
        let scrollbar = UserScrollHandle {
            handle: handle.clone(),
            input: input.clone(),
        };
        handle.set_offset(point(px(0.), px(-20.)));
        assert!(!input.take());
        scrollbar.set_offset(point(px(0.), px(-30.)));
        assert!(input.take());
        assert!(!input.take());
        scrollbar.set_offset(point(px(0.), px(-30.)));
        assert!(!input.take());
    }
}
