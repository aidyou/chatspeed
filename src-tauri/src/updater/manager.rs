//! Update manager implementation
//!
//! Provides functionality for checking and installing updates using Tauri's built-in
//! update system. The update information is hosted on multiple mirrors for better availability:
//! - Primary: GitHub releases
//! - Fallback: jsDelivr CDN
//!
//! The updater will try each endpoint in order until it finds a working one.

use super::error::{Result, UpdateError};
use super::types::VersionInfo;
use log::{info, warn};
use semver::Version;
use std::time::{Duration, Instant};
use tauri::async_runtime::spawn;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::{Update, UpdaterExt};

const EVENT_UPDATE_PROGRESS: &str = "update://download-progress";
const EVENT_UPDATE_READY: &str = "update://ready";
const EVENT_UPDATE_AVAILABLE: &str = "update://available";
const PROGRESS_REPORT_INTERVAL: Duration = Duration::from_secs(1);

/// Manages the update process
pub struct UpdateManager {
    app: AppHandle,
}

/// Downloads an update in the background.
///
/// # Arguments
/// * `app` - The Tauri AppHandle
/// * `update` - The update to download
async fn download_update(app: AppHandle, update: Update) -> Result<()> {
    info!(
        "Starting background download for version: {}",
        update.version
    );
    let last_report_time = std::sync::Arc::new(std::sync::Mutex::new(Instant::now()));

    // The `update://ready` event is emitted automatically by the plugin when download is complete
    let on_finish = || {
        info!(
            "Background download completed for version: {}",
            update.version
        );
    };

    update
        .download(
            move |current, total| {
                let now = Instant::now();
                let mut last_time = last_report_time.lock().unwrap();

                if now.duration_since(*last_time) >= PROGRESS_REPORT_INTERVAL {
                    if let Some(total) = total {
                        if total > 0 {
                            let progress = (current as f64 / total as f64) * 100.0;
                            info!(
                                "Download progress: {:.1}% ({}/{})",
                                progress, current, total
                            );

                            let progress_data = serde_json::json!({
                                "progress": progress / 100.0,
                                "current": current,
                                "total": total
                            });
                            let _ = app.emit(EVENT_UPDATE_PROGRESS, progress_data);
                        } else {
                            info!("Download progress: total size unknown ({} bytes)", current);
                            let progress_data = serde_json::json!({
                                "progress": 0.0,
                                "current": current,
                                "total": 0
                            });
                            let _ = app.emit(EVENT_UPDATE_PROGRESS, progress_data);
                        }
                    }
                    *last_time = now;
                }
            },
            on_finish,
        )
        .await
        .map_err(|e| {
            warn!("Download failed: {}", e);
            UpdateError::DownloadError(e.to_string())
        })?;

    Ok(())
}

impl UpdateManager {
    /// Creates a new UpdateManager instance
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }

    /// Notify frontend about available update
    pub fn notify_update_available(&self, version_info: &VersionInfo) {
        let _ = self.app.emit(EVENT_UPDATE_AVAILABLE, version_info);
    }

    /// Checks whether the provided version should be installed
    pub fn should_install(&self, version_info: &VersionInfo) -> Result<bool> {
        let current_version_str = self.app.package_info().version.to_string();
        let new_version_str = &version_info.version;

        let new_version = Version::parse(new_version_str).map_err(|e| {
            UpdateError::VersionParseError(new_version_str.to_string(), e.to_string())
        })?;
        let current_version = Version::parse(&current_version_str)
            .map_err(|e| UpdateError::VersionParseError(current_version_str, e.to_string()))?;

        Ok(new_version > current_version)
    }

    /// Checks for available updates and automatically downloads them in the background.
    ///
    /// Returns `Ok(())`. The update process is communicated via events to the frontend.
    pub async fn check_and_download_update(&self) -> Result<()> {
        let updater = self
            .app
            .updater()
            .map_err(|e| UpdateError::ConfigError(e.to_string()))?;

        match updater.check().await {
            Ok(Some(update)) => {
                info!("New version available: {}", update.version);
                let version_info = VersionInfo {
                    version: update.version.to_string(),
                    notes: update.body.clone().unwrap_or_default(),
                };

                if self.should_install(&version_info)? {
                    // Notify frontend that a new version is available
                    self.notify_update_available(&version_info);

                    // Spawn a background task to download the update
                    let app_clone = self.app.clone();
                    spawn(async move {
                        if let Err(e) = download_update(app_clone.clone(), update).await {
                            warn!("Background download failed: {}", e);
                            // Optionally, emit a download-failed event to the frontend
                            let _ = app_clone.emit("update://download-failed", e.to_string());
                        }
                    });
                } else {
                    info!("Current version is up to date");
                }
            }
            Ok(None) => {
                info!("No updates available");
            }
            Err(e) => {
                warn!("Failed to check for updates: {}", e);
                return Err(UpdateError::UpdateRequestError(e.to_string()));
            }
        }
        Ok(())
    }
}
