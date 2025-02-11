use std::collections::HashMap;
use std::sync::atomic::Ordering;

use rust_i18n::t;
use serde_json::Value;
// use tauri::utils::{config::WindowEffectsConfig, WindowEffect};
use crate::constants::ASSISTANT_ALWAYS_ON_TOP;
use tauri::{command, Emitter, LogicalSize, Manager, Runtime, WebviewWindowBuilder}; //window::Color, WindowEvent,

/// Open or focus the settings window
///
/// This function is used to open a new setting window, or if the window already exists, it displays and focuses the window.
///
/// # Parameters
/// - `app_handle` - Tauri application handle, used to manage windows, automatically injected by Tauri
/// - `setting_type` - The type of setting to open, optional, value can be `general`, `model`, `skill`, `privacy`, `about`
///
/// # Example
/// ```js
/// import { invoke } from '@tauri-apps/api/core'
///
/// await invoke('open_setting_window', { settingType: 'model' });
/// ```
#[command]
pub fn open_setting_window(
    app_handle: tauri::AppHandle,
    setting_type: Option<&str>,
) -> Result<(), String> {
    let label = "settings";
    if let Some(window) = app_handle.get_webview_window(label) {
        if let Some(setting_type) = setting_type {
            window
                .eval(&format!(
                    "window.location.href = '/settings/{}';console.log('/settings/{}')",
                    setting_type, setting_type
                ))
                .map_err(|e| t!("main.failed_to_navigate_to_settings", error = e))?;
        }
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
    } else {
        let webview_window = WebviewWindowBuilder::new(
            &app_handle,
            label,
            tauri::WebviewUrl::App(format!("/settings/{}", setting_type.unwrap_or("")).into()),
        )
        .title("")
        .decorations(false)
        .skip_taskbar(true)
        .maximizable(false)
        .inner_size(600.0, 700.0)
        .min_inner_size(600.0, 600.0)
        .transparent(true)
        .visible(false)
        .build()
        .map_err(|e| t!("main.failed_to_create_settings_window", error = e))?;

        if let Ok(Some(monitor)) = webview_window.current_monitor() {
            webview_window
                .set_max_size(Some(tauri::Size::Logical(LogicalSize {
                    width: 600.0,
                    height: monitor.size().height as f64,
                })))
                .map_err(|e| t!("main.failed_to_set_max_window_size", error = e))?;
        }

        webview_window
            .show()
            .map_err(|e| t!("main.failed_to_show_window", error = e))?;
        webview_window
            .set_focus()
            .map_err(|e| t!("main.failed_to_set_window_focus", error = e))?;

        let window_clone = webview_window.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = crate::window::fix_window_visual(&window_clone, None).await {
                log::error!("{}", t!("main.failed_to_fix_window_visual", error = e));
            }
        });
    }
    Ok(())
}

/// Open or focus the note window
///
/// This function is used to open a new note window, or if the window already exists, it displays and focuses the window.
///
/// # Parameters
/// - `app_handle` - Tauri application handle
#[command]
pub fn open_note_window(app_handle: tauri::AppHandle) -> Result<(), String> {
    let label = "note";
    #[cfg(debug_assertions)]
    log::debug!("Opening note window");

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
    } else {
        #[cfg(debug_assertions)]
        log::debug!("Creating new note window");

        let webview_window =
            WebviewWindowBuilder::new(&app_handle, label, tauri::WebviewUrl::App("/note".into()))
                .title("Notes")
                .decorations(false)
                .skip_taskbar(true)
                .maximizable(true)
                .resizable(true)
                .inner_size(850.0, 600.0)
                .min_inner_size(600.0, 400.0)
                .transparent(true)
                .visible(true)
                .build()
                .map_err(|e| t!("main.failed_to_create_note_window", error = e))?;

        #[cfg(debug_assertions)]
        {
            webview_window.on_window_event(|event| match event {
                tauri::WindowEvent::Focused(focused) => {
                    if *focused {
                        log::debug!("Note window focused");
                    }
                }
                tauri::WindowEvent::Resized(size) => {
                    log::debug!("Note window resized: {}x{}", size.width, size.height);
                }
                tauri::WindowEvent::ThemeChanged(theme) => {
                    log::debug!("Note window theme changed: {:?}", theme);
                }
                tauri::WindowEvent::ScaleFactorChanged {
                    scale_factor,
                    new_inner_size,
                    ..
                } => {
                    log::debug!(
                        "Note window scale factor changed: {}, new size: {}x{}",
                        scale_factor,
                        new_inner_size.width,
                        new_inner_size.height
                    );
                }
                _ => {}
            });
        }

        #[cfg(debug_assertions)]
        log::debug!("Showing note window...");

        webview_window
            .show()
            .map_err(|e| t!("main.failed_to_show_window", error = e))?;

        #[cfg(debug_assertions)]
        log::debug!("Setting note window focus...");

        webview_window
            .set_focus()
            .map_err(|e| t!("main.failed_to_set_window_focus", error = e))?;

        let window_clone = webview_window.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = crate::window::fix_window_visual(&window_clone, None).await {
                log::error!("{}", t!("main.failed_to_fix_note_window_visual", error = e));
            }
        });
    }
    Ok(())
}

