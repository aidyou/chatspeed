//! One glyph from the bundled icon font, as a reusable element.
//!
//! Vue draws every icon as `<cs name="pin" />`, i.e. a `<i class="chatspeed">`
//! with a single private-use character. This module is the native equivalent so
//! call sites pick a size and color instead of rebuilding the same div.

use std::{borrow::Cow, time::Duration};

use gpui_kit::*;

use crate::icons;

const REASONING_BULB: &[u8] = include_bytes!("../../assets/vue-reasoning.svg");
const LOADING: &[u8] = include_bytes!("../../assets/vue-loading.svg");
const PIN: &[u8] = include_bytes!("../../assets/vue-pin.svg");

pub(crate) struct IconAssets;

impl AssetSource for IconAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match path {
            "icons/reasoning-lightbulb.svg" => Ok(Some(Cow::Borrowed(REASONING_BULB))),
            "icons/vue-loading.svg" => Ok(Some(Cow::Borrowed(LOADING))),
            "icons/chatspeed-pin.svg" => Ok(Some(Cow::Borrowed(PIN))),
            "icons/disclosure.svg" => Ok(Some(Cow::Borrowed(br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M6 4l4 4-4 4" fill="none" stroke="black" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>"#))),
            "icons/disclosure-double.svg" => Ok(Some(Cow::Borrowed(br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M4 4l4 4 4-4M4 8l4 4 4-4" fill="none" stroke="black" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>"#))),
            _ => gpui_kit::assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        for asset in ["icons/reasoning-lightbulb.svg", "icons/vue-loading.svg", "icons/chatspeed-pin.svg"] {
            if asset.starts_with(path) {
                paths.push(asset.into());
            }
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
        for path in ["icons/reasoning-lightbulb.svg", "icons/vue-loading.svg", "icons/disclosure.svg", "icons/disclosure-double.svg"] {
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
    spin: Option<(ElementId, Spin)>,
    turns: f32,
}

/// Rotation curves shared by active indicators.
///
/// `Accelerate` starts slow and speeds up, which reads as work in progress;
/// `Linear` is the steady spin used by the thinking bulb.
#[derive(Clone, Copy)]
pub(crate) enum Spin {
    Linear,
    Accelerate,
    Eased,
}

impl Spin {
    fn phase(self, phase: f32) -> f32 {
        match self {
            Self::Linear => phase,
            // CSS `ease-in`: accelerating, for the running indicator.
            Self::Accelerate => bezier_phase(phase, 0.42, 0., 1., 1.),
            // CSS `ease` (Vue's `.cs-spin`).
            Self::Eased => bezier_phase(phase, 0.25, 0.1, 0.25, 1.),
        }
    }
}

/// Solves a cubic-bezier timing function for one normalized `phase`.
fn bezier_phase(phase: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    let sample = |t: f32, a: f32, b: f32| {
        3. * (1. - t).powi(2) * t * a + 3. * (1. - t) * t * t * b + t.powi(3)
    };
    let (mut low, mut high) = (0., 1.);
    for _ in 0..12 {
        let mid = (low + high) / 2.;
        if sample(mid, x1, x2) < phase {
            low = mid;
        } else {
            high = mid;
        }
    }
    sample((low + high) / 2., y1, y2)
}

impl Icon {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            size: None,
            color: None,
            spin: None,
            turns: 0.,
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

    pub(crate) fn rotate(mut self, degrees: f32) -> Self {
        self.turns = degrees / 360.;
        self
    }

    /// Animate an active indicator using its caller's stable element ID.
    pub(crate) fn spin(mut self, id: impl Into<ElementId>, curve: Spin) -> Self {
        self.spin = Some((id.into(), curve));
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
        // GPUI only transforms SVG sprites; the rotating sprites are exported
        // from the same font used for the static glyphs.
        let animated_path = match self.name {
            "reasoning" if self.spin.is_some() => Some("icons/reasoning-lightbulb.svg"),
            "loading" if self.spin.is_some() => Some("icons/vue-loading.svg"),
            "caret-right" => Some("icons/disclosure.svg"),
            "double-arrow-down" => Some("icons/disclosure-double.svg"),
            "pin" if self.turns != 0. => Some("icons/chatspeed-pin.svg"),
            _ => None,
        };
        if let Some(path) = animated_path {
            let icon = svg()
                .path(path)
                .size(px(size))
                .flex_none()
                .text_color(color);
            if let Some((id, curve)) = self.spin {
                let seconds = match curve {
                    Spin::Linear => 2,
                    Spin::Accelerate | Spin::Eased => 1,
                };
                return icon
                    .with_animation(
                        id,
                        Animation::new(Duration::from_secs(seconds)).repeat(),
                        move |icon, phase| {
                            icon.with_transformation(Transformation::rotate(percentage(
                                curve.phase(phase),
                            )))
                        },
                    )
                    .into_any_element();
            }
            return icon
                .with_transformation(Transformation::rotate(percentage(self.turns)))
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
