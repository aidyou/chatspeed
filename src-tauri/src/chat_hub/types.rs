//! Values the ChatHub page carriers share.

use serde::Serialize;

/// Label of the window the page is docked to.
pub const CHAT_HUB_HOST_WINDOW_LABEL: &str = "workflow";

/// Width the docked page opens with, in logical pixels.
///
/// The page opens at a phone width, so a chat site starts in the layout it is designed
/// around; the same value is the narrowest width a carrier accepts (see
/// [`CHAT_HUB_MIN_WIDTH`]).
pub const CHAT_HUB_DEFAULT_WIDTH: f64 = CHAT_HUB_MIN_WIDTH;

/// Narrowest page width, in logical pixels.
///
/// 375 is the logical viewport width of a common phone, so a site still renders its
/// mobile layout when the splitter is dragged all the way in.
pub const CHAT_HUB_MIN_WIDTH: f64 = 375.0;

/// Width the workflow UI always keeps next to the page, in logical pixels.
pub const CHAT_HUB_MIN_HOST_WIDTH: f64 = 480.0;

/// Width limits of the docked page, reported to the frontend.
///
/// The splitter has to clamp a drag exactly like the carrier does, so the limits are
/// owned here instead of being duplicated in the frontend.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatHubPageLimits {
    /// Narrowest page width the carrier accepts.
    pub min_width: f64,
    /// Width the workflow UI always keeps next to the page.
    pub min_host_width: f64,
}

impl ChatHubPageLimits {
    /// The limits every carrier enforces.
    pub fn current() -> Self {
        Self {
            min_width: CHAT_HUB_MIN_WIDTH,
            min_host_width: CHAT_HUB_MIN_HOST_WIDTH,
        }
    }
}
