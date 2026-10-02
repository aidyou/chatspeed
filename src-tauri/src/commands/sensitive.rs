//! Sensitive-filter Tauri commands.
//!
//! The sensitive-filter *configuration* is runtime-owned data and is reached
//! through `/control/v1/data-commands/*`. The `FilterManager` status commands
//! stay local: the manager is a desktop process object the UI inspects, and the
//! authoritative filtering itself runs inside the runtime message core.

use std::sync::Arc;

use tauri::{AppHandle, Manager, State};

use crate::runtime_client::RuntimeSupervisor;
use crate::sensitive::manager::{FilterManager, SensitiveConfig};

/// Returns the sensitive-filter configuration from the runtime.
#[tauri::command]
pub async fn get_sensitive_config(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
) -> Result<SensitiveConfig, String> {
    crate::runtime_data::get_sensitive_config(supervisor.inner().as_ref()).await
}

/// Replaces the sensitive-filter configuration in the runtime.
#[tauri::command]
pub async fn update_sensitive_config(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    config: SensitiveConfig,
) -> Result<(), String> {
    crate::runtime_data::update_sensitive_config(supervisor.inner().as_ref(), config).await
}

#[derive(serde::Serialize)]
pub struct FilterStatus {
    pub healthy: bool,
    pub error: Option<String>,
}

/// Returns the status of the local sensitive information filter.
///
/// Uses `try_state` to gracefully handle race conditions during app startup
/// when the FilterManager might not be registered yet.
#[tauri::command]
pub fn get_sensitive_status(app: AppHandle) -> FilterStatus {
    match app.try_state::<FilterManager>() {
        Some(filter_manager) => FilterStatus {
            healthy: filter_manager.is_healthy,
            error: filter_manager.error_message.clone(),
        },
        None => {
            log::warn!("FilterManager state not yet available, returning unhealthy status");
            FilterStatus {
                healthy: false,
                error: Some("Filter module is still initializing...".to_string()),
            }
        }
    }
}

/// Returns the list of supported local filter types.
#[tauri::command]
pub fn get_supported_filters(app: AppHandle) -> Result<Vec<String>, String> {
    match app.try_state::<FilterManager>() {
        Some(filter_manager) => {
            if !filter_manager.is_healthy {
                return Ok(Vec::new());
            }
            Ok(filter_manager.supported_filter_types())
        }
        None => {
            log::warn!("FilterManager state not yet available for get_supported_filters");
            Ok(Vec::new())
        }
    }
}
