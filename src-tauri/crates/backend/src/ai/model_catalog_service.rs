//! Runtime-owned Models.dev catalog service.
//!
//! The Models.dev catalog snapshot is runtime-owned state: the runtime loads it
//! from its own application-data directory, serves it to the desktop over the
//! allowlisted data commands and refreshes it in the background through its own
//! outbound proxy. The desktop no longer constructs or manages this service; it
//! reaches the same owner purely through the control plane.
//!
//! This module is a real owner module of the desktop-free backend rather than a
//! `#[path]` include of desktop source, so the runtime closure does not link any
//! desktop implementation. The Models.dev snapshot DTOs and the parsing and
//! resolution engine live in `crate::model_catalog_engine`; only the snapshot
//! owner lives here.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
    time::{Duration, Instant, SystemTime},
};

use parking_lot::RwLock;
use serde_json::from_str;

use crate::ai::network::{ApiClient, ApiConfig, DefaultApiClient, ErrorFormat, ProxyType};
use crate::db::MainStore;
use crate::model_catalog_engine::{
    models_dev_catalog, models_dev_preset_providers, set_models_dev_catalog, CatalogError,
    ModelsDevCatalog, ModelsDevProviderList,
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
    state: Arc<OnceLock<CatalogState>>,
    snapshot_path: PathBuf,
    providers_snapshot_path: PathBuf,
    success_path: PathBuf,
    refresh_lock: Arc<AtomicBool>,
}

struct CatalogState {
    index: RwLock<Arc<ModelsDevCatalog>>,
    preset_providers: RwLock<Arc<ModelsDevProviderList>>,
}

impl ModelsDevCatalogService {
    /// Defers template data without delaying the independent transport rules.
    pub fn deferred(app_data_dir: impl AsRef<Path>) -> Self {
        let directory = app_data_dir.as_ref();
        Self {
            state: Arc::new(OnceLock::new()),
            snapshot_path: directory.join(SNAPSHOT_FILE),
            providers_snapshot_path: directory.join(PROVIDERS_SNAPSHOT_FILE),
            success_path: directory.join(SUCCESS_FILE),
            refresh_lock: Arc::new(AtomicBool::new(false)),
        }
    }

    fn state(&self) -> &CatalogState {
        self.state_with_publisher(set_models_dev_catalog)
    }

    fn state_with_publisher(
        &self,
        publish: impl FnOnce(Arc<ModelsDevCatalog>) -> Result<(), CatalogError>,
    ) -> &CatalogState {
        self.state.get_or_init(|| {
            match self.read_state(publish) {
                Ok(state) => state,
                Err(error) => {
                    log::error!(
                        "Failed to load the Models.dev catalog snapshot: {error}; using the embedded catalog"
                    );
                    Self::embedded_state()
                }
            }
        })
    }

    /// Warms the same single-flight state used by catalog-dependent commands.
    pub fn prewarm(&self) {
        self.state();
    }

    pub fn load(app_data_dir: impl AsRef<Path>) -> Result<Self, CatalogError> {
        Self::load_with_publisher(app_data_dir, set_models_dev_catalog)
    }

    fn load_with_publisher(
        app_data_dir: impl AsRef<Path>,
        publish: impl FnOnce(Arc<ModelsDevCatalog>) -> Result<(), CatalogError>,
    ) -> Result<Self, CatalogError> {
        let mut service = Self::deferred(app_data_dir);
        service.state = Arc::new(OnceLock::from(service.read_state(publish)?));
        Ok(service)
    }

    fn read_state(
        &self,
        publish: impl FnOnce(Arc<ModelsDevCatalog>) -> Result<(), CatalogError>,
    ) -> Result<CatalogState, CatalogError> {
        let started = Instant::now();
        let read_started = Instant::now();
        let snapshot = fs::read(&self.snapshot_path);
        log::info!(
            "Models.dev startup catalog read: {:?}",
            read_started.elapsed()
        );
        let parse_started = Instant::now();
        let index = match snapshot {
            Ok(bytes) if bytes.len() <= MAX_SNAPSHOT_BYTES => {
                match catalog_from_slice(&bytes).and_then(|catalog| {
                    validate_catalog(&catalog)?;
                    Ok(catalog)
                }) {
                    Ok(catalog) => Arc::new(catalog),
                    Err(_) => models_dev_catalog()?,
                }
            }
            _ => models_dev_catalog()?,
        };
        log::info!(
            "Models.dev startup catalog parse/fallback: {:?}",
            parse_started.elapsed()
        );
        let read_started = Instant::now();
        let providers_snapshot = fs::read(&self.providers_snapshot_path);
        log::info!(
            "Models.dev startup providers read: {:?}",
            read_started.elapsed()
        );
        let parse_started = Instant::now();
        let preset_providers = match providers_snapshot {
            Ok(bytes) if bytes.len() <= MAX_SNAPSHOT_BYTES => match providers_from_slice(&bytes) {
                Ok(providers) => providers,
                Err(_) => models_dev_preset_providers()?,
            },
            _ => models_dev_preset_providers()?,
        };
        log::info!(
            "Models.dev startup providers parse/fallback: {:?}",
            parse_started.elapsed()
        );
        validate_catalog(&index)?;
        publish(index.clone())?;
        log::info!("Models.dev startup load total: {:?}", started.elapsed());
        Ok(CatalogState {
            index: RwLock::new(index),
            preset_providers: RwLock::new(Arc::new(preset_providers)),
        })
    }

