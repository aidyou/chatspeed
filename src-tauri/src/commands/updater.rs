#![allow(clippy::module_name_repetitions)]

use std::sync::Arc;

use log::error;
use tauri::Manager;

use crate::updater::UpdateManager;

#[tauri::command]
pub async fn install_and_restart(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(update_manager) = app.try_state::<Arc<UpdateManager>>() {
        if let Err(e) = update_manager.install_and_restart().await {
            error!("Failed to install and restart: {}", e);
            return Err(e.to_string());
        }
    } else {
        let e = "UpdateManager not found in state".to_string();
        error!("{}", e);
        return Err(e);
    }
    Ok(())
}
