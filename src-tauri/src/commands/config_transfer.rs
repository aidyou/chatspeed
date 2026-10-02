//! Configuration package transfer Tauri commands.
//!
//! Export and import run against the runtime-owned database through
//! `/control/v1/data-commands/*`; the client still chooses the authorized file
//! path. Package *inspection* is a pure read of a client-chosen file and stays
//! local.

use std::sync::Arc;

use tauri::{command, AppHandle, Emitter, State};

use crate::db::config_transfer::{ConfigCategory, ConfigImportResult, ConfigTransferPreview};
use crate::error::{AppError, Result as AppResult};
use crate::runtime_client::RuntimeSupervisor;

/// Exports a configuration package to a client-chosen path.
#[command]
pub async fn export_config_package(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    path: String,
    categories: Vec<ConfigCategory>,
) -> Result<ConfigTransferPreview, String> {
    crate::runtime_data::export_config_package(supervisor.inner().as_ref(), path, categories).await
}

/// Inspects a configuration package without touching the store.
#[command]
pub fn inspect_config_package(path: String) -> AppResult<ConfigTransferPreview> {
    crate::db::config_transfer::inspect_config_package(path).map_err(AppError::Db)
}

/// Imports a configuration package from a client-chosen path.
#[command]
pub async fn import_config_package(
    app: AppHandle,
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    path: String,
    categories: Vec<ConfigCategory>,
) -> Result<ConfigImportResult, String> {
    let result = crate::runtime_data::import_config_package(
        supervisor.inner().as_ref(),
        path,
        categories.clone(),
    )
    .await?;
    app.emit(
        "cs://sync-state",
        serde_json::json!({
            "type": "config_imported",
            "categories": categories,
            "result": result,
            "windowLabel": ""
        }),
    )
    .map_err(|error| error.to_string())?;
    Ok(result)
}
