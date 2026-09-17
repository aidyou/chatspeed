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

/// Full path of the discovery document inside a runtime directory.
///
/// `chatspeed-headless` publishes its discovery document under its own
/// experiment data directory instead of `${CHATSPEED_HOME}/runtime`, so a
/// headless instance and a desktop instance on the same machine never
/// overwrite each other's endpoint or token (AC-1/AC-6). That is why every
/// read/write below takes the directory explicitly: there is exactly one
/// resolution rule, and both runtimes pass the directory they own.
pub fn discovery_path_in(runtime_dir: &std::path::Path) -> PathBuf {
    runtime_dir.join(DISCOVERY_FILE_NAME)
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

/// Atomically writes the discovery document into a runtime directory with
/// current-user-only permissions. Overwrites any stale document from a
/// previous instance.
pub fn write_discovery_in(
    dir: &std::path::Path,
    document: &ControlPlaneDiscovery,
) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    restrict_permissions(dir, 0o700)?;

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

/// Reads the discovery document from a runtime directory.
pub fn read_discovery_in(dir: &std::path::Path) -> Result<ControlPlaneDiscovery, String> {
    let path = dir.join(DISCOVERY_FILE_NAME);
    let body = fs::read(&path).map_err(|e| e.to_string())?;
    serde_json::from_slice(&body).map_err(|e| e.to_string())
}

/// Removes the discovery document from a runtime directory, and only when it
/// still belongs to `instance_id`. Returns `true` when it was removed.
pub fn remove_discovery_if_instance_in(dir: &std::path::Path, instance_id: &str) -> bool {
    match read_discovery_in(dir) {
        Ok(document) if document.server_instance_id == instance_id => {
            let path = dir.join(DISCOVERY_FILE_NAME);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn document(instance_id: &str) -> ControlPlaneDiscovery {
        ControlPlaneDiscovery {
            protocol_version: CONTROL_PROTOCOL_VERSION.to_string(),
            server_instance_id: instance_id.to_string(),
            pid: std::process::id(),
            host: CONTROL_PLANE_HOST.to_string(),
            port: 41234,
            token: "token-value".to_string(),
            started_at: "unix-1".to_string(),
        }
    }

    #[test]
    fn an_explicit_runtime_dir_round_trips() {
        let directory = tempfile::tempdir().expect("tempdir");
        write_discovery_in(directory.path(), &document("instance-a")).expect("write");
        let read = read_discovery_in(directory.path()).expect("read");
        assert_eq!(read.server_instance_id, "instance-a");
        assert_eq!(read.port, 41234);
        assert!(discovery_path_in(directory.path()).exists());

        // Removal is fenced on the instance id.
        assert!(!remove_discovery_if_instance_in(
            directory.path(),
            "instance-b"
        ));
        assert!(discovery_path_in(directory.path()).exists());
        assert!(remove_discovery_if_instance_in(
            directory.path(),
            "instance-a"
        ));
        assert!(!discovery_path_in(directory.path()).exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_published_document_is_current_user_only() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().expect("tempdir");
        write_discovery_in(directory.path(), &document("instance-a")).expect("write");
        let file_mode = fs::metadata(discovery_path_in(directory.path()))
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(file_mode, 0o600);
        let dir_mode = fs::metadata(directory.path())
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700);
    }
}
