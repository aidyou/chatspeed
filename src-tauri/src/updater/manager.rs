//! Update manager implementation
//!
//! Provides functionality for checking and installing application updates.

use super::error::{Result, UpdateError};
use super::types::VersionInfo;
use log::{info, warn};
use semver::Version;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::async_runtime::spawn;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

const EVENT_UPDATE_PROGRESS: &str = "update://download-progress";
const EVENT_UPDATE_READY: &str = "update://ready";
const EVENT_UPDATE_AVAILABLE: &str = "update://available";

#[derive(Clone)]
struct UpdateState {
    update: Update,
    tmp_path: PathBuf,
}

/// Manages the update process
pub struct UpdateManager {
    app: AppHandle,
    latest_update: Arc<Mutex<Option<UpdateState>>>,
}

impl UpdateManager {
    /// Creates a new UpdateManager instance
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            latest_update: Arc::new(Mutex::new(None)),
        }
    }

    /// Checks for available updates and automatically downloads them in the background.
    pub async fn check_and_download_update(&self) -> Result<()> {
        let updater = self
            .app
            .updater()
            .map_err(|e| UpdateError::ConfigError(e.to_string()))?;

        if let Ok(Some(update)) = updater.check().await {
            info!("New version available: {}", update.version);
            let version_info = VersionInfo {
                version: update.version.to_string(),
                notes: update.body.clone().unwrap_or_default(),
            };

            if self.should_install(&version_info)? {
                self.notify_update_available(&version_info);

                let app_clone = self.app.clone();
                let latest_update_clone = self.latest_update.clone();
                spawn(async move {
                    if let Err(e) =
                        download_update_to_file(app_clone.clone(), latest_update_clone, update)
                            .await
                    {
                        warn!("Background download failed: {}", e);
                        let _ = app_clone.emit("update://download-failed", e.to_string());
                    }
                });
            }
        }

        Ok(())
    }

    /// Installs the latest downloaded update and restarts the application.
    pub async fn install_and_restart(&self) -> Result<()> {
        let update_state = self
            .latest_update
            .lock()
            .map_err(|_| UpdateError::LockError("Failed to lock latest_update mutex".to_string()))?
            .take();

        if let Some(state) = update_state {
            info!("Installing update and restarting...");
            let bytes = tokio::fs::read(&state.tmp_path).await?;
            state
                .update
                .install(&bytes)
                .map_err(|e| UpdateError::InstallError(e.to_string()))?;
            tokio::fs::remove_file(&state.tmp_path).await?;
            self.app.restart();
        } else {
            warn!("No update available to install.");
        }

        Ok(())
    }

    fn notify_update_available(&self, version_info: &VersionInfo) {
        let _ = self.app.emit(EVENT_UPDATE_AVAILABLE, version_info);
    }

    fn should_install(&self, version_info: &VersionInfo) -> Result<bool> {
        let current_version_str = self.app.package_info().version.to_string();
        let new_version_str = &version_info.version;

        let new_version = Version::parse(new_version_str).map_err(|e| {
            UpdateError::VersionParseError(new_version_str.to_string(), e.to_string())
        })?;
        let current_version = Version::parse(&current_version_str)
            .map_err(|e| UpdateError::VersionParseError(current_version_str, e.to_string()))?;

        Ok(new_version > current_version)
    }
}

async fn download_update_to_file(
    app: AppHandle,
    latest_update: Arc<Mutex<Option<UpdateState>>>,
    update: Update,
) -> Result<()> {
    info!(
        "Starting background download for version: {}",
        update.version
    );

    let on_chunk = |chunk_size, total| {
        let progress = if let Some(total) = total {
            (chunk_size as f64 / total as f64) * 100.0
        } else {
            0.0
        };
        let _ = app.emit(
            EVENT_UPDATE_PROGRESS,
            serde_json::json!({ "progress": progress }),
        );
    };

    let on_finish = || {
        info!(
            "Background download completed for version: {}",
            update.version
        );
    };

    let bytes = update
        .download(on_chunk, on_finish)
        .await
        .map_err(|e| UpdateError::DownloadError(e.to_string()))?;

    let cache_dir = app.path().app_cache_dir().map_err(|e| {
        UpdateError::IoError(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            e.to_string(),
        ))
    })?;
    if !cache_dir.exists() {
        tokio::fs::create_dir_all(&cache_dir).await?;
    }
    let tmp_path = cache_dir.join(format!("update-{}.bin", update.version));

    tokio::fs::write(&tmp_path, &bytes).await?;

    let mut lu = latest_update
        .lock()
        .map_err(|_| UpdateError::LockError("Failed to lock latest_update mutex".to_string()))?;
    *lu = Some(UpdateState { update, tmp_path });

    let _ = app.emit(EVENT_UPDATE_READY, ());

    Ok(())
}