    /// Eagerly loads the catalog for callers that need compatibility with the
    /// previous startup behavior. Production assembly uses [`Self::deferred`]
    /// so the background refresh task can warm the same shared state later.
    pub fn load_or_embedded(app_data_dir: impl AsRef<Path>) -> Self {
        let service = Self::deferred(app_data_dir);
        service.prewarm();
        service
    }

    /// Builds a state backed only by the bundled catalog.
    fn embedded_state() -> CatalogState {
        let index = models_dev_catalog().unwrap_or_else(|error| {
            log::error!("Embedded Models.dev catalog is unusable: {error}");
            Arc::new(ModelsDevCatalog {
                models: HashMap::new(),
                providers: HashMap::new(),
            })
        });
        let preset_providers = models_dev_preset_providers().unwrap_or_else(|error| {
            log::error!("Embedded Models.dev provider presets are unusable: {error}");
            ModelsDevProviderList {
                providers: Vec::new(),
            }
        });
        let _ = set_models_dev_catalog(index.clone());
        CatalogState {
            index: RwLock::new(index),
            preset_providers: RwLock::new(Arc::new(preset_providers)),
        }
    }

    pub fn snapshot(&self) -> Arc<ModelsDevCatalog> {
        self.state().index.read().clone()
    }

    pub fn preset_providers(&self) -> Arc<ModelsDevProviderList> {
        self.state().preset_providers.read().clone()
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
        // Initialization must precede refresh publication, including fresh snapshots.
        let service = self.clone();
        tokio::task::spawn_blocking(move || service.prewarm())
            .await
            .map_err(|error| format!("catalog initialization failed: {error}"))?;
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
        let catalog = Arc::new(catalog);
        *self.state().index.write() = catalog.clone();
        *self.state().preset_providers.write() = Arc::new(preset_providers);
        set_models_dev_catalog(catalog).map_err(|error| error.to_string())?;
        Ok(true)
    }
}

