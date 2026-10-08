//! Shared, filesystem-free contract for the runtime-owned `agent-skills`
//! plugin service.
//!
//! The executable lifecycle (path resolution, staging, publication, ownership
//! proof and removal) lives in [`crate::plugin`], which is compiled only into
//! the standalone runtime behind the `plugin-service` feature. This module
//! carries the stable error codes, manifest descriptor and wire DTOs, so the
//! desktop's typed forwarding adapters can name the exact tokens the control
//! plane returns without compiling the plugin filesystem implementation into
//! the desktop binary.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// The manifest schema this service understands.
pub const PLUGIN_SCHEMA: &str = "chatspeed.agent-skills.plugin/v2";
/// The version of the inventory collection returned by the runtime.
pub const PLUGIN_INVENTORY_SCHEMA_VERSION: u32 = 1;
/// Version of the runtime-controlled static UI resource route.
pub const PLUGIN_UI_ROUTE_VERSION: u32 = 1;
/// The plugin id. It also names the on-disk bundle directory.
pub const PLUGIN_ID: &str = "agent-skills";
/// The only plugin kind exposed by this release.
pub const BUILTIN_PLUGIN_KIND: &str = "builtin";
/// The canonical manifest file name inside a bundle.
pub const MANIFEST_FILE_NAME: &str = "plugin.json";
/// The service-owned lifecycle state file. It is not part of the bundle assets.
pub const STATE_FILE_NAME: &str = ".host-state.json";
/// The closed permission set a manifest may request. The static service grants
/// no filesystem, database, token or IPC permission beyond it.
pub const ALLOWED_PERMISSIONS: &[&str] = &["skills:read"];
/// Upper bound on one asset path length inside the bundle.
const MAX_ASSET_PATH: usize = 512;
/// What uninstall is allowed to remove, published for the client contract.
pub const UNINSTALL_SCOPE: &str = "plugin-bundle-only";
/// The host kind reported in the inventory.
pub const HOST_KIND: &str = "static-bundle-no-exec";
/// The IPC surface reported in the inventory.
pub const IPC_SURFACE: &str = "fixed-typed-facade";

/// Canonical machine-readable error codes for the plugin service.
pub mod plugin_code {
    /// The manifest is malformed or violates the descriptor contract.
    pub const INVALID_MANIFEST: &str = "invalid_manifest";
    /// The mutation is intentionally refused (path confinement, integrity).
    pub const REFUSED: &str = "refused";
    /// A filesystem operation failed.
    pub const IO: &str = "io_error";
    /// The bundle is not installed.
    pub const NOT_INSTALLED: &str = "not_installed";
    /// The service has no resolved `${CHATSPEED_HOME}`.
    pub const UNAVAILABLE: &str = "unavailable";
    /// A staging directory for the same bundle already exists.
    pub const CONFLICT: &str = "conflict";
    /// An unclassified internal failure.
    pub const INTERNAL: &str = "internal";
}

/// A plugin service failure with a stable code and a non-secret message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PluginError {
    pub code: String,
    pub message: String,
}

impl PluginError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
        }
    }

    pub fn invalid_manifest(message: impl Into<String>) -> Self {
        Self::new(plugin_code::INVALID_MANIFEST, message)
    }

    pub fn refused(message: impl Into<String>) -> Self {
        Self::new(plugin_code::REFUSED, message)
    }

    pub fn io(message: impl Into<String>) -> Self {
        Self::new(plugin_code::IO, message)
    }

    pub fn not_installed(message: impl Into<String>) -> Self {
        Self::new(plugin_code::NOT_INSTALLED, message)
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(plugin_code::UNAVAILABLE, message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(plugin_code::CONFLICT, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(plugin_code::INTERNAL, message)
    }
}

impl std::fmt::Display for PluginError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for PluginError {}

/// The strict plugin manifest: `schema`, `id`, `version`, `entry`, `assets`,
/// `permissions` and optional `ui`. An unknown field is refused so the contract
/// cannot silently grow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    pub schema: String,
    pub id: String,
    pub version: String,
    pub entry: String,
    pub assets: Vec<String>,
    pub permissions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui: Option<PluginUiManifest>,
}

/// Static UI metadata declared by a built-in plugin manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginUiManifest {
    pub entry: String,
    pub assets: Vec<String>,
}

impl PluginUiManifest {
    pub fn validate(&self, manifest: &PluginManifest) -> Result<(), PluginError> {
        if !is_safe_relative_path(&self.entry) {
            return Err(PluginError::invalid_manifest(
                "the plugin UI entry is not a confined relative path",
            ));
        }
        if self.assets.is_empty() {
            return Err(PluginError::invalid_manifest(
                "the plugin UI declares no assets",
            ));
        }
        let mut seen = BTreeSet::new();
        for asset in &self.assets {
            if !is_safe_relative_path(asset) || !seen.insert(asset.as_str()) {
                return Err(PluginError::invalid_manifest(
                    "the plugin UI contains an invalid or duplicate asset path",
                ));
            }
        }
        if !seen.contains(self.entry.as_str()) || !manifest.assets.contains(&self.entry) {
            return Err(PluginError::invalid_manifest(
                "the plugin UI entry is not a declared bundle asset",
            ));
        }
        if self.assets.iter().any(|asset| !manifest.assets.contains(asset)) {
            return Err(PluginError::invalid_manifest(
                "the plugin UI asset is not a declared bundle asset",
            ));
        }
        Ok(())
    }
}

