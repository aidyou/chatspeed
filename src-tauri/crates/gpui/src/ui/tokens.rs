//! Shared native UI tokens. Keep layout values independent of typography and hairlines.

use gpui_kit::{component::ActiveTheme, px, rgb, white, App, Hsla, Pixels};

pub const SPACE_4: Pixels = px(4.);
pub const SPACE_8: Pixels = px(8.);
pub const SPACE_12: Pixels = px(12.);
pub const SPACE_16: Pixels = px(16.);
pub const SPACE_24: Pixels = px(24.);
pub const RADIUS_SMALL: Pixels = px(4.);
pub const RADIUS_MEDIUM: Pixels = px(8.);

/// Native fields stay white in light mode; dark mode follows the web input token.
pub fn field_bg(cx: &App) -> Hsla {
    if cx.theme().is_dark() {
        rgb(0x282b2d).into()
    } else {
        white()
    }
}

/// Native chrome colors mirror the light and dark `--cs-*` web tokens.
pub struct Surfaces {
    pub deep: Hsla,
    pub titlebar: Hsla,
    pub titlebar_border: Hsla,
    pub hover: Hsla,
    pub active: Hsla,
}

pub fn surfaces(dark: bool) -> Surfaces {
    let [deep, titlebar, titlebar_border, hover, active] = if dark {
        [0x232629, 0x21221e, 0x111111, 0x2e2f2c, 0x2f3234]
    } else {
        [0xe7e7e6, 0xfcfcfc, 0xe2e2e0, 0xefefef, 0xd2d3d1]
    };
    Surfaces {
        deep: rgb(deep).into(),
        titlebar: rgb(titlebar).into(),
        titlebar_border: rgb(titlebar_border).into(),
        hover: rgb(hover).into(),
        active: rgb(active).into(),
    }
}

/// Surfaces stacked inside a dialog, mirroring the web `--cs-bg-*` steps.
///
/// A dialog is level 1 and keeps the container's own background. `card` is
/// level 2, for a group of controls inside it, and `track` is level 3, for a
/// well inside that card such as a segmented control.
///
/// The steps are deliberately small — roughly 5 L* each — so the dialog stays
/// calm, but they must stay larger than the old ad-hoc mix they replace: a
/// `muted` wash on top of a `muted` track left the reasoning capsule nearly
/// indistinguishable from the model card it sat in (OKLab ΔE ≈ 0.03, about the
/// noise floor of an 8-bit neutral). Every level also carries `hairline`, so an
/// edge never depends on the fill step alone.
#[derive(Clone, Copy)]
pub struct Layers {
    pub card: Hsla,
    pub track: Hsla,
    pub hairline: Hsla,
}

pub fn layers(dark: bool) -> Layers {
    let [card, track, hairline] = if dark {
        [0x222325, 0x2e3134, 0x34383b]
    } else {
        [0xeaeaea, 0xdcdcdb, 0xd4d4d3]
    };
    Layers {
        card: rgb(card).into(),
        track: rgb(track).into(),
        hairline: rgb(hairline).into(),
    }
}
