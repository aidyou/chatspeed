//! Shared native UI tokens. Keep layout values independent of typography and hairlines.

use gpui_kit::{component::ActiveTheme, px, rgb, white, App, Hsla, Pixels};

pub(crate) const SPACE_4: Pixels = px(4.);
pub(crate) const SPACE_8: Pixels = px(8.);
pub(crate) const SPACE_12: Pixels = px(12.);
pub(crate) const SPACE_16: Pixels = px(16.);
pub(crate) const SPACE_24: Pixels = px(24.);
pub(crate) const RADIUS_SMALL: Pixels = px(4.);
pub(crate) const RADIUS_MEDIUM: Pixels = px(8.);

/// Native fields stay white in light mode; dark mode follows the web input token.
pub(crate) fn field_bg(cx: &App) -> Hsla {
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
