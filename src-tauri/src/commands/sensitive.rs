use crate::db::MainStore;
use crate::error::{AppError, Result};
use crate::sensitive::manager::{FilterManager, SensitiveConfig};
use std::sync::{Arc, RwLock};
use tauri::State;

#[tauri::command]
pub fn get_sensitive_config(
    main_store: State<'_, Arc<RwLock<MainStore>>>,
) -> Result<SensitiveConfig> {
    let store = main_store
        .read()
        .map_err(|e| AppError::Db(crate::db::StoreError::IoError(e.to_string())))?;
    Ok(store.get_config("sensitive_config", SensitiveConfig::default()))
}

#[tauri::command]
pub fn update_sensitive_config(
    main_store: State<'_, Arc<RwLock<MainStore>>>,
    config: SensitiveConfig,
) -> Result<()> {
    let mut store = main_store
        .write()
        .map_err(|e| AppError::Db(crate::db::StoreError::IoError(e.to_string())))?;

    // Convert config to Value for storage
    let config_value = serde_json::to_value(&config).map_err(|e| AppError::General {
        message: format!("Failed to serialize config: {}", e),
    })?;

    // Use set_config to save to database and update memory
    store
        .set_config("sensitive_config", &config_value)
        .map_err(|e| AppError::Db(e))?;

    Ok(())
}

#[derive(serde::Serialize)]
pub struct FilterStatus {
    pub healthy: bool,
    pub error: Option<String>,
}

#[tauri::command]
pub fn get_sensitive_status(filter_manager: State<'_, FilterManager>) -> FilterStatus {
    FilterStatus {
        healthy: filter_manager.is_healthy,
        error: filter_manager.error_message.clone(),
    }
}

#[tauri::command]
pub fn get_supported_filters(filter_manager: State<'_, FilterManager>) -> Result<Vec<String>> {
    if !filter_manager.is_healthy {
        return Ok(Vec::new());
    }
    Ok(filter_manager.supported_filter_types())
}
