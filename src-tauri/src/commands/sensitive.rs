use crate::db::MainStore;
use crate::error::{AppError, Result};
use crate::sensitive::manager::{FilterManager, SensitiveConfig};
use std::sync::{Arc, RwLock};
use tauri::{AppHandle, Manager, State};

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

/// Get the status of the sensitive information filter.
/// Uses try_state to gracefully handle race conditions during app startup
/// when the FilterManager might not be registered yet.
#[tauri::command]
pub fn get_sensitive_status(app: AppHandle) -> FilterStatus {
    match app.try_state::<FilterManager>() {
        Some(filter_manager) => FilterStatus {
            healthy: filter_manager.is_healthy,
            error: filter_manager.error_message.clone(),
        },
        None => {
            // FilterManager not yet registered - likely a race condition during startup
            log::warn!("FilterManager state not yet available, returning unhealthy status");
            FilterStatus {
                healthy: false,
                error: Some("Filter module is still initializing...".to_string()),
            }
        }
    }
}

/// Get the list of supported filter types.
/// Uses try_state to gracefully handle race conditions during app startup.
#[tauri::command]
pub fn get_supported_filters(app: AppHandle) -> Result<Vec<String>> {
    match app.try_state::<FilterManager>() {
        Some(filter_manager) => {
            if !filter_manager.is_healthy {
                return Ok(Vec::new());
            }
            Ok(filter_manager.supported_filter_types())
        }
        None => {
            // FilterManager not yet registered
            log::warn!("FilterManager state not yet available for get_supported_filters");
            Ok(Vec::new())
        }
    }
}
