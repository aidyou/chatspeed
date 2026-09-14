//! Current-user discovery for the loopback workflow control plane.
//!
//! The running desktop app publishes a discovery document so local clients
//! (the `cs` CLI today, future chat clients later) can find the endpoint and
//! authenticate. The document contains a random per-instance bearer token and
//! is only readable by the current user (Unix `0700`/`0600`; on Windows the
//! file lives under the current-user profile and inherits its ACL).
//!
//! The token must never be logged or embedded in URLs.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

/// Discovery document file name inside the runtime directory.
pub const DISCOVERY_FILE_NAME: &str = "control-plane-v1.json";

/// Protocol major version advertised by the control plane.
pub const CONTROL_PROTOCOL_VERSION: &str = "1";

/// Loopback host the control plane binds to.
pub const CONTROL_PLANE_HOST: &str = "127.0.0.1";

/// A published control-plane discovery document.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ControlPlaneDiscovery {
    pub protocol_version: String,
    pub server_instance_id: String,
    pub pid: u32,
    pub host: String,
    pub port: u16,
    pub token: String,
    pub started_at: String,
}

/// Root of the discovery runtime directory:
/// `${CHATSPEED_HOME:-~/.chatspeed}/runtime`.
pub fn discovery_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("CHATSPEED_HOME") {
        return PathBuf::from(home).join("runtime");
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".chatspeed").join("runtime")
}

/// Full path of the discovery document.
pub fn discovery_path() -> PathBuf {
    discovery_dir().join(DISCOVERY_FILE_NAME)
}

#[cfg(unix)]
fn restrict_permissions(path: &std::path::Path, mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(path).map_err(|e| e.to_string())?.permissions();
    permissions.set_mode(mode);
    fs::set_permissions(path, permissions).map_err(|e| e.to_string())
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &std::path::Path, _mode: u32) -> Result<(), String> {
    // On Windows the discovery directory lives under the current-user profile
    // and inherits the user-only ACL from the parent directory.
    Ok(())
}

/// Atomically writes the discovery document with current-user-only
/// permissions. Overwrites any stale document from a previous instance.
pub fn write_discovery(document: &ControlPlaneDiscovery) -> Result<(), String> {
    let dir = discovery_dir();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    restrict_permissions(&dir, 0o700)?;

    let path = dir.join(DISCOVERY_FILE_NAME);
    let tmp_path = dir.join(format!("{}.tmp", DISCOVERY_FILE_NAME));
    let body = serde_json::to_vec_pretty(document).map_err(|e| e.to_string())?;

    {
        let mut file = fs::File::create(&tmp_path).map_err(|e| e.to_string())?;
        file.write_all(&body).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
    }
    restrict_permissions(&tmp_path, 0o600)?;
    fs::rename(&tmp_path, &path).map_err(|e| e.to_string())?;
    Ok(())
}

/// Reads the discovery document (used by tests and diagnostics).
pub fn read_discovery() -> Result<ControlPlaneDiscovery, String> {
    let path = discovery_path();
    let body = fs::read(&path).map_err(|e| e.to_string())?;
    serde_json::from_slice(&body).map_err(|e| e.to_string())
}

/// Removes the discovery document only when it belongs to `instance_id`.
/// Returns `true` when the document was removed.
pub fn remove_discovery_if_instance(instance_id: &str) -> bool {
    match read_discovery() {
        Ok(document) if document.server_instance_id == instance_id => {
            let path = discovery_path();
            match fs::remove_file(&path) {
                Ok(()) => true,
                Err(error) => {
                    log::warn!(
                        "[ControlPlane] Failed to remove discovery document: {}",
                        error
                    );
                    false
                }
            }
        }
        _ => false,
    }
}
