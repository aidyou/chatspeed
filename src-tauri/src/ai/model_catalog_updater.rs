use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, SystemTime},
};

use parking_lot::RwLock;
use serde_json::from_str;

use super::{
    model_catalog::{
        models_dev_catalog, models_dev_preset_providers, set_models_dev_catalog, CatalogError,
        ModelsDevCatalog, ModelsDevProviderList,
    },
    network::{ApiClient, ApiConfig, DefaultApiClient, ErrorFormat, ProxyType},
};

const SNAPSHOT_FILE: &str = "models-dev-catalog.json";
const PROVIDERS_SNAPSHOT_FILE: &str = "models-dev-providers.json";
const SUCCESS_FILE: &str = "models-dev-catalog.success";
const METADATA_BASE_URL: &str =
    "https://raw.githubusercontent.com/aidyou/chatspeed/refs/heads/updater-meta/models-dev";
const MAX_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;
const REFRESH_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone)]
pub struct ModelsDevCatalogService {
    index: Arc<RwLock<Arc<ModelsDevCatalog>>>,
    preset_providers: Arc<RwLock<Arc<ModelsDevProviderList>>>,
    snapshot_path: PathBuf,
    providers_snapshot_path: PathBuf,
    success_path: PathBuf,
    refresh_lock: Arc<AtomicBool>,
}

impl ModelsDevCatalogService {
    pub fn load(app_data_dir: impl AsRef<Path>) -> Result<Self, CatalogError> {
        let snapshot_path = app_data_dir.as_ref().join(SNAPSHOT_FILE);
        let providers_snapshot_path = app_data_dir.as_ref().join(PROVIDERS_SNAPSHOT_FILE);
        let index = match fs::read(&snapshot_path) {
            Ok(bytes) if bytes.len() <= MAX_SNAPSHOT_BYTES => {
                match catalog_from_slice(&bytes).and_then(|catalog| {
                    validate_catalog(&catalog)?;
                    Ok(catalog)
                }) {
                    Ok(catalog) => catalog,
                    Err(_) => models_dev_catalog()?.as_ref().clone(),
                }
            }
            _ => models_dev_catalog()?.as_ref().clone(),
        };
        let preset_providers = match fs::read(&providers_snapshot_path) {
            Ok(bytes) if bytes.len() <= MAX_SNAPSHOT_BYTES => match providers_from_slice(&bytes) {
                Ok(providers) => providers,
                Err(_) => models_dev_preset_providers()?,
            },
            _ => models_dev_preset_providers()?,
        };
        validate_catalog(&index)?;
        set_models_dev_catalog(index.clone())?;
        Ok(Self {
            index: Arc::new(RwLock::new(Arc::new(index))),
            preset_providers: Arc::new(RwLock::new(Arc::new(preset_providers))),
            providers_snapshot_path,
            success_path: app_data_dir.as_ref().join(SUCCESS_FILE),
            refresh_lock: Arc::new(AtomicBool::new(false)),
            snapshot_path,
        })
    }

    pub fn snapshot(&self) -> Arc<ModelsDevCatalog> {
        self.index.read().clone()
    }

    pub fn preset_providers(&self) -> Arc<ModelsDevProviderList> {
        self.preset_providers.read().clone()
    }

