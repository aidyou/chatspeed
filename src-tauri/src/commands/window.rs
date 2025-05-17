//! Window management commands for the application
//!
//! This module implements commands for managing application windows, including
//! creating and focusing note, setting, and URL windows.
//!
//! # Window Creation Strategy
//!
//! Instead of creating windows directly in command handlers, we use an event-based
//! approach where commands emit events that are handled by the main thread. This
//! design choice addresses a critical issue in WebView2 (Windows) where creating
//! windows from IPC handlers can lead to deadlocks.
//!
//! ## Background
//!
//! The issue stems from how WebView2 handles window creation and IPC calls on Windows:
//! - WebView2 requires window creation to happen on the main UI thread
//! - IPC handlers run on a different thread
//! - Attempting to create windows directly from IPC handlers can cause thread
//!   synchronization issues and deadlocks
//!
//! For more details, see the discussion at:
//! <https://github.com/tauri-apps/wry/issues/583>
//!
//! ## Implementation
//!
//! Our solution uses Tauri's event system:
//! 1. Commands emit events (e.g., "create-note-window", "create-setting-window")
//! 2. Event listeners on the main thread handle window creation
//! 3. This ensures windows are always created on the correct thread
//!
//! This approach provides several benefits:
//! - Prevents deadlocks on Windows
//! - Works consistently across all platforms
//! - Maintains clean separation of concerns
//! - Improves code maintainability

use std::collections::HashMap;
use std::sync::atomic::Ordering;

use rust_i18n::t;
use serde_json::Value;
// use tauri::utils::{config::WindowEffectsConfig, WindowEffect};
use crate::constants::{ASSISTANT_ALWAYS_ON_TOP, MAIN_WINDOW_ALWAYS_ON_TOP};
use tauri::{command, Emitter, Manager}; //window::Color, WindowEvent,

#[derive(serde::Serialize, Clone)]
struct SettingWindowPayload {
    setting_type: String,
}

/// Opens the settings window via event system
///
/// IMPORTANT: We use events instead of direct window creation to avoid deadlocks
/// on Windows. This is because WebView2 requires window creation to happen on the
/// main UI thread, while IPC handlers run on a different thread. Using events
/// ensures window creation occurs on the correct thread.
///
/// See: https://github.com/tauri-apps/wry/issues/583
///
/// # Arguments
/// - `app_handle` - Tauri application handle
/// - `setting_type` - Optional setting type to display (defaults to "general")
///
/// # Example
/// ```js
/// import { invoke } from '@tauri-apps/api/core'
///
/// await invoke('open_setting_window', { settingType: 'model' });
/// ```
#[command]
pub async fn open_setting_window(
    app_handle: tauri::AppHandle,
    setting_type: Option<String>,
) -> Result<(), String> {
    // Get the main window to emit the event
    let main_window = app_handle
        .get_webview_window("main")
        .ok_or_else(|| t!("main.window_not_ready"))?;

    // Emit an event to create the window on the main thread
    // use events instead of direct window creation to avoid deadlocks on Windows
    main_window
        .emit(
            "create-setting-window",
            SettingWindowPayload {
                setting_type: setting_type.unwrap_or_else(|| "general".to_string()),
            },
        )
        .map_err(|e| t!("main.failed_to_emit_event", error = e))?;

    Ok(())
}

/// Opens the note window via event system
///
/// IMPORTANT: We use events instead of direct window creation to avoid deadlocks
/// on Windows. This is because WebView2 requires window creation to happen on the
/// main UI thread, while IPC handlers run on a different thread. Using events
/// ensures window creation occurs on the correct thread.
///
/// See: https://github.com/tauri-apps/wry/issues/583
///
/// # Arguments
/// - `app_handle` - Tauri application handle
///
/// # Example
/// ```js
/// import { invoke } from '@tauri-apps/api/core'
///
/// await invoke('open_note_window');
/// ```
#[command]
pub async fn open_note_window(app_handle: tauri::AppHandle) -> Result<(), String> {
    // Get the main window to emit the event
    let main_window = app_handle
        .get_webview_window("main")
        .ok_or_else(|| t!("main.window_not_ready"))?;

    // Emit an event to create the window on the main thread
    // use events instead of direct window creation to avoid deadlocks on Windows
    main_window
        .emit("create-note-window", ())
        .map_err(|e| t!("main.failed_to_emit_event", error = e))?;

    Ok(())
}

/// Show the window by label
///
/// # Arguments
/// - `app_handle` - Tauri application handle
///
/// # Example
/// ```js
/// import { invoke } from '@tauri-apps/api/core'
///
/// await invoke('show_window');
/// ```
#[command]
pub fn show_window(app_handle: tauri::AppHandle, label: &str) -> Result<(), String> {
    if let Some(window) = app_handle.get_webview_window(label) {
        if !window
            .is_visible()
            .map_err(|e| t!("main.failed_to_check_window_visibility", error = e))?
        {
            window
                .show()
                .map_err(|e| t!("main.failed_to_show_window", error = e))?;
        }
        window
            .set_focus()
            .map_err(|e| t!("main.failed_to_set_window_focus", error = e))?;
    }
    Ok(())
}

