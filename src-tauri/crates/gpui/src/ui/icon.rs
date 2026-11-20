//! One glyph from the bundled icon font, as a reusable element.
//!
//! Vue draws every icon as `<cs name="pin" />`, i.e. a `<i class="chatspeed">`
//! with a single private-use character. This module is the native equivalent so
//! call sites pick a size and color instead of rebuilding the same div.

use std::{borrow::Cow, time::Duration};

use gpui_kit::*;

use crate::icons;

const REASONING_BULB: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9 18h6m-5 3h4M9 15c0-2-3-3-3-7a6 6 0 0 1 12 0c0 4-3 5-3 7z"/></svg>"#;

pub(crate) struct IconAssets;

impl AssetSource for IconAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == "icons/reasoning-lightbulb.svg" {
            Ok(Some(Cow::Borrowed(REASONING_BULB)))
        } else {
            gpui_kit::assets::Assets.load(path)
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        if "icons/reasoning-lightbulb.svg".starts_with(path) {
            paths.push("icons/reasoning-lightbulb.svg".into());
        }
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::IconAssets;
    use gpui_kit::AssetSource;

    #[test]
    fn animated_icons_are_available_with_the_default_assets() {
        for path in ["icons/reasoning-lightbulb.svg", "icons/loader-circle.svg"] {
            let svg = IconAssets.load(path).unwrap().unwrap();
            assert!(svg.starts_with(b"<svg"), "{path}");
        }
    }
}

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
    spin: Option<ElementId>,
}

impl Icon {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            size: None,
            color: None,
            spin: None,
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

    /// Animate an active indicator using its caller's stable element ID.
    pub fn spin(mut self, id: impl Into<ElementId>) -> Self {
        self.spin = Some(id.into());
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
        // GPUI transforms SVGs, but not text glyphs. Static icons still use the font.
        let animated_path = match self.name {
            "reasoning" => Some("icons/reasoning-lightbulb.svg"),
            "loading" => Some("icons/loader-circle.svg"),
            _ => None,
        };
        if let (Some(id), Some(path)) = (self.spin, animated_path) {
            return svg()
                .path(path)
                .size(px(size))
                .flex_none()
                .text_color(color)
                .with_animation(
                    id,
                    Animation::new(Duration::from_millis(1200))
                        .repeat_synced()
                        .with_max_fps(30.),
                    |icon, phase| {
                        icon.with_transformation(Transformation::rotate(percentage(phase)))
                    },
                )
                .into_any_element();
        }

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
            .into_any_element()
    }
}
