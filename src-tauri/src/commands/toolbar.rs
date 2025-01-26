use std::sync::{Arc, Mutex};

use rust_i18n::t;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::{
    db::config_store::ConfigStore, snap::text_monitor::TextMonitorManager,
    CFG_WORD_SELECTION_TOOLBAR,
};
/// Opens the screenshot permission settings
///
/// # Returns
/// * `Result<(), String>` - A result indicating success or failure
///
/// # Example
///
/// ```js
/// // Call from frontend:
/// import { invoke } from '@tauri-apps/api/core';
///
/// const result = await invoke('open_screenshot_permission_settings');
/// console.log(result);
/// ```
#[tauri::command]
pub fn open_screenshot_permission_settings() -> Result<(), String> {
    snap_rs::get_permission_checker()
        .open_permission_settings(snap_rs::PermissionType::Screenshot)
        .map_err(|e| {
            t!(
                "permission.failed_to_open_screenshot_permission_settings",
                error = e
            )
            .to_string()
        })
}

/// Opens the text selection permission settings
///
/// # Returns
/// * `Result<(), String>` - A result indicating success or failure
///
/// # Example
///
/// ```js
/// // Call from frontend:
/// import { invoke } from '@tauri-apps/api/core';
///
/// const result = await invoke('open_text_selection_permission_settings');
/// console.log(result);
/// ```
#[tauri::command]
pub fn open_text_selection_permission_settings() -> Result<(), String> {
    snap_rs::get_permission_checker()
        .open_permission_settings(snap_rs::PermissionType::TextSelection)
        .map_err(|e| {
            t!(
                "permission.failed_to_open_text_selection_permission_settings",
                error = e
            )
            .to_string()
        })
}

/// Checks the text selection permission
///
/// # Returns
/// * `Result<(), String>` - A result indicating success or failure
///
/// # Example
///
/// ```js
/// // Call from frontend:
/// import { invoke } from '@tauri-apps/api/core';
///
/// const result = await invoke('check_text_selection_permission');
/// console.log(result);
/// ```
#[tauri::command]
pub fn check_text_selection_permission() -> Result<(), String> {
    if snap_rs::get_permission_checker()
        .check_permission(snap_rs::PermissionType::TextSelection)
        .is_ok()
    {
        Ok(())
    } else {
        Err(t!("permission.failed_to_get_text_selection_permission").to_string())
    }
}

/// Checks the screenshot permission
///
/// # Returns
/// * `Result<(), String>` - A result indicating success or failure
///
/// # Example
///
/// ```js
/// // Call from frontend:
/// import { invoke } from '@tauri-apps/api/core';
///
/// const result = await invoke('check_screenshot_permission');
/// console.log(result);
/// ```
#[tauri::command]
pub fn check_screenshot_permission() -> Result<(), String> {
    if snap_rs::get_permission_checker()
        .check_permission(snap_rs::PermissionType::Screenshot)
        .is_ok()
    {
        Ok(())
    } else {
        Err(t!("permission.failed_to_get_screenshot_permission").to_string())
    }
}

/// Starts the text monitor
///
/// # Parameters
/// * `app`: The app handle
///
/// # Returns
/// * `Result<(), String>` - A result indicating success or failure
#[tauri::command]
pub fn start_text_monitor(app: AppHandle, force: Option<bool>) -> Result<(), String> {
    let config_store = app.state::<Arc<Mutex<ConfigStore>>>();
    let monitor = app.state::<Arc<Mutex<TextMonitorManager>>>();
    let is_start = if let Some(force) = force {
        force
    } else {
        if let Ok(c) = config_store.clone().lock() {
            if let Some(desktop_toolbar) = c.config.get_setting(CFG_WORD_SELECTION_TOOLBAR) {
                desktop_toolbar.as_bool().unwrap_or(false)
            } else {
                false
            }
        } else {
            false
        }
    };
    if !is_start {
        log::info!("Text selection toolbar is not enabled");
        return Ok(());
    }
    // check permission
    if check_text_selection_permission().is_ok() {
        let monitor_clone = monitor.clone();
        // 在异步块之前获取 receiver
        let mut rx = {
            let monitor_guard = monitor_clone.lock().map_err(|e| e.to_string())?;
            monitor_guard.subscribe()
        };

        let app_handle = app.clone();
        // 启动监听
        tauri::async_runtime::spawn(async move {
            while let Ok(event) = rx.recv().await {
                // 处理选中的文本
                println!("Selected text: {}", event.text);

                // 发送事件到前端
                if let Err(e) = app_handle.emit("text-selected", &event) {
                    eprintln!("Failed to emit text event: {}", e);
                }
            }
        });

        // 启动监控
        if let Ok(monitor_guard) = monitor.lock() {
            monitor_guard.start()?;
        } else {
            log::info!("Text monitor has not been setup properly");
        }
    } else {
        log::error!("Text selection has no permission");
    }
    Ok(())
}

/// Stops the text monitor
///
/// # Parameters
/// * `monitor`: The text monitor manager
///
/// # Returns
/// * `Result<(), String>` - A result indicating success or failure
#[tauri::command]
pub fn stop_text_monitor(monitor: State<Arc<Mutex<TextMonitorManager>>>) -> Result<(), String> {
    if let Ok(monitor_guard) = monitor.lock() {
        monitor_guard.stop()?;
    }
    Ok(())
}
