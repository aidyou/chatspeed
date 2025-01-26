use serde_json::{json, Value};
use std::env::consts;

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