    pub fn is_fresh(&self) -> bool {
        fs::read_to_string(&self.success_path)
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .and_then(|seconds| SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(seconds)))
            .and_then(|timestamp| SystemTime::now().duration_since(timestamp).ok())
            .is_some_and(|age| age < REFRESH_INTERVAL)
    }

    pub async fn refresh(&self, proxy_type: ProxyType) -> Result<bool, String> {
        if self
            .refresh_lock
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return Err("catalog refresh already in progress".to_string());
        }
        let _release = RefreshGuard(self.refresh_lock.clone());
        if self.is_fresh() {
            return Ok(false);
        }
        let client = DefaultApiClient::new(ErrorFormat::Custom(Box::new(|_| None)));
        let config = ApiConfig::new(Some(METADATA_BASE_URL.to_string()), None, proxy_type, None);
        let response = client.get_request(&config, "catalog.json", None).await?;
        let providers_response = client.get_request(&config, "providers.json", None).await?;
        if response.content.len() > MAX_SNAPSHOT_BYTES {
            return Err("models.dev catalog exceeds the maximum supported size".to_string());
        }
        if providers_response.content.len() > MAX_SNAPSHOT_BYTES {
            return Err("models.dev providers exceeds the maximum supported size".to_string());
        }
        let preset_providers = providers_from_slice(providers_response.content.as_bytes())
            .map_err(|error| format!("failed to parse models.dev providers: {error}"))?;
        let catalog = catalog_from_slice(response.content.as_bytes())
            .map_err(|error| format!("failed to parse models.dev catalog: {error}"))?;
        validate_catalog(&catalog)
            .map_err(|error| format!("invalid models.dev catalog: {error}"))?;
        let parent = self
            .snapshot_path
            .parent()
            .ok_or_else(|| "catalog snapshot has no parent directory".to_string())?;
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let temporary = self.snapshot_path.with_extension("json.tmp");
        let providers_temporary = self.providers_snapshot_path.with_extension("json.tmp");
        let success_temporary = self.success_path.with_extension("tmp");
        let backup = self.snapshot_path.with_extension("json.backup");
        let providers_backup = self.providers_snapshot_path.with_extension("json.backup");
        let success_backup = self.success_path.with_extension("backup");
        let mut file = fs::File::create(&temporary).map_err(|error| error.to_string())?;
        use std::io::Write;
        file.write_all(response.content.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|error| error.to_string())?;
        let mut providers_file =
            fs::File::create(&providers_temporary).map_err(|error| error.to_string())?;
        providers_file
            .write_all(providers_response.content.as_bytes())
            .and_then(|_| providers_file.sync_all())
            .map_err(|error| error.to_string())?;
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(|error| error.to_string())?;
        let mut success_file =
            fs::File::create(&success_temporary).map_err(|error| error.to_string())?;
        success_file
            .write_all(now.as_secs().to_string().as_bytes())
            .and_then(|_| success_file.sync_all())
            .map_err(|error| error.to_string())?;

        let had_snapshot = self.snapshot_path.exists();
        let had_providers_snapshot = self.providers_snapshot_path.exists();
        let had_success = self.success_path.exists();
        let mut snapshot_backed_up = false;
        let mut providers_backed_up = false;
        let mut success_backed_up = false;
        let mut snapshot_installed = false;
        let mut providers_installed = false;
        let mut success_installed = false;
        let rollback = |snapshot_installed: bool,
                        providers_installed: bool,
                        success_installed: bool,
                        snapshot_backed_up: bool,
                        providers_backed_up: bool,
                        success_backed_up: bool|
         -> Result<(), String> {
            rollback_catalog_files(
                &self.snapshot_path,
                &self.providers_snapshot_path,
                &self.success_path,
                &backup,
                &providers_backup,
                &success_backup,
                snapshot_installed,
                providers_installed,
                success_installed,
                snapshot_backed_up,
                providers_backed_up,
                success_backed_up,
            )
        };
        if had_snapshot {
            let _ = fs::remove_file(&backup);
            fs::rename(&self.snapshot_path, &backup).map_err(|error| error.to_string())?;
            snapshot_backed_up = true;
        }
        if had_providers_snapshot {
            let _ = fs::remove_file(&providers_backup);
            if let Err(error) = fs::rename(&self.providers_snapshot_path, &providers_backup) {
                let rollback_error =
                    rollback(false, false, false, snapshot_backed_up, false, false).err();
                return Err(match rollback_error {
                    Some(rollback_error) => format!("{error}; {rollback_error}"),
                    None => error.to_string(),
                });
            }
            providers_backed_up = true;
        }
        if had_success {
            let _ = fs::remove_file(&success_backup);
            if let Err(error) = fs::rename(&self.success_path, &success_backup) {
                let rollback_error = rollback(
                    false,
                    false,
                    false,
                    snapshot_backed_up,
                    providers_backed_up,
                    false,
                )
                .err();
                return Err(match rollback_error {
                    Some(rollback_error) => format!("{error}; {rollback_error}"),
                    None => error.to_string(),
                });
            }
            success_backed_up = true;
        }
        let commit_result = (|| {
            fs::rename(&temporary, &self.snapshot_path).map_err(|error| error.to_string())?;
            snapshot_installed = true;
            fs::rename(&providers_temporary, &self.providers_snapshot_path)
                .map_err(|error| error.to_string())?;
            providers_installed = true;
            fs::rename(&success_temporary, &self.success_path)
                .map_err(|error| error.to_string())?;
            success_installed = true;
            if let Ok(directory) = fs::File::open(parent) {
                directory.sync_all().map_err(|error| error.to_string())?;
            }
            Ok::<(), String>(())
        })();
        if let Err(error) = commit_result {
            let _ = fs::remove_file(&temporary);
            let _ = fs::remove_file(&providers_temporary);
            let _ = fs::remove_file(&success_temporary);
            let rollback_error = rollback(
                snapshot_installed,
                providers_installed,
                success_installed,
                snapshot_backed_up,
                providers_backed_up,
                success_backed_up,
            )
            .err();
            return Err(match rollback_error {
                Some(rollback_error) => format!("{error}; {rollback_error}"),
                None => error,
            });
        }
        let _ = fs::remove_file(&backup);
        let _ = fs::remove_file(&providers_backup);
        let _ = fs::remove_file(&success_backup);
        *self.index.write() = Arc::new(catalog.clone());
        *self.preset_providers.write() = Arc::new(preset_providers);
        set_models_dev_catalog(catalog).map_err(|error| error.to_string())?;
        Ok(true)
    }
}

struct RefreshGuard(Arc<AtomicBool>);

