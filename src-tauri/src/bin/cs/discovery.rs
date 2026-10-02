//! Shared discovery loading and runtime startup for `cscli`.
//!
//! Discovery, protocol validation, the readiness handshake and the runtime child
//! are implemented by the reusable runtime client so Tauri and `cscli` cannot
//! drift in path priority or wire checks.

use crate::client::{map_client_error, ControlPlaneClient};
use crate::error::CliError;
use crate::output::eprint_diagnostic;
pub use chatspeed_contracts::ControlPlaneDiscovery;
use chatspeed_contracts::DISCOVERY_FILE_NAME;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long a freshly started runtime may take to become ready.
const SPAWN_READY_TIMEOUT: Duration = Duration::from_secs(10);

/// Resolves the discovery file using the shared runtime-client priority.
pub fn discovery_path(explicit: Option<&Path>) -> PathBuf {
    chatspeed_runtime_client::discovery_path(explicit)
}

/// Loads and validates the discovery document.
pub fn load_discovery(explicit: Option<&Path>) -> Result<ControlPlaneDiscovery, CliError> {
    let path = discovery_path(explicit);
    chatspeed_runtime_client::load_discovery(Some(&path)).map_err(map_client_error)
}

/// Resolves a ready client, starting the runtime only when that is legitimate.
///
/// The runtime is started only when the discovery document is genuinely absent
/// and the resolved file name is the one the runtime itself publishes. A custom
/// `--discovery-file` name cannot be satisfied by a spawned runtime (it writes
/// the fixed name), so that case fails instead of starting a runtime whose
/// document would never appear at the requested path.
pub async fn load_or_spawn(
    explicit: Option<&Path>,
) -> Result<
    (
        ControlPlaneDiscovery,
        ControlPlaneClient,
        Option<chatspeed_runtime_client::RuntimeChild>,
    ),
    CliError,
> {
    match load_discovery(explicit) {
        Ok(document) => {
            let client = ControlPlaneClient::connect(&document).await?;
            Ok((document, client, None))
        }
        Err(error) => {
            if !may_spawn(explicit, &error) {
                return Err(error);
            }
            let path = discovery_path(explicit);
            let runtime_dir = path
                .parent()
                .ok_or_else(|| CliError::discovery("cannot resolve runtime directory for spawn"))?;
            let mut child =
                chatspeed_runtime_client::spawn_runtime(runtime_dir).map_err(map_client_error)?;
            let document = match chatspeed_runtime_client::wait_for_discovery(
                Some(&path),
                SPAWN_READY_TIMEOUT,
            )
            .await
            {
                Ok(document) => document,
                Err(error) => {
                    // Surface the failed child's stderr so a startup failure is
                    // diagnosable instead of a bare timeout.
                    if let Some(detail) = child.diagnostics() {
                        eprint_diagnostic(&format!("cs: runtime failed to become ready: {detail}"));
                    }
                    return Err(map_client_error(error));
                }
            };
            // Readiness is proven; the child is long-lived from here.
            child.silence_stderr();
            let client = ControlPlaneClient::new(&document)?;
            Ok((document, client, Some(child)))
        }
    }
}

/// Whether a discovery failure permits starting a runtime ourselves.
///
/// Only a genuinely absent discovery document counts: an unreadable or malformed
/// document that still exists must be reported, never silently worked around by
/// starting a second runtime.
fn may_spawn(explicit: Option<&Path>, error: &CliError) -> bool {
    matches!(error, CliError::Discovery(_))
        && spawn_allowed(explicit)
        && chatspeed_runtime_client::discovery_absent(&discovery_path(explicit))
}

/// Whether a spawned runtime could publish the requested discovery file.
fn spawn_allowed(explicit: Option<&Path>) -> bool {
    match explicit {
        None => true,
        Some(path) => path.file_name() == Some(OsStr::new(DISCOVERY_FILE_NAME)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_discovery_path_is_preserved() {
        let path = Path::new("/tmp/explicit.json");
        assert_eq!(discovery_path(Some(path)), path);
    }

    #[test]
    fn incompatible_protocol_is_rejected_without_duplicate_validation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(DISCOVERY_FILE_NAME);
        std::fs::write(
            &path,
            serde_json::json!({
                "protocol_version": "2",
                "server_instance_id": "instance",
                "pid": 1,
                "host": "127.0.0.1",
                "port": 1,
                "token": "secret",
                "started_at": "now"
            })
            .to_string(),
        )
        .expect("write discovery");

        assert!(matches!(
            load_discovery(Some(&path)),
            Err(CliError::Protocol(_))
        ));
    }

    #[test]
    fn only_a_missing_default_named_document_allows_a_spawn() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Genuinely absent default-named file: a spawn is allowed.
        let missing = dir.path().join(DISCOVERY_FILE_NAME);
        assert!(may_spawn(Some(&missing), &CliError::discovery("missing")));

        // A malformed document still exists: never spawn over it.
        std::fs::write(&missing, b"not json").expect("write");
        assert!(!may_spawn(
            Some(&missing),
            &CliError::discovery("malformed")
        ));

        // A discovery path that is a directory is not absent either.
        let as_directory = dir.path().join(DISCOVERY_FILE_NAME);
        std::fs::remove_file(&as_directory).expect("remove");
        std::fs::create_dir(&as_directory).expect("mkdir");
        assert!(!may_spawn(
            Some(&as_directory),
            &CliError::discovery("unreadable")
        ));

        // A custom file name can never be satisfied by a spawned runtime.
        let custom = dir.path().join("custom.json");
        assert!(!may_spawn(Some(&custom), &CliError::discovery("missing")));

        // Non-discovery errors are never spawnable.
        assert!(!may_spawn(Some(&missing), &CliError::protocol("major")));
        assert!(!may_spawn(Some(&missing), &CliError::transport("down")));
    }

    #[test]
    fn spawn_is_allowed_only_for_the_runtime_default_file_name() {
        assert!(spawn_allowed(None));
        assert!(spawn_allowed(Some(Path::new(
            "/home/u/.chatspeed/runtime/control-plane-v1.json"
        ))));
        assert!(!spawn_allowed(Some(Path::new("/tmp/custom.json"))));
        assert!(!spawn_allowed(Some(Path::new("/tmp/no-file-name/"))));
    }

    #[tokio::test]
    async fn a_wrong_control_plane_fails_before_any_lease_is_registered() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        // The desktop in-process control plane reports its own service name; the
        // readiness handshake must reject it, so no lease is ever registered.
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let server = tokio::spawn(async move {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let mut buffer = [0u8; 1024];
            let _ = stream.read(&mut buffer).await;
            let body = r#"{"service":"chatspeed-workflow-control-plane","protocol_version":"1","schema_version":1,"server_instance_id":"instance","pid":1}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.flush().await;
        });

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(DISCOVERY_FILE_NAME);
        std::fs::write(
            &path,
            serde_json::json!({
                "protocol_version": "1",
                "server_instance_id": "instance",
                "pid": 1,
                "host": "127.0.0.1",
                "port": addr.port(),
                "token": "secret",
                "started_at": "now"
            })
            .to_string(),
        )
        .expect("write discovery");

        match load_or_spawn(Some(&path)).await {
            Err(CliError::Protocol(message)) => {
                assert!(
                    message.contains("chatspeed-runtime"),
                    "unexpected {message}"
                );
            }
            Err(other) => panic!("expected a protocol error, got {other:?}"),
            Ok(_) => panic!("the desktop control plane must be rejected"),
        }
        server.await.expect("server task");
    }
}
