//! Height-constrained footer popovers that grow to fit short lists.

use std::{cell::Cell, rc::Rc};

use gpui_kit::base::{ElementExt as _, InteractiveElementExt as _, ScrollBounce};
use gpui_kit::component::{
    popover::{Popover, PopoverState},
    scroll::{ScrollableElement as _, Scrollbar},
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use super::trigger;

const GAP: Pixels = px(4.);
const EDGE: Pixels = px(8.);

fn list_height(top: Pixels, inset: Pixels, rem: Pixels) -> Pixels {
    // The styled popover adds p-3 on both ends; the positioner only moves it.
    (top - inset - EDGE - GAP - rem * 1.5).max(px(0.))
}

fn reveal_offset(
    viewport: Bounds<Pixels>,
    row: Bounds<Pixels>,
    offset: Pixels,
    max_offset: Pixels,
) -> Pixels {
    let top = row.top() + offset;
    let bottom = row.bottom() + offset;
    let next = if top < viewport.top() || row.size.height > viewport.size.height {
        offset + viewport.top() - top
    } else if bottom > viewport.bottom() {
        offset + viewport.bottom() - bottom
    } else {
        offset
    };
    next.clamp(-max_offset, px(0.))
}

#[derive(IntoElement)]
pub struct MenuPopover {
    id: &'static str,
    button: AnyElement,
    width: Option<Pixels>,
    // Outer None denotes a menu without selectable rows.
    selection: Option<Option<usize>>,
    content: Box<MenuContent>,
}

type MenuContent =
    dyn Fn(&mut PopoverState, &mut Window, &mut Context<PopoverState>) -> Stateful<Div>;

impl MenuPopover {
    pub fn new(
        id: &'static str,
        button: AnyElement,
        content: impl Fn(&mut PopoverState, &mut Window, &mut Context<PopoverState>) -> Stateful<Div>
            + 'static,
    ) -> Self {
        Self {
            id,
            button,
            width: None,
            selection: None,
            content: Box::new(content),
        }
    }

    /// Sets the outer popover width, including its built-in padding.
    pub fn width(mut self, width: Pixels) -> Self {
        self.width = Some(width);
        self
    }

    /// Reveals the selected direct child when the popover opens, including
    /// any headings that precede it in the index.
    pub fn selected_index(mut self, index: Option<usize>) -> Self {
        self.selection = Some(index);
        self
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
        let scroll = self.selection.map(|_| {
            window
                .use_keyed_state(
                    SharedString::from(format!("{id}-selected-scroll")),
                    cx,
                    |_, _| (ScrollHandle::new(), Rc::new(Cell::new(false))),
                )
                .read(cx)
                .clone()
        });
        let open_scroll = scroll.clone();
        Popover::new(id)
            .on_open_change(move |open, _, _| {
                if let Some((_, pending)) = &open_scroll {
                    pending.set(*open);
                }
            })
            .when_some(self.width, |popover, width| popover.w(width))
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
                let list = build(state, window, cx).max_h(limit);
                if let Some((scroll, pending)) = &scroll {
                    let selected = self.selection.flatten();
                    let measure_scroll = scroll.clone();
                    let pending = pending.clone();
                    div()
                        .relative()
                        .child(ScrollBounce::new(
                            "selected-menu-bounce",
                            scroll,
                            list.overflow_y_scroll()
                                .lock_scroll_axis()
                                .track_scroll(scroll),
                        ))
                        .on_prepaint(move |_, window, _| {
                            // GPUI's indexed scroll runs before the current viewport is
                            // recorded. Read it after the scroll area has been laid out.
                            if !pending.replace(false) {
                                return;
                            }
                            let Some(row) =
                                selected.and_then(|index| measure_scroll.bounds_for_item(index))
                            else {
                                return;
                            };
                            let mut offset = measure_scroll.offset();
                            let next = reveal_offset(
                                measure_scroll.bounds(),
                                row,
                                offset.y,
                                measure_scroll.max_offset().y,
                            );
                            if next != offset.y {
                                offset.y = next;
                                measure_scroll.set_offset(offset);
                                window.on_next_frame(|window, _| window.refresh());
                            }
                        })
                        .child(
                            div().absolute().inset_0().child(
                                Scrollbar::vertical(scroll)
                                    .id("selected-menu-scrollbar")
                                    .viewport_from_layout(),
                            ),
                        )
                        .into_any_element()
                } else {
                    list.overflow_y_scrollbar()
                        .id(SharedString::from(format!("{id}-menu-scroll")))
                        .into_any_element()
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{list_height, point, px, reveal_offset, size, Bounds};

    #[test]
    fn reveal_scrolls_a_bottom_row_into_view_without_moving_a_visible_row() {
        let viewport = Bounds::new(point(px(0.), px(100.)), size(px(240.), px(96.)));
        let bottom_row = Bounds::new(point(px(0.), px(460.)), size(px(240.), px(24.)));
        let max_offset = px(288.);
        assert_eq!(
            reveal_offset(viewport, bottom_row, px(0.), max_offset),
            px(-288.)
        );
        assert_eq!(
            reveal_offset(viewport, bottom_row, px(-288.), max_offset),
            px(-288.)
        );

        let earlier_row = Bounds::new(point(px(0.), px(192.)), size(px(240.), px(24.)));
        assert_eq!(
            reveal_offset(viewport, earlier_row, px(-288.), max_offset),
            px(-92.)
        );
        assert_eq!(
            reveal_offset(viewport, earlier_row, px(-48.), max_offset),
            px(-48.)
        );
    }

    #[test]
    fn menu_budget_tracks_trigger_and_accounts_for_chrome() {
        assert_eq!(list_height(px(550.), px(0.), px(16.)), px(514.));
        assert_eq!(list_height(px(220.), px(12.), px(20.)), px(166.));
        assert_eq!(list_height(px(20.), px(12.), px(16.)), px(0.));
    }
}
