//! Desktop-side reads and writes of runtime-owned configuration.
//!
//! The desktop no longer opens the runtime database, so the startup
//! configuration and the window geometry it needs have to come from the runtime
//! control plane instead of a local `MainStore`. This adapter resolves the two
//! allowlisted configuration commands — `get_all_config` and `set_config` — into
//! a small typed snapshot, plus the exact geometry writes the window layer
//! needs. It is deliberately narrow: there is no generic method/path passthrough
//! and no second configuration cache that could diverge from the runtime.
//!
//! [`RuntimeConfigCache`] keeps the last loaded snapshot so the synchronous
//! window event handlers can read saved geometry without blocking on I/O. The
//! cache is written from a successful connection and refreshed by the async
//! readers ([`current_or_load`]) that need the latest values; a configuration
//! write goes straight to the runtime and never mutates the cached snapshot, so
//! a later read still reflects the runtime's own value.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use serde::de::DeserializeOwned;
use serde_json::Value;
use tauri::{AppHandle, Manager};

#[cfg(test)]
use crate::ai::network::types::ProxyType;
use crate::constants::{
    CFG_ASSISTANT_WINDOW_SIZE, CFG_PROXY_SWITCHER_WINDOW_SIZE, CFG_SCRAPER_DEBUG_MODE,
    CFG_WINDOW_POSITION, CFG_WINDOW_SIZE, CFG_WORKFLOW_WINDOW_POSITION, CFG_WORKFLOW_WINDOW_SIZE,
};
use crate::runtime_client::RuntimeSupervisor;
use crate::window::{MainWindowPosition, WindowRestoreConfig, WindowSize};

/// Configuration keys the desktop reads for its startup side effects.
pub const CFG_PROXY_TYPE: &str = "proxy_type";
/// HTTP proxy server address.
pub const CFG_PROXY_SERVER: &str = "proxy_server";
/// HTTP proxy user name.
pub const CFG_PROXY_USERNAME: &str = "proxy_username";
/// HTTP proxy password.
pub const CFG_PROXY_PASSWORD: &str = "proxy_password";

/// Returns the runtime configuration key a window label stores its size under.
///
/// Labels without a remembered size return `None`, matching the previous
/// `MainStore` behaviour of skipping unknown windows.
pub fn window_size_key(window_label: &str) -> Option<&'static str> {
    match window_label {
        "main" => Some(CFG_WINDOW_SIZE),
        "assistant" => Some(CFG_ASSISTANT_WINDOW_SIZE),
        "workflow" => Some(CFG_WORKFLOW_WINDOW_SIZE),
        "proxy_switcher" => Some(CFG_PROXY_SWITCHER_WINDOW_SIZE),
        _ => None,
    }
}

/// A read-only view of the runtime configuration map.
///
/// Only the keys the desktop actually needs are decoded; everything else stays
/// an opaque `Value` so an unknown future key can never fail the snapshot.
#[derive(Debug, Clone, Default)]
pub struct RuntimeConfigSnapshot {
    settings: HashMap<String, Value>,
}

impl RuntimeConfigSnapshot {
    /// Wraps the raw configuration map returned by `get_all_config`.
    pub fn from_settings(settings: HashMap<String, Value>) -> Self {
        Self { settings }
    }

    /// Deserializes one key, returning `None` when it is absent or malformed.
    fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let value = self.settings.get(key)?;
        match serde_json::from_value(value.clone()) {
            Ok(parsed) => Some(parsed),
            Err(error) => {
                log::warn!("Ignoring malformed runtime config key '{key}': {error}");
                None
            }
        }
    }

    /// Reads a boolean setting, falling back to `default`.
    pub fn get_bool(&self, key: &str, default: bool) -> bool {
        self.get(key).unwrap_or(default)
    }

    /// Reads a string setting, falling back to `default`.
    pub fn get_string(&self, key: &str, default: &str) -> String {
        self.get::<String>(key)
            .unwrap_or_else(|| default.to_string())
    }

    /// Whether scraped pages are kept visible for inspection.
    ///
    /// A local display preference only: when the runtime has not published a
    /// snapshot this stays off rather than falling back to a local store.
    pub fn scraper_debug_mode(&self) -> bool {
        self.get_bool(CFG_SCRAPER_DEBUG_MODE, false)
    }

    /// The remembered size for a window label, when one is stored.
    pub fn window_size(&self, window_label: &str) -> Option<WindowSize> {
        self.get(window_size_key(window_label)?)
    }

    /// The remembered position for the main window.
    pub fn main_window_position(&self) -> MainWindowPosition {
        self.get(CFG_WINDOW_POSITION).unwrap_or_default()
    }

    /// The remembered position for the workflow window.
    pub fn workflow_window_position(&self) -> MainWindowPosition {
        self.get(CFG_WORKFLOW_WINDOW_POSITION).unwrap_or_default()
    }

    /// Builds the geometry a window label is restored from.
    ///
    /// Main and workflow restore a position as well as a size; the assistant and
    /// proxy switcher windows only remember a size, exactly as before.
    pub fn restore_config(&self, window_label: &str) -> WindowRestoreConfig {
        match window_label {
            "main" => WindowRestoreConfig {
                size: self.window_size(window_label),
                position: Some(self.main_window_position()),
            },
            "workflow" => WindowRestoreConfig {
                size: self.window_size(window_label),
                position: Some(self.workflow_window_position()),
            },
            "assistant" | "proxy_switcher" => WindowRestoreConfig {
                size: self.window_size(window_label),
                position: None,
            },
            _ => WindowRestoreConfig::default(),
        }
    }

    /// Resolves the outbound proxy the model catalog refresh should use.
    #[cfg(test)]
    pub fn proxy_type(&self) -> ProxyType {
        match self.get_string(CFG_PROXY_TYPE, "none").as_str() {
            "http" => ProxyType::Http(
                self.get_string(CFG_PROXY_SERVER, ""),
                Some(self.get_string(CFG_PROXY_USERNAME, "")),
                Some(self.get_string(CFG_PROXY_PASSWORD, "")),
            ),
            "system" => ProxyType::System,
            _ => ProxyType::None,
        }
    }
}

