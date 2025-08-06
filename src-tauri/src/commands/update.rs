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
            UpdateError::VersionParseError(version, error) => Error::CheckFailed(
                t!(
                    "updater.errors.version_parse",
                    version = version,
                    error = error
                )
                .to_string(),
            ),
            UpdateError::UpdateRequestError(error) => {
                Error::CheckFailed(t!("updater.errors.update_request", error = error).to_string())
            }
            UpdateError::DownloadError(error) => {
                Error::InstallFailed(t!("updater.errors.download", error = error).to_string())
            }
            UpdateError::ConfigError(error) => {
                Error::CheckFailed(t!("updater.errors.config", error = error).to_string())
            }
            UpdateError::VersionMismatch => {
                Error::InstallFailed(t!("updater.errors.version_mismatch").into())
            }
            UpdateError::UpdateNotFound => {
                Error::InstallFailed(t!("updater.errors.update_not_found").into())
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
        .map_err(Into::into)
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
