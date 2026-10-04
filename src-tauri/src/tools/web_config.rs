//! Transport-neutral configuration port for the WebView-backed web tools.
//!
//! `WebFetch` and `WebSearch` used to read their proxy/search configuration from
//! an `Arc<MainStore>` reached through the Tauri `AppHandle`. The runtime now
//! owns that database, so the desktop tools are constructed with a small port
//! instead: the client bridge dispatcher supplies [`MapWebToolConfig`] built from
//! the runtime configuration it reads over the control plane. The desktop never
//! reaches a local database for these tools.

use std::collections::HashMap;

use serde_json::Value;

/// The narrow synchronous configuration surface the web tools need.
pub trait WebToolConfig: Send + Sync + 'static {
    /// Reads a string setting, falling back to `default` when absent.
    fn get_string(&self, key: &str, default: &str) -> String;
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
