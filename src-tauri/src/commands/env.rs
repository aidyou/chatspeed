use serde_json::{json, Value};
use std::env::consts;

use crate::constants::get_static_var;

/// Get the OS information
///
/// This function is used to get the OS information from the system.
///
/// # Returns
/// * `Value` - Returns the OS information as a JSON value
///
/// # Example
/// ```js
/// import { invoke } from '@tauri-apps/api/core';
///
/// const osInfo = await invoke('get_os_info');
/// console.log(osInfo);
/// ```
#[tauri::command]
pub fn get_os_info() -> Value {
    json!({
        "os": consts::OS,
        "arch": consts::ARCH,
    })
}

#[tauri::command]
pub fn get_env() -> Value {
    json!({
        "httpServer": get_static_var(&crate::constants::HTTP_SERVER).to_string(),
        "chatCompletionProxy": get_static_var(&crate::constants::CHAT_COMPLETION_PROXY).to_string(),
        "logDir": get_static_var(&crate::constants::LOG_DIR).display().to_string(),
    })
}
