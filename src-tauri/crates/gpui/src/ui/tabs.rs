//! Element Plus underlined tabs, shared by the native configuration dialogs.

use std::rc::Rc;

use gpui_kit::component::{h_flex, ActiveTheme};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use super::tokens::{layers, SPACE_24, SPACE_8};

/// A single-selection capsule shared by configuration forms.
///
/// The options fill the capsule edge to edge, so the gray well and its
/// segments share one height and an unselected option reads as part of the
/// capsule instead of a pill nested in a padded frame.
#[derive(IntoElement)]
pub struct CapsuleSegments {
    id: SharedString,
    options: Vec<(&'static str, String)>,
    selected: SharedString,
    height: f32,
    font_size: Option<f32>,
    on_select: Rc<dyn Fn(&'static str, &mut Window, &mut App)>,
}

impl CapsuleSegments {
    pub fn new(
        id_prefix: impl Into<SharedString>,
        options: Vec<(&'static str, String)>,
        selected: impl Into<SharedString>,
        on_select: impl Fn(&'static str, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id_prefix.into(),
            options,
            selected: selected.into(),
            // 30px of options plus the 1px hairline on each side matches the
            // 32px form fields the capsule usually sits beside.
            height: 30.,
            font_size: None,
            on_select: Rc::new(on_select),
        }
    }

    /// Height shared by the capsule and every option in it.
    pub fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    /// Type size of the option labels.
    ///
    /// The default already follows [`Self::height`], so a dense capsule gets
    /// denser text; this overrides that step when the surrounding row needs a
    /// different one.
    pub fn font_size(mut self, font_size: f32) -> Self {
        self.font_size = Some(font_size);
        self
    }

    fn resolved_font_size(&self) -> f32 {
        self.font_size
            .unwrap_or_else(|| (self.height * 0.44).clamp(10., 13.))
    }
}

impl RenderOnce for CapsuleSegments {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        use gpui_kit::component::button::{Button, ButtonRounded, ButtonVariants};
        let height = self.height;
        let font_size = self.resolved_font_size();
        let id = self.id;
        let selected = self.selected;
        let on_select = self.on_select;
        // A segmented control is a well: level 3 of the dialog ladder.
        let surface = layers(cx.theme().is_dark());
        let track = h_flex()
            .id(id.clone())
            .min_w_0()
            .items_center()
            // Wrapping keeps the capsule inside a narrow popover; each row is
            // one option tall, so a wrapped capsule stays a stadium of rows.
            .flex_wrap()
            .gap(px(2.))
            .rounded_full()
            .border_1()
            .border_color(surface.hairline)
            .bg(surface.track)
            .children(self.options.into_iter().map(|(value, label)| {
                let callback = on_select.clone();
                Button::new(SharedString::from(format!("{id}-{value}")))
                    .rounded(ButtonRounded::Size(px(height / 2.)))
                    .h(px(height))
                    .px(SPACE_8)
                    .pt(px(0.))
                    .pb(px(2.)) // Optical baseline correction, not layout spacing.
                    .line_height(px(font_size + 4.))
                    .accessibility_label(label.clone())
                    .child(div().text_size(px(font_size)).child(label))
                    .map(|button| {
                        if selected.as_ref() == value {
                            button.primary()
                        } else {
                            button.ghost()
                        }
                    })
                    .on_click(move |_, window, cx| callback(value, window, cx))
            }));
        h_flex()
            .min_w_0()
            .justify_start()
            .child(track)
            .into_any_element()
    }
}

/// The capsule at its form height, matching the 32px fields beside it.
pub fn capsule_segments(
    id_prefix: &str,
    options: Vec<(&'static str, String)>,
    selected: &str,
    on_select: impl Fn(&'static str, &mut Window, &mut App) + 'static,
) -> CapsuleSegments {
    CapsuleSegments::new(id_prefix, options, selected, on_select)
}

/// The same capsule at a caller-chosen height, for dense popovers.
///
/// The label follows the height, so a shorter capsule reads as smaller text
/// instead of keeping the form-sized type crammed inside the track.
pub fn capsule_segments_sized(
    id_prefix: &str,
    options: Vec<(&'static str, String)>,
    selected: &str,
    height: f32,
    on_select: impl Fn(&'static str, &mut Window, &mut App) + 'static,
) -> CapsuleSegments {
    CapsuleSegments::new(id_prefix, options, selected, on_select).height(height)
}

/// Renders Vue's default `el-tabs` header: bottom hairline, primary-colored
/// active label, and a 2px primary underline. The caller owns the active index
/// and the body below the header.
pub fn underline_tabs(
    id_prefix: &'static str,
    labels: Vec<String>,
    active: usize,
    on_select: impl Fn(usize, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let border = cx.theme().border;
    let primary = cx.theme().primary;
    let muted = cx.theme().muted_foreground;
    let on_select = Rc::new(on_select);
    let mut tabs = h_flex()
        .w_full()
        .min_w_0()
        .items_end()
        .gap(SPACE_24)
        .border_b_1()
        .border_color(border);
    for (index, label) in labels.into_iter().enumerate() {
        let selected = index == active;
        let callback = on_select.clone();
        tabs = tabs.child(
            div()
                .id(SharedString::from(format!("{id_prefix}-{index}")))
                .flex_none()
                .pb(px(8.))
                .cursor_pointer()
                .text_size(px(13.))
                .text_color(if selected { primary } else { muted })
                .when(selected, |tab| tab.border_b_2().border_color(primary))
                .child(label)
                .on_click(move |_, window, cx| callback(index, window, cx)),
        );
    }
    tabs.into_any_element()
}