impl Drop for RefreshGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn rollback_catalog_files(
    snapshot_path: &Path,
    providers_snapshot_path: &Path,
    success_path: &Path,
    snapshot_backup: &Path,
    providers_backup: &Path,
    success_backup: &Path,
    snapshot_installed: bool,
    providers_installed: bool,
    success_installed: bool,
    snapshot_backed_up: bool,
    providers_backed_up: bool,
    success_backed_up: bool,
) -> Result<(), String> {
    let mut errors = Vec::new();
    if snapshot_installed {
        if let Err(error) = fs::remove_file(snapshot_path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                errors.push(format!("failed to remove new catalog snapshot: {error}"));
            }
        }
    }
    if providers_installed {
        if let Err(error) = fs::remove_file(providers_snapshot_path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                errors.push(format!("failed to remove new providers snapshot: {error}"));
            }
        }
    }
    if success_installed {
        if let Err(error) = fs::remove_file(success_path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                errors.push(format!(
                    "failed to remove new catalog success marker: {error}"
                ));
            }
        }
    }
    if snapshot_backed_up {
        if let Err(error) = fs::rename(snapshot_backup, snapshot_path) {
            errors.push(format!("failed to restore catalog snapshot: {error}"));
        }
    }
    if providers_backed_up {
        if let Err(error) = fs::rename(providers_backup, providers_snapshot_path) {
            errors.push(format!("failed to restore providers snapshot: {error}"));
        }
    }
    if success_backed_up {
        if let Err(error) = fs::rename(success_backup, success_path) {
            errors.push(format!("failed to restore catalog success marker: {error}"));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn validate_catalog(catalog: &ModelsDevCatalog) -> Result<(), CatalogError> {
    if catalog.providers.is_empty()
        || catalog.models.is_empty()
        || !catalog
            .providers
            .values()
            .any(|provider| !provider.models.is_empty())
    {
        return Err(CatalogError::EmptyModelsDevCatalog);
    }
    Ok(())
}

fn catalog_from_slice(bytes: &[u8]) -> Result<ModelsDevCatalog, CatalogError> {
    from_str(std::str::from_utf8(bytes).map_err(|error| {
        CatalogError::Parse(serde_json::Error::io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            error.to_string(),
        )))
    })?)
    .map_err(CatalogError::Parse)
}

fn providers_from_slice(bytes: &[u8]) -> Result<ModelsDevProviderList, CatalogError> {
    from_str(std::str::from_utf8(bytes).map_err(|error| {
        CatalogError::Parse(serde_json::Error::io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            error.to_string(),
        )))
    })?)
    .map_err(CatalogError::Parse)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn rollback_preserves_original_success_marker_when_backup_fails() {
        let directory = tempdir().expect("temporary directory");
        let snapshot_path = directory.path().join(SNAPSHOT_FILE);
        let providers_snapshot_path = directory.path().join(PROVIDERS_SNAPSHOT_FILE);
        let success_path = directory.path().join(SUCCESS_FILE);
        let snapshot_backup = directory.path().join("snapshot.backup");
        let providers_backup = directory.path().join("providers.backup");
        let success_backup = directory.path().join("success.backup");
        let original_snapshot = br#"{"original":true}"#;
        let original_success = b"123";

        fs::write(&snapshot_backup, original_snapshot).expect("snapshot backup");
        fs::write(&success_path, original_success).expect("success marker");
        fs::create_dir(&success_backup).expect("unwritable backup target");
        let backup_error =
            fs::rename(&success_path, &success_backup).expect_err("backup must fail");
        assert_ne!(backup_error.kind(), std::io::ErrorKind::NotFound);

        rollback_catalog_files(
            &snapshot_path,
            &providers_snapshot_path,
            &success_path,
            &snapshot_backup,
            &providers_backup,
            &success_backup,
            false,
            false,
            false,
            true,
            false,
            false,
        )
        .expect("rollback");

        assert_eq!(
            fs::read(&snapshot_path).expect("restored snapshot"),
            original_snapshot
        );
        assert_eq!(
            fs::read(&success_path).expect("original success marker"),
            original_success
        );
    }

    #[test]
    fn invalid_app_snapshot_falls_back_to_bundled_catalog() {
        let directory = tempdir().expect("temporary directory");
        fs::write(directory.path().join(SNAPSHOT_FILE), b"invalid").expect("snapshot");
        let service = ModelsDevCatalogService::load(directory.path()).expect("catalog service");
        assert!(!service.snapshot().providers.is_empty());
    }

    #[test]
    fn success_timestamp_controls_freshness() {
        let directory = tempdir().expect("temporary directory");
        fs::write(
            directory.path().join(SUCCESS_FILE),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("clock")
                .as_secs()
                .to_string(),
        )
        .expect("success timestamp");
        let service = ModelsDevCatalogService::load(directory.path()).expect("catalog service");
        assert!(service.is_fresh());
    }
    #[test]
    fn bundled_catalog_is_used_when_snapshot_is_missing() {
        let directory = tempdir().expect("temporary directory");
        let service = ModelsDevCatalogService::load(directory.path()).expect("catalog service");
        assert!(!service.is_fresh());
        assert!(!service.snapshot().providers.is_empty());
    }
}