/// Holds the most recent runtime configuration snapshot for synchronous readers.
///
/// Written from a successful supervisor connection and from each async refresh,
/// then read by the window event handlers, which cannot await. The lock is never
/// held across an await.
#[derive(Default)]
pub struct RuntimeConfigCache {
    snapshot: RwLock<Option<Arc<RuntimeConfigSnapshot>>>,
}

impl RuntimeConfigCache {
    /// Creates an empty cache; the snapshot arrives once the runtime connects.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the cached snapshot with a freshly loaded one.
    pub fn store(&self, snapshot: RuntimeConfigSnapshot) {
        match self.snapshot.write() {
            Ok(mut guard) => *guard = Some(Arc::new(snapshot)),
            Err(_) => {
                log::error!("Runtime config cache lock is poisoned; keeping the previous snapshot")
            }
        }
    }

    /// Returns the cached snapshot, or `None` before the runtime has connected.
    pub fn current(&self) -> Option<Arc<RuntimeConfigSnapshot>> {
        match self.snapshot.read() {
            Ok(guard) => guard.clone(),
            Err(_) => None,
        }
    }

    /// Replaces the cached snapshot with a freshly loaded one, keeping the
    /// previous snapshot when the load failed.
    ///
    /// The runtime stays the authority: the cache is only ever written from a
    /// successful runtime read, so a failed refresh cannot leave a partial or
    /// invented value behind. Split out of [`refresh`](Self::refresh) so this
    /// policy is testable without a live control plane.
    fn persist(&self, loaded: Result<RuntimeConfigSnapshot, String>) -> Result<(), String> {
        match loaded {
            Ok(snapshot) => {
                self.store(snapshot);
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    /// Reloads the snapshot from the runtime over the control plane.
    pub async fn refresh(&self, supervisor: &RuntimeSupervisor) -> Result<(), String> {
        self.persist(load(supervisor).await)
    }
}

/// Loads the runtime configuration map over the control plane.
pub async fn load(supervisor: &RuntimeSupervisor) -> Result<RuntimeConfigSnapshot, String> {
    let value = crate::runtime_data::get_all_config(supervisor).await?;
    let settings = serde_json::from_value::<HashMap<String, Value>>(value)
        .map_err(|error| format!("unexpected runtime config response: {error}"))?;
    Ok(RuntimeConfigSnapshot::from_settings(settings))
}

/// Reads the freshest runtime configuration an async desktop path can get.
///
/// Prefers a live control-plane read so a value the runtime just changed is
/// visible at once, writes it to the cache for the synchronous readers, and
/// falls back to the last cached snapshot only when the runtime is
/// unavailable. Returns `None` before the runtime has ever published a
/// snapshot; callers then apply their own explicit default instead of reading a
/// local database.
pub async fn current_or_load<R: tauri::Runtime>(
    app: &AppHandle<R>,
) -> Option<Arc<RuntimeConfigSnapshot>> {
    let cache = app.try_state::<Arc<RuntimeConfigCache>>()?;
    if let Some(supervisor) = app.try_state::<Arc<RuntimeSupervisor>>() {
        if let Err(error) = cache.refresh(supervisor.as_ref()).await {
            log::warn!("Failed to refresh the runtime configuration: {error}");
        }
    }
    cache.current()
}

/// Persists one window size to the runtime configuration.
pub async fn save_window_size(
    supervisor: &RuntimeSupervisor,
    window_label: &str,
    size: WindowSize,
) -> Result<(), String> {
    let Some(key) = window_size_key(window_label) else {
        return Ok(());
    };
    let value = serde_json::to_value(size).map_err(|error| error.to_string())?;
    crate::runtime_data::set_config(supervisor, key.to_string(), value).await
}

/// Persists a window position under the given configuration key.
pub async fn save_window_position(
    supervisor: &RuntimeSupervisor,
    key: &str,
    position: MainWindowPosition,
) -> Result<(), String> {
    let value = serde_json::to_value(position).map_err(|error| error.to_string())?;
    crate::runtime_data::set_config(supervisor, key.to_string(), value).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn snapshot() -> RuntimeConfigSnapshot {
        let mut settings = HashMap::new();
        settings.insert(
            CFG_WINDOW_SIZE.to_string(),
            json!({ "width": 800.0, "height": 600.0 }),
        );
        settings.insert(
            CFG_ASSISTANT_WINDOW_SIZE.to_string(),
            json!({ "width": 500.0, "height": 600.0 }),
        );
        settings.insert(
            CFG_WORKFLOW_WINDOW_POSITION.to_string(),
            json!({ "screen_name": "primary", "x": 10, "y": 20 }),
        );
        settings.insert(CFG_PROXY_TYPE.to_string(), json!("http"));
        settings.insert(CFG_PROXY_SERVER.to_string(), json!("127.0.0.1:8080"));
        RuntimeConfigSnapshot::from_settings(settings)
    }

    #[test]
    fn window_size_key_covers_only_known_labels() {
        assert_eq!(window_size_key("main"), Some(CFG_WINDOW_SIZE));
        assert_eq!(
            window_size_key("assistant"),
            Some(CFG_ASSISTANT_WINDOW_SIZE)
        );
        assert_eq!(window_size_key("workflow"), Some(CFG_WORKFLOW_WINDOW_SIZE));
        assert_eq!(
            window_size_key("proxy_switcher"),
            Some(CFG_PROXY_SWITCHER_WINDOW_SIZE)
        );
        assert_eq!(window_size_key("settings"), None);
    }

    #[test]
    fn restore_config_matches_previous_labels() {
        let snapshot = snapshot();

        let main = snapshot.restore_config("main");
        assert_eq!(main.size.map(|size| size.width), Some(800.0));
        assert!(main.position.is_some());

        let workflow = snapshot.restore_config("workflow");
        assert!(workflow.position.is_some());
        assert_eq!(workflow.position.map(|position| position.x), Some(10));

        let assistant = snapshot.restore_config("assistant");
        assert_eq!(assistant.size.map(|size| size.height), Some(600.0));
        assert!(assistant.position.is_none());

        let settings_window = snapshot.restore_config("settings");
        assert!(settings_window.size.is_none());
        assert!(settings_window.position.is_none());
    }

    #[test]
    fn proxy_type_reads_http_configuration() {
        let proxy = snapshot().proxy_type();
        match proxy {
            ProxyType::Http(server, username, password) => {
                assert_eq!(server, "127.0.0.1:8080");
                assert_eq!(username.as_deref(), Some(""));
                assert_eq!(password.as_deref(), Some(""));
            }
            other => panic!("expected an HTTP proxy, got {other:?}"),
        }
    }

    #[test]
    fn cache_starts_empty_and_stores_a_snapshot() {
        let cache = RuntimeConfigCache::new();
        assert!(cache.current().is_none());
        cache.store(snapshot());
        assert_eq!(
            cache
                .current()
                .map(|snapshot| snapshot.get_bool("missing", true)),
            Some(true)
        );
    }

    #[test]
    fn scraper_debug_mode_defaults_off_and_reads_the_flag() {
        // No snapshot at all means the local display preference stays off
        // instead of falling back to a local store.
        assert!(!RuntimeConfigSnapshot::default().scraper_debug_mode());

        let mut settings = HashMap::new();
        settings.insert(CFG_SCRAPER_DEBUG_MODE.to_string(), json!(true));
        assert!(RuntimeConfigSnapshot::from_settings(settings).scraper_debug_mode());
    }

    #[test]
    fn a_failed_refresh_keeps_the_previous_snapshot() {
        let cache = RuntimeConfigCache::new();
        cache.store(snapshot());
        let previous = cache.current().expect("the snapshot was just stored");

        let error = cache
            .persist(Err("runtime unavailable".to_string()))
            .expect_err("a failed load is reported");
        assert_eq!(error, "runtime unavailable");
        // The cache is written from the runtime only; a failure changes nothing.
        assert!(
            Arc::ptr_eq(
                &previous,
                &cache.current().expect("the previous snapshot is kept")
            ),
            "a failed refresh must not replace the cached snapshot"
        );

        // A later successful load does replace it.
        cache
            .persist(Ok(RuntimeConfigSnapshot::default()))
            .expect("a successful load is applied");
        assert!(!cache
            .current()
            .expect("the new snapshot is present")
            .scraper_debug_mode());
        assert!(!Arc::ptr_eq(
            &previous,
            &cache.current().expect("the new snapshot is present")
        ));
    }

    #[tokio::test]
    async fn current_or_load_falls_back_to_the_cached_snapshot() {
        let app = crate::test::get_app_handle();
        app.manage(Arc::new(RuntimeConfigCache::new()));

        // With the runtime cache registered but no snapshot published yet,
        // there is nothing to read.
        assert!(current_or_load(&app).await.is_none());

        {
            let cache = app.state::<Arc<RuntimeConfigCache>>();
            cache.store(snapshot());
        }
        // Without a connected supervisor the last cached snapshot is returned
        // instead of blocking on a control-plane read.
        assert!(current_or_load(&app).await.is_some());
    }
}