/// Show the window by label
///
/// # Parameters
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

/// Opens a URL in the webview window
///
/// Uses a single window for all URLs, creating it if it doesn't exist
/// or navigating to the new URL if it does.
///
/// # Parameters
/// - `app_handle` - Tauri application handle
/// - `url` - URL to open in the webview window
///
/// # Returns
/// - `Result<(), String>` - Success or error message
#[command]
pub fn open_url<R: Runtime>(app_handle: tauri::AppHandle<R>, url: String) -> Result<(), String> {
    let window_label = "webview";

    if let Some(window) = app_handle.get_webview_window(window_label) {
        // Update the URL if the window already exists
        if let Err(e) = window.eval(&format!("window.location.href = '{}';", url)) {
            return Err(t!("main.failed_to_navigate_to_url", url = url, error = e).to_string());
        }

        // 确保窗口可见并获得焦点
        if !window.is_visible().unwrap_or(false) {
            let _ = window.show();
        }
        let _ = window.set_focus();

        Ok(())
    } else {
        // Create a new webview window if it doesn't exist
        let webview_window = WebviewWindowBuilder::new(
            &app_handle,
            window_label,
            tauri::WebviewUrl::App(url.into()),
        )
        .title("Web View")
        .inner_size(1200.0, 800.0)
        .min_inner_size(800.0, 600.0)
        .build()
        .map_err(|e| t!("main.failed_to_create_webview_window", error = e).to_string())?;

        // Show the window and set focus
        let _ = webview_window.show();
        let _ = webview_window.set_focus();

        // cleanup if window is closed
        // let window_clone = webview_window.clone();
        // webview_window.on_window_event(move |event| match event {
        //     tauri::WindowEvent::Destroyed => {
        //         // Clear all browsing data when window is destroyed
        //         if let Err(e) = window_clone.clear_all_browsing_data() {
        //             log::error!("Failed to clear browsing data: {}", e);
        //         }
        //     }
        //     _ => {}
        // });

        Ok(())
    }
}

/// Sync the state of the application
///
/// It is used to sync the state of the application.
///
/// # Parameters
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
/// # Parameters
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
    if window_label == "assistant" {
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
        ASSISTANT_ALWAYS_ON_TOP.store(new_state, Ordering::Relaxed);
    }

    Ok(new_state)
}

/// Get the always on top state of a window.
///
/// Note: Only the assistant window is supported now.
///
/// # Parameters
/// - `window_label` - The label of the window
///
/// # Returns
/// - `bool` - The always on top state
#[tauri::command]
pub fn get_window_always_on_top(window_label: &str) -> bool {
    if window_label == "assistant" {
        ASSISTANT_ALWAYS_ON_TOP.load(Ordering::Relaxed)
    } else {
        false
    }
}

/// Quit the application
///
/// # Parameters
/// - `app` - The app handle
#[command]
pub fn quit_window(app: tauri::AppHandle) -> Result<(), String> {
    for (_, window) in app.webview_windows() {
        window
            .close()
            .map_err(|e| format!("Failed to close window: {}", e))?;
    }
    std::process::exit(0);
}
