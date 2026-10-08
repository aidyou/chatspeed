//! Rust-only wire types for the plugin UI loopback gateway.
//!
//! None of these values reach the frontend: the capability token and the
//! resolved URLs stay inside the host, which is what keeps one tab from naming
//! another tab's assets. The inventory DTOs are the local, `Deserialize` view of
//! the runtime's plugin inventory; the shared contract in
//! [`crate::plugin_types`] is `Serialize`-only because the runtime owns it.

use serde::Deserialize;

/// A per-tab capability grant handed to the host.
///
/// The host keeps this in Rust only; the capability token and the resolved URLs
/// are never returned to the frontend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginUiGrant {
    /// The opaque, random capability that authorises one tab's asset requests.
    pub capability: String,
    /// The absolute URL of the plugin UI entry the tab should load.
    pub url: String,
    /// The absolute URL prefix shared by every asset under this grant.
    pub prefix: String,
}

/// The subset of the runtime plugin inventory the gateway proves before a grant.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct PluginUiInventory {
    pub schema_version: u32,
    pub plugins: Vec<PluginUiRecord>,
}

/// One plugin record from the inventory.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct PluginUiRecord {
    pub id: String,
    pub kind: String,
    pub state: PluginUiState,
    #[serde(default)]
    pub ui: Option<PluginUiDescriptor>,
}

/// The verified static UI descriptor of a plugin record.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct PluginUiDescriptor {
    pub entry: String,
    #[serde(default)]
    pub verified: bool,
}

/// The lifecycle state reported for a plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PluginUiState {
    NotInstalled,
    Enabled,
    Disabled,
}