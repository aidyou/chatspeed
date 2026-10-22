//! Update manager implementation
//!
//! Provides functionality for checking and installing application updates.

use super::error::{Result, UpdateError};
use super::types::VersionInfo;
use crate::runtime_client::{RuntimeStopOutcome, RuntimeSupervisor};
use chatspeed_runtime_backend::error_diagnostics::{ErrorOrigin, FailureMessage};
use log::{info, warn};
use reqwest::Proxy;
use semver::Version;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::async_runtime::spawn;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

const UPDATE_RUNTIME_STOP_BUDGET: std::time::Duration = std::time::Duration::from_secs(30);

const EVENT_UPDATE_PROGRESS: &str = "update://download-progress";
const EVENT_UPDATE_READY: &str = "update://ready";
const EVENT_UPDATE_AVAILABLE: &str = "update://available";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateCheckOutcome {
    NoUpdate,
    Started,
    InProgress,
    ReadyToInstall,
}

#[derive(Clone)]
struct UpdateState {
    update: Update,
    tmp_path: PathBuf,
}

/// Manages the update process
pub struct UpdateManager {
    app: AppHandle,
    latest_update: Arc<Mutex<Option<UpdateState>>>,
    check_in_progress: Arc<Mutex<bool>>,
    install_in_progress: Arc<tokio::sync::Mutex<()>>,
}

impl UpdateManager {
    /// Creates a new UpdateManager instance
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            latest_update: Arc::new(Mutex::new(None)),
            check_in_progress: Arc::new(Mutex::new(false)),
            install_in_progress: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// Checks for available updates and automatically downloads them in the background.
    pub async fn check_and_download_update(&self) -> Result<UpdateCheckOutcome> {
        if self.has_ready_update()? {
            return Ok(UpdateCheckOutcome::ReadyToInstall);
        }

        if !self.try_mark_check_started()? {
            return Ok(UpdateCheckOutcome::InProgress);
        }

        let result = async {
            let updater = self.build_updater().await.map_err(|e| {
                UpdateError::ConfigError(
                    FailureMessage::leaf(e.to_string()).with_origin(e.error_origin()),
                )
            })?;

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
                    let check_in_progress_clone = self.check_in_progress.clone();
                    spawn(async move {
                        if let Err(e) = download_update_to_file(
                            app_clone.clone(),
                            latest_update_clone,
                            check_in_progress_clone.clone(),
                            update,
                        )
                        .await
                        {
                            warn!("Background download failed: {}", e);
                            let _ = app_clone.emit("update://download-failed", e.to_string());
                            set_check_in_progress(&check_in_progress_clone, false);
                        }
                    });
                    return Ok(UpdateCheckOutcome::Started);
                }
            }

