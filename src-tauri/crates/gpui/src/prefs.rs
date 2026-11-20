//! User preference helpers: the interface locales the desktop ships and the
//! primary color presets the Vue settings expose.

use locale_config::Locale;
use serde_json::Value;

/// Runtime configuration key the desktop stores the workflow window size under.
pub const WORKFLOW_WINDOW_SIZE_KEY: &str = "workflow_window_size";

/// ChatSpeed's default primary preset (`green`).
pub const DEFAULT_PRIMARY: u32 = 0x41b555;

/// The Vue primary presets from `src/style/element/css-vars.css`.
pub fn primary_preset(name: &str) -> u32 {
    match name {
        "purple" => 0xaf52de,
        "yellow" => 0xf7831c,
        "pink" => 0xff2d55,
        "blue" => 0x007aff,
        _ => DEFAULT_PRIMARY,
    }
}

/// Reads the remembered workflow window size in logical pixels.
///
/// Returns `None` for a missing, malformed or non-positive value, so the window
/// keeps the geometry it was created with rather than collapsing.
pub fn saved_window_size(config: &Value) -> Option<(f32, f32)> {
    let saved = config.get(WORKFLOW_WINDOW_SIZE_KEY)?;
    let width = saved.get("width")?.as_f64()? as f32;
    let height = saved.get("height")?.as_f64()? as f32;
    (width.is_finite() && height.is_finite() && width > 0. && height > 0.)
        .then_some((width, height))
}

/// Maps a stored or system locale onto the three interface locales.
pub fn normalize_locale(locale: &str) -> &'static str {
    match locale {
        "zh-Hans" | "zh-CN" | "zh-SG" => "zh-Hans",
        "zh-Hant" | "zh-TW" | "zh-HK" | "zh-MO" => "zh-Hant",
        _ => "en",
    }
}

/// The locale to use before the runtime configuration arrives.
pub fn system_locale() -> &'static str {
    normalize_locale(Locale::current().as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remembered_window_size_requires_a_usable_pair() {
        let config =
            serde_json::json!({"workflow_window_size": {"width": 1400.0, "height": 900.0}});
        assert_eq!(saved_window_size(&config), Some((1400.0, 900.0)));
        assert_eq!(saved_window_size(&serde_json::json!({})), None);
        assert_eq!(
            saved_window_size(
                &serde_json::json!({"workflow_window_size": {"width": 0.0, "height": 700.0}})
            ),
            None
        );
        assert_eq!(
            saved_window_size(&serde_json::json!({"workflow_window_size": "wide"})),
            None
        );
    }

    #[test]
    fn interface_locales_map_to_the_shipped_three() {
        assert_eq!(normalize_locale("zh-CN"), "zh-Hans");
        assert_eq!(normalize_locale("zh-Hans"), "zh-Hans");
        assert_eq!(normalize_locale("zh-TW"), "zh-Hant");
        assert_eq!(normalize_locale("zh-Hant"), "zh-Hant");
        assert_eq!(normalize_locale("de"), "en");
        assert_eq!(normalize_locale(""), "en");
    }

    #[test]
    fn primary_presets_match_the_vue_palette() {
        assert_eq!(primary_preset("green"), 0x41b555);
        assert_eq!(primary_preset("purple"), 0xaf52de);
        assert_eq!(primary_preset("yellow"), 0xf7831c);
        assert_eq!(primary_preset("pink"), 0xff2d55);
        assert_eq!(primary_preset("blue"), 0x007aff);
        assert_eq!(primary_preset("unknown"), DEFAULT_PRIMARY);
    }
}