impl PluginManifest {
    /// Parses and fully validates a manifest.
    pub fn parse(json: &str) -> Result<Self, PluginError> {
        let manifest: PluginManifest = serde_json::from_str(json).map_err(|error| {
            PluginError::invalid_manifest(format!("unparsable manifest: {error}"))
        })?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Validates the manifest against the closed service contract.
    pub fn validate(&self) -> Result<(), PluginError> {
        if self.schema != PLUGIN_SCHEMA {
            return Err(PluginError::invalid_manifest(format!(
                "unsupported plugin schema '{}'",
                self.schema
            )));
        }
        if self.id != PLUGIN_ID {
            return Err(PluginError::invalid_manifest(format!(
                "unexpected plugin id '{}'",
                self.id
            )));
        }
        if semver::Version::parse(&self.version).is_err() {
            return Err(PluginError::invalid_manifest(format!(
                "plugin version '{}' is not semantic",
                self.version
            )));
        }
        if !is_safe_relative_path(&self.entry) {
            return Err(PluginError::invalid_manifest(format!(
                "plugin entry '{}' is not a confined relative path",
                self.entry
            )));
        }
        if self.assets.is_empty() {
            return Err(PluginError::invalid_manifest(
                "the manifest declares no assets".to_string(),
            ));
        }

        let mut seen = BTreeSet::new();
        for asset in &self.assets {
            if !is_safe_relative_path(asset) {
                return Err(PluginError::invalid_manifest(format!(
                    "asset path '{asset}' is not a confined relative path"
                )));
            }
            if !seen.insert(asset.as_str()) {
                return Err(PluginError::invalid_manifest(format!(
                    "asset path '{asset}' is declared twice"
                )));
            }
        }
        if !seen.contains(self.entry.as_str()) {
            return Err(PluginError::invalid_manifest(format!(
                "entry '{}' is not one of the declared assets",
                self.entry
            )));
        }

        if let Some(ui) = &self.ui {
            ui.validate(self)?;
        }

        if self.permissions.is_empty() {
            return Err(PluginError::invalid_manifest(
                "the manifest requests no permissions".to_string(),
            ));
        }
        let mut granted = BTreeSet::new();
        for permission in &self.permissions {
            if !ALLOWED_PERMISSIONS.contains(&permission.as_str()) {
                return Err(PluginError::invalid_manifest(format!(
                    "permission '{permission}' is not in the closed allow-list"
                )));
            }
            if !granted.insert(permission.as_str()) {
                return Err(PluginError::invalid_manifest(format!(
                    "permission '{permission}' is requested twice"
                )));
            }
        }
        Ok(())
    }
}

/// Whether a manifest path stays inside the bundle: relative, `/`-separated and
/// free of `.`/`..`/drive/`~`/control components.
pub(crate) fn is_safe_relative_path(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_ASSET_PATH {
        return false;
    }
    if value.starts_with('/') || value.starts_with('\\') || value.ends_with('/') {
        return false;
    }
    if value.contains('\\') {
        return false;
    }
    value.split('/').all(|part| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part != "~"
            && !part.contains(':')
            && !part.chars().any(char::is_control)
    })
}

/// The published isolation contract. Every field is a constant the service can
/// actually prove: the bundle is static, no code is executed, and the only
/// surface is the fixed typed facade.
#[derive(Debug, Clone, Serialize)]
pub struct HostIsolation {
    pub host: &'static str,
    pub ipc_surface: &'static str,
    pub plugin_code_executed: bool,
    pub tauri_ipc: bool,
    pub generic_command_surface: bool,
    pub filesystem_api: bool,
    pub database_access: bool,
    pub token_access: bool,
}

pub fn host_isolation() -> HostIsolation {
    HostIsolation {
        host: HOST_KIND,
        ipc_surface: IPC_SURFACE,
        plugin_code_executed: false,
        tauri_ipc: false,
        generic_command_surface: false,
        filesystem_api: false,
        database_access: false,
        token_access: false,
    }
}

/// The plugin inventory returned by every lifecycle call.
#[derive(Debug, Clone, Serialize)]
pub struct PluginInventory {
    pub schema_version: u32,
    pub plugins: Vec<PluginRecord>,
    pub uninstall_scope: &'static str,
    pub managed_skills_dir: String,
    pub host: HostIsolation,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginRecord {
    pub id: String,
    pub kind: &'static str,
    pub version: Option<String>,
    pub state: PluginState,
    pub capabilities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ui: Option<PluginUi>,
    pub root: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle_digest: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginState {
    NotInstalled,
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginUi {
    pub entry: String,
    pub assets: Vec<String>,
    pub verified: bool,
    pub content_digest: String,
    pub route_version: u32,
}
