//! User preference helpers: the interface locales the desktop ships and the
//! primary color presets the Vue settings expose.

use locale_config::Locale;
use serde_json::Value;
use std::{fs, path::PathBuf};

/// Runtime configuration key the desktop stores the workflow window size under.
pub const WORKFLOW_WINDOW_SIZE_KEY: &str = "workflow_window_size";

/// Global status-panel positions shared by every conversation.
pub const STATUS_PANEL_POSITION_KEY: &str = "workflow_status_panel_position";

/// Where the last window origin is mirrored for the next launch.
///
/// The runtime configuration only arrives with the first poll, after the window
/// has already opened, so the origin is cached locally the way Tauri's
/// window-state plugin keeps it.
fn window_state_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let dir = std::env::var_os("LOCALAPPDATA").map(|base| PathBuf::from(base).join("ChatSpeed"));
    #[cfg(target_os = "macos")]
    let dir = std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support/ChatSpeed"));
    #[cfg(all(unix, not(target_os = "macos")))]
    let dir = match std::env::var_os("XDG_STATE_HOME").filter(|value| !value.is_empty()) {
        Some(state) => Some(PathBuf::from(state).join("chatspeed-gpui")),
        None => std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join(".local/state/chatspeed-gpui")),
    };
    dir.map(|dir| dir.join("window-state.json"))
}

/// The window origin saved by the previous session, in logical pixels.
///
/// Returns `None` when the cache is missing or malformed, so the window keeps
/// the position it is created with.
pub fn saved_window_origin() -> Option<(f32, f32)> {
    let state: Value = serde_json::from_str(&fs::read_to_string(window_state_path()?).ok()?).ok()?;
    let x = state.get("x")?.as_f64()? as f32;
    let y = state.get("y")?.as_f64()? as f32;
    (x.is_finite() && y.is_finite()).then_some((x, y))
}

/// Mirrors the current window origin for the next launch. Best effort: a
/// missing or read-only cache directory only loses the remembered position.
pub fn save_window_origin(x: f32, y: f32) {
    if !(x.is_finite() && y.is_finite()) {
        return;
    }
    let Some(path) = window_state_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, serde_json::json!({"x": x, "y": y}).to_string());
}

/// Reads one saved status-panel position in logical pixels.
pub fn saved_status_panel_position(config: &Value, expanded: bool) -> Option<(f32, f32)> {
    let slot = if expanded { "expanded" } else { "collapsed" };
    let saved = config.get(STATUS_PANEL_POSITION_KEY)?.get(slot)?;
    let x = saved.get("x")?.as_f64()? as f32;
    let y = saved.get("y")?.as_f64()? as f32;
    (x.is_finite() && y.is_finite()).then_some((x, y))
}

/// Tauri saves the physical outer origin and the monitor name together.
pub fn saved_window_position(config: &Value) -> Option<(i32, i32, &str)> {
    let saved = config.get("workflow_window_position")?;
    let x = i32::try_from(saved.get("x")?.as_i64()?).ok()?;
    let y = i32::try_from(saved.get("y")?.as_i64()?).ok()?;
    let monitor = saved.get("screen_name")?.as_str()?;
    ((x != 0 || y != 0) && !monitor.trim().is_empty()).then_some((x, y, monitor))
}

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
    fn status_panel_positions_are_mode_specific() {
        let config = serde_json::json!({
            "workflow_status_panel_position": {
                "collapsed": {"x": 800.0, "y": 120.0},
                "expanded": {"x": 40.0, "y": 60.0}
            }
        });
        assert_eq!(
            saved_status_panel_position(&config, false),
            Some((800.0, 120.0))
        );
        assert_eq!(
            saved_status_panel_position(&config, true),
            Some((40.0, 60.0))
        );
        assert_eq!(saved_status_panel_position(&serde_json::json!({}), false), None);
    }

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
    fn remembered_position_requires_a_named_monitor_and_signed_coordinates() {
        let config = serde_json::json!({"workflow_window_position": {"x": -1200, "y": 8, "screen_name": "Display"}});
        assert_eq!(saved_window_position(&config), Some((-1200, 8, "Display")));
        for position in [
            serde_json::json!({"x": 0, "y": 0, "screen_name": "Display"}),
            serde_json::json!({"x": 10, "y": 20}),
            serde_json::json!({"x": 2147483648_i64, "y": 20, "screen_name": "Display"}),
        ] {
            assert_eq!(
                saved_window_position(&serde_json::json!({"workflow_window_position": position})),
                None
            );
        }
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