/// Resolves the outbound proxy the catalog refresh should use from the runtime
/// store, so the refresh honours the same proxy configuration the desktop reads.
pub fn proxy_type_for_store(main_store: &MainStore) -> ProxyType {
    match main_store
        .get_config("proxy_type", "none".to_string())
        .as_str()
    {
        "http" => ProxyType::Http(
            main_store.get_config("proxy_server", String::new()),
            Some(main_store.get_config("proxy_username", String::new())),
            Some(main_store.get_config("proxy_password", String::new())),
        ),
        "system" => ProxyType::System,
        _ => ProxyType::None,
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

    fn write_template_fixture(directory: &Path) {
        fs::write(
            directory.join(SNAPSHOT_FILE),
            br#"{"models":{"test-model":{"id":"test-model","name":"Template","reasoning":true,"limit":{"context":32768},"vendor_option":{"enabled":true}}},"providers":{"test-provider":{"id":"test-provider","name":"Provider","models":{"test-model":{"id":"test-model","name":"Template","cost":{"input":2.5},"vendor_option":{"enabled":true}}}}}}"#,
        ).expect("catalog fixture");
        fs::write(
            directory.join(PROVIDERS_SNAPSHOT_FILE),
            br#"{"providers":[]}"#,
        )
        .expect("provider fixture");
    }

    #[test]
    fn deferred_catalog_keeps_transport_available_without_loading_templates() {
        let directory = tempdir().expect("temporary directory");
        let service = ModelsDevCatalogService::deferred(directory.path());
        assert!(service.state.get().is_none());
        let transport = crate::model_catalog_engine::resolve_transport(
            "gpt-5",
            Some("https://api.openai.com/v1"),
            Some("openai"),
            None,
        )
        .expect("transport rules");
        assert!(transport.is_some());
        assert!(service.state.get().is_none());
        // A snapshot installed after construction must be read on first use.
        write_template_fixture(directory.path());
        let state = service.state_with_publisher(|catalog| {
            assert_eq!(
                catalog.models["test-model"].limit.as_ref().unwrap().context,
                Some(32768)
            );
            Ok(())
        });
        let snapshot = state.index.read().clone();
        assert_eq!(
            snapshot.models["test-model"].extra["vendor_option"]["enabled"],
            true
        );
        assert_eq!(
            snapshot.providers["test-provider"].models["test-model"]
                .cost
                .as_ref()
                .unwrap()
                .input,
            Some(2.5)
        );
        assert!(Arc::ptr_eq(&snapshot, &service.snapshot()));
        assert!(service.preset_providers().providers.is_empty());
    }

    #[test]
    fn concurrent_first_use_publishes_once_and_shares_allocation() {
        let directory = tempdir().expect("temporary directory");
        write_template_fixture(directory.path());
        let service = ModelsDevCatalogService::deferred(directory.path());
        let publications = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        let state = service.state_with_publisher(|catalog| {
                            publications.fetch_add(1, Ordering::SeqCst);
                            assert!(catalog.models.contains_key("test-model"));
                            Ok(())
                        });
                        state.index.read().clone()
                    })
                })
                .collect();
            let snapshots: Vec<_> = handles
                .into_iter()
                .map(|handle| handle.join().expect("reader"))
                .collect();
            assert!(snapshots
                .iter()
                .all(|snapshot| Arc::ptr_eq(&snapshots[0], snapshot)));
        });
        assert_eq!(publications.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn local_load_shares_catalog_with_publisher_and_falls_back_for_bad_providers() {
        let directory = tempdir().expect("temporary directory");
        write_template_fixture(directory.path());
        fs::write(directory.path().join(PROVIDERS_SNAPSHOT_FILE), b"invalid")
            .expect("bad providers");
        let mut published = None;
        let service = ModelsDevCatalogService::load_with_publisher(directory.path(), |catalog| {
            published = Some(catalog);
            Ok(())
        })
        .expect("catalog");
        assert!(Arc::ptr_eq(
            &published.expect("published"),
            &service.snapshot()
        ));
        assert!(!service.preset_providers().providers.is_empty());
    }

    #[tokio::test]
    async fn refresh_initializes_even_when_fresh_without_network() {
        let directory = tempdir().expect("temporary directory");
        fs::write(
            directory.path().join(SUCCESS_FILE),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("clock")
                .as_secs()
                .to_string(),
        )
        .expect("fresh marker");
        let service = ModelsDevCatalogService::deferred(directory.path());
        assert!(!service
            .refresh(ProxyType::None)
            .await
            .expect("fresh refresh"));
        assert!(service.state.get().is_some());
        let snapshot = service.snapshot();
        service.prewarm();
        assert!(Arc::ptr_eq(&snapshot, &service.snapshot()));
        assert!(!snapshot.models.is_empty());
    }

    #[test]
    #[ignore = "manual startup profiling"]
    fn profile_catalog_startup_phases() {
        use std::time::Instant;
        let temporary = tempdir().expect("temporary directory");
        let directory = std::env::var_os("CHATSPEED_CATALOG_PROFILE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| temporary.path().to_path_buf());
        let started = Instant::now();
        let catalog_path = directory.join(SNAPSHOT_FILE);
        let providers_path = directory.join(PROVIDERS_SNAPSHOT_FILE);
        let catalog_bytes = match fs::read(&catalog_path) {
            Ok(bytes) => bytes,
            Err(_) => serde_json::to_vec(models_dev_catalog().expect("embedded catalog").as_ref())
                .expect("serialize embedded catalog"),
        };
        let read_catalog = started.elapsed();
        let catalog_started = Instant::now();
        let catalog = catalog_from_slice(&catalog_bytes).expect("catalog parse");
        let catalog_parse = catalog_started.elapsed();
        let providers_started = Instant::now();
        let providers_bytes = match fs::read(&providers_path) {
            Ok(bytes) => bytes,
            Err(_) => {
                serde_json::to_vec(&models_dev_preset_providers().expect("embedded providers"))
                    .expect("serialize embedded providers")
            }
        };
        let providers_read = providers_started.elapsed();
        let providers_parse_started = Instant::now();
        let providers = providers_from_slice(&providers_bytes).expect("providers parse");
        let providers_parse = providers_parse_started.elapsed();
        let clone_started = Instant::now();
        let _catalog_clone = std::hint::black_box(catalog.clone());
        let deep_clone = clone_started.elapsed();
        let catalog_arc = Arc::new(catalog);
        let arc_started = Instant::now();
        let _arc_clone = std::hint::black_box(Arc::clone(&catalog_arc));
        let arc_clone = arc_started.elapsed();
        let _ = providers;
        println!(
            "catalog phases read={read_catalog:?} catalog_parse={catalog_parse:?} providers_read={providers_read:?} providers_parse={providers_parse:?} deep_clone={deep_clone:?} arc_clone={arc_clone:?} total={:?}",
            started.elapsed()
        );
        let deferred_started = Instant::now();
        let deferred = ModelsDevCatalogService::deferred(&directory);
        println!(
            "catalog deferred_construct={:?}",
            deferred_started.elapsed()
        );
        assert!(deferred.state.get().is_none());
        let load_started = Instant::now();
        deferred.prewarm();
        let service = deferred;
        println!(
            "catalog service_load={:?} models={} providers={}",
            load_started.elapsed(),
            service.snapshot().models.len(),
            service.preset_providers().providers.len()
        );
    }

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