            Ok(UpdateCheckOutcome::NoUpdate)
        }
        .await;

        if !matches!(result, Ok(UpdateCheckOutcome::Started)) {
            self.set_check_in_progress(false)?;
        }

        result
    }

    /// Builds the updater, applying the proxy the runtime configuration describes.
    ///
    /// The proxy is read from the runtime over the control plane, never a local
    /// database. Before the runtime publishes a snapshot the builder keeps its
    /// default, so the updater follows system/env proxy settings instead of
    /// inventing a value.
    async fn build_updater(&self) -> Result<tauri_plugin_updater::Updater> {
        let mut builder = self.app.updater_builder();

        if let Some(snapshot) = crate::runtime_config::current_or_load(&self.app).await {
            use crate::runtime_config::{
                CFG_PROXY_PASSWORD, CFG_PROXY_SERVER, CFG_PROXY_TYPE, CFG_PROXY_USERNAME,
            };

            let proxy_type = snapshot.get_string(CFG_PROXY_TYPE, "none");
            match proxy_type.as_str() {
                "none" => {
                    builder = builder.no_proxy();
                }
                "http" => {
                    let proxy_server = snapshot.get_string(CFG_PROXY_SERVER, "");
                    if proxy_server.trim().is_empty() {
                        return Err(UpdateError::ConfigError(
                            "HTTP proxy is enabled for updates, but proxy_server is empty".into(),
                        ));
                    }

                    let mut proxy = Proxy::all(proxy_server.as_str())
                        .map_err(|e| UpdateError::ConfigError(FailureMessage::from_error(&e)))?;
                    let proxy_username = snapshot.get_string(CFG_PROXY_USERNAME, "");
                    let proxy_password = snapshot.get_string(CFG_PROXY_PASSWORD, "");

                    if !proxy_username.is_empty() && !proxy_password.is_empty() {
                        proxy = proxy.basic_auth(&proxy_username, &proxy_password);
                    }

                    builder = builder.configure_client(move |client| client.proxy(proxy.clone()));
                }
                _ => {
                    // Keep reqwest default behavior so updater follows system/env proxy settings.
                }
            }
        }

        builder
            .build()
            .map_err(|e| UpdateError::ConfigError(FailureMessage::from_error(&e)))
    }

    /// Installs the latest downloaded update and restarts the application.
    pub async fn install_and_restart(&self) -> Result<()> {
        let _install = self.install_in_progress.try_lock().map_err(|_| {
            UpdateError::InstallError(FailureMessage::leaf(
                "An update installation is already in progress",
            ))
        })?;
        let update_state = self
            .latest_update
            .lock()
            .map_err(|e| {
                UpdateError::LockError(
                    FailureMessage::leaf("Failed to lock latest_update mutex")
                        .with_origin(ErrorOrigin::leaf(e.to_string(), None, None, None)),
                )
            })?
            .clone();

        if let Some(state) = update_state {
            info!("Installing update and restarting...");
            let bytes = tokio::fs::read(&state.tmp_path).await?;
            // An installed desktop must never attach to the old sidecar after
            // restart. Confirm its exact process has exited before replacing the
            // bundled runtime binary.
            let supervisor = self
                .app
                .try_state::<Arc<RuntimeSupervisor>>()
                .ok_or_else(|| {
                    UpdateError::InstallError(FailureMessage::leaf(
                        "Runtime supervisor is unavailable",
                    ))
                })?;
            supervisor.begin_stopping().await;
            let stop_result = match supervisor.runtime_identity().await {
                Ok(identity) => {
                    let report = supervisor
                        .request_forced_runtime_shutdown(
                            &identity.instance_id,
                            identity.pid,
                            &format!("update-stop-{}", identity.instance_id),
                            UPDATE_RUNTIME_STOP_BUDGET,
                        )
                        .await;
                    if report.outcome != RuntimeStopOutcome::Confirmed {
                        Err(UpdateError::InstallError(FailureMessage::leaf(format!(
                            "Runtime shutdown was not confirmed before updating: {}",
                            report.outcome.code()
                        ))))
                    } else {
                        Ok(())
                    }
                }
                Err(error) => {
                    info!("Runtime identity is unavailable before update: {error}; checking the owner lock");
                    Ok(())
                }
            };
            if let Err(error) = stop_result {
                supervisor.resume_after_failed_update().await;
                return Err(error);
            }
            let config = match crate::runtime_client::default_launch_config() {
                Ok(config) => config,
                Err(cause) => {
                    supervisor.resume_after_failed_update().await;
                    return Err(UpdateError::InstallError(FailureMessage::leaf(format!(
                        "Cannot resolve runtime before updating: {cause}"
                    ))));
                }
            };
            let _runtime_lock =
                match chatspeed_runtime_client::lock_idle_runtime_for_update(&config) {
                    Ok(lock) => lock,
                    Err(cause) => {
                        supervisor.resume_after_failed_update().await;
                        return Err(UpdateError::InstallError(FailureMessage::leaf(format!(
                            "Cannot confirm runtime exit before updating: {cause}"
                        ))));
                    }
                };
            if let Err(error) = state.update.install(&bytes) {
                drop(_runtime_lock);
                supervisor.resume_after_failed_update().await;
                return Err(UpdateError::InstallError(FailureMessage::from_error(
                    &error,
                )));
            }
            self.latest_update
                .lock()
                .map_err(|_| {
                    UpdateError::LockError(FailureMessage::leaf(
                        "Failed to lock latest_update mutex",
                    ))
                })?
                .take();
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

        let new_version =
            Version::parse(new_version_str).map_err(|e| UpdateError::VersionParseError {
                version: new_version_str.to_string(),
                error: FailureMessage::from_error(&e),
            })?;
        let current_version =
            Version::parse(&current_version_str).map_err(|e| UpdateError::VersionParseError {
                version: current_version_str,
                error: FailureMessage::from_error(&e),
            })?;

        Ok(new_version > current_version)
    }

    fn has_ready_update(&self) -> Result<bool> {
        let latest_update = self.latest_update.lock().map_err(|e| {
            UpdateError::LockError(
                FailureMessage::leaf("Failed to lock latest_update mutex")
                    .with_origin(ErrorOrigin::leaf(e.to_string(), None, None, None)),
            )
        })?;
        Ok(latest_update.is_some())
    }

    fn try_mark_check_started(&self) -> Result<bool> {
        let mut check_in_progress = self.check_in_progress.lock().map_err(|e| {
            UpdateError::LockError(
                FailureMessage::leaf("Failed to lock check_in_progress mutex")
                    .with_origin(ErrorOrigin::leaf(e.to_string(), None, None, None)),
            )
        })?;

        if *check_in_progress {
            return Ok(false);
        }

        *check_in_progress = true;
        Ok(true)
    }

    fn set_check_in_progress(&self, value: bool) -> Result<()> {
        let mut check_in_progress = self.check_in_progress.lock().map_err(|e| {
            UpdateError::LockError(
                FailureMessage::leaf("Failed to lock check_in_progress mutex")
                    .with_origin(ErrorOrigin::leaf(e.to_string(), None, None, None)),
            )
        })?;
        *check_in_progress = value;
        Ok(())
    }
}

