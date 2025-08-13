//! Update manager implementation
//!
//! Provides functionality for checking and installing updates using Tauri's built-in
//! update system. The update information is hosted on multiple mirrors for better availability:
//! - Primary: aidyou.ai
//! - Backup: jsDelivr CDN
//! - Fallback: GitHub releases
//!
//! The updater will try each endpoint in order until it finds a working one.

use super::error::{Result, UpdateError};
use super::types::VersionInfo;
use log::{info, warn};
use semver::Version;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

const EVENT_UPDATE_PROGRESS: &str = "update://download-progress";
const EVENT_UPDATE_READY: &str = "update://ready";
const EVENT_UPDATE_AVAILABLE: &str = "update://available";
const PROGRESS_REPORT_INTERVAL: Duration = Duration::from_secs(1);

/// Manages the update process
pub struct UpdateManager {
    app: AppHandle,
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
    ///
    /// # Arguments
    /// * `version_info` - Information about the version to check
    ///
    /// # Returns
    /// * `Ok(true)` if the version should be installed
    /// * `Ok(false)` if the version should not be installed
    /// * `Err(UpdateError)` if there was an error during the check
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

    /// Checks for available updates
    ///
    /// Returns Some(VersionInfo) if an update is available, None if no update is available.
    /// The version info includes the new version number, download URL, and release notes.
    pub async fn check_update(&self) -> Result<Option<VersionInfo>> {
        let updater = self
            .app
            .updater()
            .map_err(|e| UpdateError::ConfigError(e.to_string()))?;

        match updater.check().await {
            Ok(Some(update)) => {
                info!("New version available: {}", update.version);
                let version_info = VersionInfo {
                    version: update.version.to_string(),
                    notes: update.body.unwrap_or_default(),
                };

                if self.should_install(&version_info)? {
                    Ok(Some(version_info))
                } else {
                    info!("Current version is up to date");
                    Ok(None)
                }
            }
            Ok(None) => {
                info!("No updates available");
                Ok(None)
            }
            Err(e) => {
                warn!("Failed to check for updates: {}", e);
                Err(UpdateError::UpdateRequestError(e.to_string()))
            }
        }
    }

    /// Downloads and installs an update
    ///
    /// # Arguments
    /// * `version_info` - Information about the version to install
    ///
    /// # Returns
    /// * `Ok(())` if the update was successfully downloaded and installed
    /// * `Err(UpdateError)` if there was an error during the process
    pub async fn download_and_install(&self, version_info: &VersionInfo) -> Result<()> {
        info!(
            "Starting download and install for version: {}",
            version_info.version
        );
        let updater = self
            .app
            .updater()
            .map_err(|e| UpdateError::ConfigError(e.to_string()))?;

        // Check for updates again to get the latest update details
        let update = updater
            .check()
            .await
            .map_err(|e| UpdateError::UpdateRequestError(e.to_string()))?
            .ok_or(UpdateError::UpdateNotFound)?;

        // Verify that the version from the server matches the one we intend to install
        if update.version != version_info.version {
            return Err(UpdateError::VersionMismatch);
        }

        let app_handle = self.app.clone();
        let app_handle2 = app_handle.clone();
        let last_report_time = std::sync::Arc::new(std::sync::Mutex::new(Instant::now()));

        // Download and install the update
        update
            .download_and_install(
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

                                // Send progress as object with both percentage and bytes
                                let progress_data = serde_json::json!({
                                    "progress": progress / 100.0,  // 0.0 to 1.0 for frontend
                                    "current": current,
                                    "total": total
                                });
                                let _ = app_handle.emit(EVENT_UPDATE_PROGRESS, progress_data);
                            } else {
                                info!("Download progress: total size unknown ({} bytes)", current);
                                let progress_data = serde_json::json!({
                                    "progress": 0.0,
                                    "current": current,
                                    "total": 0
                                });
                                let _ = app_handle.emit(EVENT_UPDATE_PROGRESS, progress_data);
                            }
                        }
                        *last_time = now;
                    }
                },
                move || {
                    info!("Download completed, starting installation");
                    let _ = app_handle2.emit(EVENT_UPDATE_READY, ());
                },
            )
            .await
            .map_err(|e| {
                warn!("Download and install failed: {}", e);
                UpdateError::DownloadError(e.to_string())
            })?;

        info!("Update installation completed successfully");
        Ok(())
    }
}
