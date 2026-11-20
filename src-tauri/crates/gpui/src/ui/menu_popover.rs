//! Height-constrained footer popovers that grow to fit short lists.

use std::{cell::Cell, rc::Rc};

use gpui_kit::base::ElementExt as _;
use gpui_kit::component::{
    popover::{Popover, PopoverState},
    scroll::ScrollableElement as _,
};
use gpui_kit::*;

use super::trigger;

const GAP: Pixels = px(4.);
const EDGE: Pixels = px(8.);

fn list_height(top: Pixels, inset: Pixels, rem: Pixels) -> Pixels {
    // The styled popover adds p-3 on both ends; the positioner only moves it.
    (top - inset - EDGE - GAP - rem * 1.5).max(px(0.))
}

#[derive(IntoElement)]
pub(super) struct MenuPopover {
    id: &'static str,
    button: AnyElement,
    content: Box<MenuContent>,
}

type MenuContent =
    dyn Fn(&mut PopoverState, &mut Window, &mut Context<PopoverState>) -> Stateful<Div>;

impl MenuPopover {
    pub(super) fn new(
        id: &'static str,
        button: AnyElement,
        content: impl Fn(&mut PopoverState, &mut Window, &mut Context<PopoverState>) -> Stateful<Div>
            + 'static,
    ) -> Self {
        Self {
            id,
            button,
            content: Box::new(content),
        }
    }
}

impl RenderOnce for MenuPopover {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let top = window
            .use_keyed_state(
                SharedString::from(format!("{}-trigger-top", self.id)),
                cx,
                |_, _| Rc::new(Cell::new(None)),
            )
            .read(cx)
            .clone();
        let measured = top.clone();
        let button = div().relative().flex_none().child(self.button).on_prepaint(
            move |bounds, window, _| {
                let next = bounds.top();
                if measured
                    .get()
                    .is_none_or(|old: Pixels| (old - next).abs() >= px(1.))
                {
                    measured.set(Some(next));
                    window.on_next_frame(|window, _| window.refresh());
                }
            },
        );
        let build = self.content;
        let id = self.id;
        Popover::new(id)
            .anchor(Anchor::BottomLeft)
            .offset(GAP)
            .trigger(trigger(button))
            .content(move |state, window, cx| {
                let viewport = window.viewport_size().height;
                let anchor = top.get().unwrap_or(viewport * 0.5).min(viewport);
                let inset = match window.window_decorations() {
                    Decorations::Client { tiling } if !tiling.top => {
                        window.client_inset().unwrap_or(px(0.))
                    }
                    _ => px(0.),
                };
                let limit = list_height(anchor, inset, window.rem_size());
                build(state, window, cx)
                    .max_h(limit)
                    .overflow_y_scrollbar()
                    .id(SharedString::from(format!("{id}-menu-scroll")))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{list_height, px};

    #[test]
    fn menu_budget_tracks_trigger_and_accounts_for_chrome() {
        assert_eq!(list_height(px(550.), px(0.), px(16.)), px(514.));
        assert_eq!(list_height(px(220.), px(12.), px(20.)), px(166.));
        assert_eq!(list_height(px(20.), px(12.), px(16.)), px(0.));
    }
}
