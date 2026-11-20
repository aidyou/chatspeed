//! One glyph from the bundled icon font, as a reusable element.
//!
//! Vue draws every icon as `<cs name="pin" />`, i.e. a `<i class="chatspeed">`
//! with a single private-use character. This module is the native equivalent so
//! call sites pick a size and color instead of rebuilding the same div.

use gpui_kit::*;

use crate::icons;

/// A single icon-font glyph.
///
/// The glyph is drawn in a square whose edge is the requested size; callers get
/// `Icon::new("pin").size(18.).color(theme.muted_foreground)` and nothing else
/// to think about.
///
/// Two details keep the glyph optically centered the way the web window renders
/// it:
///
/// * The box is a minimum rather than a fixed size. Most glyphs advance exactly
///   one em, but 21 of the 256 glyphs advance further (the double chevrons used
///   by "load earlier" and the caret are 1.19em). With a fixed `w(size)` box
///   `TextAlign::Center` cannot center a wider line: it computes
///   `origin + (box - line) / 2`, which is *negative*, so the glyph is painted
///   left of the box and leaves the extra advance hanging on its right. A
///   minimum box grows to the advance instead, which keeps the ink centered.
/// * The row direction is explicit. `justify_center` only centers horizontally
///   in a row; relying on the default direction silently turns the horizontal
///   centering into vertical centering if that default ever changes.
#[derive(IntoElement)]
pub struct Icon {
    name: &'static str,
    size: Option<f32>,
    color: Option<Hsla>,
}

impl Icon {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            size: None,
            color: None,
        }
    }

    /// Glyph size in pixels. Defaults to the inherited text size, matching the
    /// icon font's `font-size` inheritance in CSS.
    pub fn size(mut self, size: f32) -> Self {
        self.size = Some(size);
        self
    }

    /// Glyph color. Defaults to the inherited text color.
    pub fn color(mut self, color: impl Into<Hsla>) -> Self {
        self.color = Some(color.into());
        self
    }
}

impl RenderOnce for Icon {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let inherited = window.text_style();
        let size = self
            .size
            .unwrap_or_else(|| f32::from(inherited.font_size.to_pixels(window.rem_size())));
        let color = self.color.unwrap_or(inherited.color);

        div()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .justify_center()
            .min_w(px(size))
            .min_h(px(size))
            .font_family(icons::FAMILY)
            .text_size(px(size))
            // A square line box keeps the font's own ascent and descent from
            // pushing the glyph below the text it sits next to.
            .line_height(px(size))
            .text_color(color)
            .child(icons::glyph(self.name))
    }
}