#[derive(serde::Serialize, Clone)]
struct UrlWindowPayload {
    url: String,
}
/// Opens a URL in a new window via event system
///
/// IMPORTANT: We use events instead of direct window creation to avoid deadlocks
/// on Windows. This is because WebView2 requires window creation to happen on the
/// main UI thread, while IPC handlers run on a different thread. Using events
/// ensures window creation occurs on the correct thread.
///
/// See: https://github.com/tauri-apps/wry/issues/583
///
/// # Arguments
/// - `app_handle` - Tauri application handle
/// - `url` - URL to open in the window
///
/// # Example
/// ```js
/// import { invoke } from '@tauri-apps/api/core'
///
/// await invoke('open_url', { url: 'https://example.com' });
/// ```
#[command]
pub async fn open_url(app_handle: tauri::AppHandle, url: String) -> Result<(), String> {
    // Get the main window to emit the event
    let main_window = app_handle
        .get_webview_window("main")
        .ok_or_else(|| t!("main.window_not_ready"))?;

    // Emit an event to create the window on the main thread
    // use events instead of direct window creation to avoid deadlocks on Windows
    main_window
        .emit("create-url-window", UrlWindowPayload { url })
        .map_err(|e| t!("main.failed_to_emit_event", error = e))?;

    Ok(())
}

/// Sync the state of the application
///
/// It is used to sync the state of the application.
///
/// # Arguments
/// - `app` - The app handle, automatically injected by Tauri
/// - `sync_type` - The type of sync to perform
/// - `label` - The window label of the sync, the available labels can be found in `src-tauri/tauri.conf.json`: app.windows[window_config_index].label
///
/// # Example
///
/// ```js
/// import { invoke } from '@tauri-apps/api/core';
///
/// await invoke('sync_state', { syncType: 'model', label: 'model' });
/// ```
#[tauri::command]
pub fn sync_state(app: tauri::AppHandle, sync_type: &str, label: &str, metadata: Option<Value>) {
    let mut payload: HashMap<String, Value> = HashMap::new();
    payload.insert("type".to_string(), Value::String(sync_type.to_string()));
    payload.insert("label".to_string(), Value::String(label.to_string()));
    if let Some(metadata) = metadata {
        payload.insert("metadata".to_string(), metadata);
    }

    let _ = app.emit("sync_state", payload);
}

/// Toggle the always on top state of a window
///
/// Note: Only the assistant window is supported now.
///
/// # Arguments
/// - `app` - The app handle
/// - `window_label` - The label of the window
/// - `new_state` - The new state to set
///
/// # Returns
/// - `Result<bool, String>` - The new state
#[tauri::command]
pub async fn toggle_window_always_on_top(
    app: tauri::AppHandle,
    window_label: &str,
    new_state: bool,
) -> Result<bool, String> {
    if window_label == "assistant" || window_label == "main" {
        let window = app.get_webview_window(window_label).ok_or_else(|| {
            t!(
                "main.failed_to_find_window_with_label",
                label = window_label
            )
            .to_string()
        })?;

        // Set always on top state
        window
            .set_always_on_top(new_state)
            .map_err(|e| t!("main.failed_to_set_window_always_on_top", error = e).to_string())?;

        // Update global state
        if window_label == "assistant" {
            ASSISTANT_ALWAYS_ON_TOP.store(new_state, Ordering::Relaxed);
        } else if window_label == "main" {
            MAIN_WINDOW_ALWAYS_ON_TOP.store(new_state, Ordering::Relaxed);
        }
    }

    Ok(new_state)
}

/// Get the always on top state of a window.
///
/// Note: Only the assistant window is supported now.
///
/// # Arguments
/// - `window_label` - The label of the window
///
/// # Returns
/// - `bool` - The always on top state
#[tauri::command]
pub fn get_window_always_on_top(window_label: &str) -> bool {
    match window_label {
        "assistant" => ASSISTANT_ALWAYS_ON_TOP.load(Ordering::Relaxed),
        "main" => MAIN_WINDOW_ALWAYS_ON_TOP.load(Ordering::Relaxed),
        _ => false,
    }
}

/// Quit the application
///
/// # Arguments
/// - `app` - The app handle
#[command]
pub fn quit_window(app: tauri::AppHandle) -> Result<(), String> {
    for (_, window) in app.webview_windows() {
        window
            .close()
            .map_err(|e| t!("command.window.failed_to_close", error = e.to_string()).to_string())?;
    }
    std::process::exit(0);
}
