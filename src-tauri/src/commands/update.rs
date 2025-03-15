use rust_i18n::t;
use serde::Serialize;
use tauri::AppHandle;
use thiserror::Error;

use crate::{UpdateError, UpdateManager, VersionInfo};

/// Error type for update commands
#[derive(Debug, Error, Serialize)]
#[serde(tag = "type", content = "message")]
pub enum Error {
    /// Update check failed
    #[error("Update check failed: {0}")]
    CheckFailed(String),
    /// Update installation failed
    #[error("Update installation failed: {0}")]
    InstallFailed(String),
}

impl From<UpdateError> for Error {
    fn from(err: UpdateError) -> Self {
        match err {
            UpdateError::VersionCheckError(msg)
            | UpdateError::ConfigError(msg)
            | UpdateError::CheckError(msg) => Error::CheckFailed(msg),
            UpdateError::DownloadError(msg) => Error::InstallFailed(msg),
            UpdateError::VersionMismatch => {
                Error::InstallFailed(t!("updater.version_mismatch").into())
            }
            UpdateError::UpdateNotFound => {
                Error::InstallFailed(t!("updater.update_not_found").into())
            }
        }
    }
}

type Result<T> = std::result::Result<T, Error>;

/// Command to check for available updates
#[tauri::command]
pub async fn check_update(app: AppHandle) -> Result<Option<VersionInfo>> {
    let manager = UpdateManager::new(app);
    manager.check_update().await.map_err(Into::into)
}

/// Command to confirm an update
#[tauri::command]
pub async fn confirm_update(app: AppHandle, version_info: VersionInfo) -> Result<()> {
    let update_manager = UpdateManager::new(app);
    update_manager
        .download_and_install(&version_info)
        .await
        .map_err(|e| Error::InstallFailed(e.to_string()))
}

/// Command to download and install an update
#[tauri::command]
pub async fn install_update(app: AppHandle, version_info: VersionInfo) -> Result<()> {
    let manager = UpdateManager::new(app);
    manager
        .download_and_install(&version_info)
        .await
        .map_err(Into::into)
}

/// Command to restart the application after update
#[tauri::command]
pub async fn restart_app(app: AppHandle) -> Result<()> {
    app.restart()
}
