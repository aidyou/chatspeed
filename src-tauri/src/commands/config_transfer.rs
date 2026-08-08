use std::sync::Arc;

use tauri::{command, AppHandle, Emitter, State};

use crate::{
    db::{
        config_transfer::{self, ConfigCategory, ConfigImportResult, ConfigTransferPreview},
        MainStore,
    },
    error::{AppError, Result},
};

#[command]
pub fn export_config_package(
    state: State<Arc<MainStore>>,
    path: String,
    categories: Vec<ConfigCategory>,
) -> Result<ConfigTransferPreview> {
    let runtime = state.db_runtime().map_err(AppError::Db)?;
    runtime
        .read_blocking(move |conn| config_transfer::export_config_package(conn, path, categories))
        .map_err(AppError::Db)
}

#[command]
pub fn inspect_config_package(path: String) -> Result<ConfigTransferPreview> {
    config_transfer::inspect_config_package(path).map_err(AppError::Db)
}

#[command]
pub fn import_config_package(
    app: AppHandle,
    state: State<Arc<MainStore>>,
    path: String,
    categories: Vec<ConfigCategory>,
) -> Result<ConfigImportResult> {
    let result =
        config_transfer::import_config_package(state.inner().as_ref(), path, categories.clone())
            .map_err(AppError::Db)?;
    app.emit(
        "cs://sync-state",
        serde_json::json!({
            "type": "config_imported",
            "categories": categories,
            "result": result,
            "windowLabel": ""
        }),
    )
    .map_err(AppError::from)?;
    Ok(result)
}
