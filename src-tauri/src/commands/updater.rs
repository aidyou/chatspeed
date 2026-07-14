#![allow(clippy::module_name_repetitions)]

use std::sync::Arc;

use log::error;
use serde::Serialize;
use tauri::Manager;

use crate::error::{AppError, Result};
use crate::updater::UpdateError::UpdateNotFound;
use crate::updater::{UpdateCheckOutcome, UpdateManager};

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub struct UpdateCheckResponse {
    status: UpdateCheckStatus,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum UpdateCheckStatus {
    NoUpdate,
    Started,
    InProgress,
    ReadyToInstall,
}

#[tauri::command]
pub async fn check_for_updates(app: tauri::AppHandle) -> Result<UpdateCheckResponse> {
    if let Some(update_manager) = app.try_state::<Arc<UpdateManager>>() {
        let status = match update_manager
            .check_and_download_update()
            .await
            .map_err(AppError::Updater)?
        {
            UpdateCheckOutcome::NoUpdate => UpdateCheckStatus::NoUpdate,
            UpdateCheckOutcome::Started => UpdateCheckStatus::Started,
            UpdateCheckOutcome::InProgress => UpdateCheckStatus::InProgress,
            UpdateCheckOutcome::ReadyToInstall => UpdateCheckStatus::ReadyToInstall,
        };

        Ok(UpdateCheckResponse { status })
    } else {
        let e = "UpdateManager not found in state".to_string();
        error!("{}", e);
        Err(AppError::Updater(UpdateNotFound))
    }
}

#[tauri::command]
pub async fn install_and_restart(app: tauri::AppHandle) -> Result<()> {
    if let Some(update_manager) = app.try_state::<Arc<UpdateManager>>() {
        if let Err(e) = update_manager.install_and_restart().await {
            error!("Failed to install and restart: {}", e);
            return Err(AppError::Updater(e));
        }
    } else {
        let e = "UpdateManager not found in state".to_string();
        error!("{}", e);
        return Err(AppError::Updater(UpdateNotFound));
    }
    Ok(())
}
