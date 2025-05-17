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
use rust_i18n::t;
use semver::Version;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

const EVENT_UPDATE_PROGRESS: &str = "update://download-progress";
const EVENT_UPDATE_READY: &str = "update://ready";
const EVENT_UPDATE_AVAILABLE: &str = "update://available";

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
        let current_version = self.app.package_info().version.clone().to_string();

        let new_version = Version::parse(&version_info.version).map_err(|e| {
            UpdateError::VersionCheckError(
                t!("updater.invalid_version_format", error = e.to_string()).to_string(),
            )
        })?;
        let current = Version::parse(&current_version).map_err(|e| {
            UpdateError::VersionCheckError(
                t!(
                    "updater.invalid_current_version_format",
                    error = e.to_string()
                )
                .to_string(),
            )
        })?;

        Ok(new_version > current)
    }

    /// Checks for available updates
    ///
    /// Returns Some(VersionInfo) if an update is available, None if no update is available.
    /// The version info includes the new version number, download URL, and release notes.
    pub async fn check_update(&self) -> Result<Option<VersionInfo>> {
        let updater = self.app.updater().map_err(|e| {
            UpdateError::CheckError(
                t!("updater.updater_init_failed", error = e.to_string()).to_string(),
            )
        })?;

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
                Err(UpdateError::CheckError(
                    t!("updater.check_update_request_failed", error = e.to_string()).to_string(),
                ))
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
        let updater = self.app.updater().map_err(|e| {
            UpdateError::CheckError(
                t!("updater.updater_init_failed", error = e.to_string()).to_string(),
            )
        })?;

        // 检查更新
        let update = updater
            .check()
            .await
            .map_err(|e| {
                UpdateError::CheckError(
                    t!("updater.check_update_request_failed", error = e.to_string()).to_string(),
                )
            })?
            .ok_or_else(|| UpdateError::UpdateNotFound)?;

        // 验证版本信息
        if update.version.to_string() != version_info.version {
            return Err(UpdateError::VersionMismatch);
        }

        let app_handle = self.app.clone();
        let app_handle2 = app_handle.clone();

        // 下载并安装更新
        update
            .download_and_install(
                move |current, total| {
                    if let Some(total) = total {
                        let progress = (current as f64 / total as f64) * 100.0;
                        if progress.floor() % 10.0 == 0.0 {
                            info!("Download progress: {:.0}%", progress);
                            let _ =
                                app_handle.emit(EVENT_UPDATE_PROGRESS, format!("{:.0}", progress));
                        }
                    }
                },
                move || {
                    info!("Download completed, starting installation");
                    let _ = app_handle2.emit(EVENT_UPDATE_READY, ());
                },
            )
            .await
            .map_err(|e| {
                UpdateError::DownloadError(
                    t!("updater.download_install_failed", error = e.to_string()).to_string(),
                )
            })?;

        info!("Update installation completed");
        Ok(())
    }
}
