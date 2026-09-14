//! Discovery file reading for the `cs` CLI.
//!
//! The CLI and the desktop app must agree on the same discovery algorithm:
//! `${CHATSPEED_HOME:-~/.chatspeed}/runtime/control-plane-v1.json`. The CLI
//! only reads this file; it never writes it.

use crate::error::CliError;
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Discovery document file name inside the runtime directory.
pub const DISCOVERY_FILE_NAME: &str = "control-plane-v1.json";

/// Protocol major version this CLI understands.
pub const SUPPORTED_PROTOCOL_MAJOR: u32 = 1;

/// A published control-plane discovery document.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ControlPlaneDiscovery {
    pub protocol_version: String,
    pub server_instance_id: String,
    pub pid: u32,
    pub host: String,
    pub port: u16,
    pub token: String,
}

/// Resolves the discovery file path:
/// `--discovery-file` > `${CHATSPEED_HOME}` > `~/.chatspeed/runtime`.
pub fn discovery_path(explicit: Option<&Path>) -> PathBuf {
    if let Some(path) = explicit {
        return path.to_path_buf();
    }
    if let Some(home) = std::env::var_os("CHATSPEED_HOME") {
        return PathBuf::from(home)
            .join("runtime")
            .join(DISCOVERY_FILE_NAME);
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".chatspeed")
        .join("runtime")
        .join(DISCOVERY_FILE_NAME)
}

/// Loads and validates the discovery document.
pub fn load_discovery(explicit: Option<&Path>) -> Result<ControlPlaneDiscovery, CliError> {
    let path = discovery_path(explicit);
    let body = std::fs::read(&path).map_err(|error| {
        CliError::discovery(format!(
            "Cannot read discovery file {}: {}. Is ChatSpeed running?",
            path.display(),
            error
        ))
    })?;
    let document: ControlPlaneDiscovery = serde_json::from_slice(&body).map_err(|error| {
        CliError::discovery(format!(
            "Invalid discovery file {}: {}",
            path.display(),
            error
        ))
    })?;

    // Protocol negotiation: reject incompatible major versions.
    let major = document
        .protocol_version
        .split('.')
        .next()
        .and_then(|part| part.parse::<u32>().ok())
        .unwrap_or(0);
    if major != SUPPORTED_PROTOCOL_MAJOR {
        return Err(CliError::protocol(format!(
            "Control plane reports protocol version {} but this CLI supports major version {}",
            document.protocol_version, SUPPORTED_PROTOCOL_MAJOR
        )));
    }

    Ok(document)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_path_prefers_explicit_then_env_then_default() {
        let explicit = Path::new("/tmp/explicit.json");
        assert_eq!(
            discovery_path(Some(explicit)),
            PathBuf::from("/tmp/explicit.json")
        );

        std::env::set_var("CHATSPEED_HOME", "/tmp/cs-home");
        assert_eq!(
            discovery_path(None),
            PathBuf::from("/tmp/cs-home/runtime/control-plane-v1.json")
        );
        std::env::remove_var("CHATSPEED_HOME");
    }

    #[test]
    fn load_discovery_rejects_incompatible_major_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("discovery.json");
        std::fs::write(
            &path,
            serde_json::json!({
                "protocol_version": "2",
                "server_instance_id": "i",
                "pid": 1,
                "host": "127.0.0.1",
                "port": 1,
                "token": "t",
                "started_at": "x"
            })
            .to_string(),
        )
        .unwrap();

        let error = load_discovery(Some(&path)).expect_err("must reject major version 2");
        assert!(matches!(error, CliError::Protocol(_)));
    }

    #[test]
    fn load_discovery_reports_missing_file_as_discovery_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.json");
        let error = load_discovery(Some(&path)).expect_err("must fail on missing file");
        assert!(matches!(error, CliError::Discovery(_)));
    }
}
