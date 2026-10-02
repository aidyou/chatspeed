//! Transport-neutral configuration port for the WebView-backed web tools.
//!
//! `WebFetch` and `WebSearch` used to read their proxy/search configuration from
//! an `Arc<MainStore>` reached through the Tauri `AppHandle`. The runtime now
//! owns that database, so the desktop tools are constructed with a small port
//! instead: the legacy tool manager supplies a store-backed port, and the client
//! bridge dispatcher supplies a port built from the runtime configuration it
//! reads over the control plane. Neither path ever puts runtime state into the
//! desktop process.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use crate::db::MainStore;

/// The narrow synchronous configuration surface the web tools need.
pub trait WebToolConfig: Send + Sync + 'static {
    /// Reads a string setting, falling back to `default` when absent.
    fn get_string(&self, key: &str, default: &str) -> String;
}

/// A port backed by the legacy local `MainStore`.
///
/// Only the (dormant) `ToolManager::register_available_tools` path builds this;
/// the runtime-owned desktop no longer creates a `MainStore`, so the client
/// bridge uses [`MapWebToolConfig`] instead.
pub struct MainStoreWebToolConfig {
    store: Arc<MainStore>,
}

impl MainStoreWebToolConfig {
    /// Wraps a local store as a web-tool configuration port.
    pub fn new(store: Arc<MainStore>) -> Self {
        Self { store }
    }
}

impl WebToolConfig for MainStoreWebToolConfig {
    fn get_string(&self, key: &str, default: &str) -> String {
        self.store.get_config(key, default.to_string())
    }
}

/// A port backed by a plain key/value map.
///
/// The client bridge dispatcher builds this from the runtime's own
/// configuration, so the desktop tools read exactly the values the runtime
/// owner holds without a database in the desktop process.
pub struct MapWebToolConfig {
    values: HashMap<String, Value>,
}

impl MapWebToolConfig {
    /// Wraps a configuration map read from the runtime.
    pub fn new(values: HashMap<String, Value>) -> Self {
        Self { values }
    }
}

impl WebToolConfig for MapWebToolConfig {
    fn get_string(&self, key: &str, default: &str) -> String {
        match self.values.get(key) {
            Some(Value::String(value)) => value.clone(),
            // A number/bool stored for a string setting is coerced so a
            // mis-typed config value degrades to a readable string instead of
            // silently disappearing.
            Some(other) if !other.is_null() => other.to_string(),
            _ => default.to_string(),
        }
    }
}