fn set_check_in_progress(check_in_progress: &Arc<Mutex<bool>>, value: bool) {
    if let Ok(mut state) = check_in_progress.lock() {
        *state = value;
    }
}

async fn download_update_to_file(
    app: AppHandle,
    latest_update: Arc<Mutex<Option<UpdateState>>>,
    check_in_progress: Arc<Mutex<bool>>,
    update: Update,
) -> Result<()> {
    info!(
        "Starting background download for version: {}",
        update.version
    );

    let downloaded_bytes = Arc::new(Mutex::new(0_u64));
    let downloaded_bytes_clone = downloaded_bytes.clone();
    let progress_app = app.clone();
    let on_chunk = move |chunk_size, total| {
        let progress = if let Some(total) = total {
            if let Ok(mut downloaded) = downloaded_bytes_clone.lock() {
                *downloaded = (*downloaded + chunk_size as u64).min(total);
                if total == 0 {
                    0.0
                } else {
                    ((*downloaded as f64 / total as f64) * 99.0).min(99.0)
                }
            } else {
                0.0
            }
        } else {
            0.0
        };
        let _ = progress_app.emit(
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
        .map_err(|e| UpdateError::DownloadError(FailureMessage::from_error(&e)))?;

    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| UpdateError::IoError(FailureMessage::from_error(&e)))?;
    if !cache_dir.exists() {
        tokio::fs::create_dir_all(&cache_dir).await?;
    }
    let tmp_path = cache_dir.join(format!("update-{}.bin", update.version));

    tokio::fs::write(&tmp_path, &bytes).await?;

    let mut lu = latest_update.lock().map_err(|e| {
        UpdateError::LockError(
            FailureMessage::leaf("Failed to lock latest_update mutex")
                .with_origin(ErrorOrigin::leaf(e.to_string(), None, None, None)),
        )
    })?;
    *lu = Some(UpdateState { update, tmp_path });

    let _ = app.emit(EVENT_UPDATE_READY, ());
    set_check_in_progress(&check_in_progress, false);

    Ok(())
}
