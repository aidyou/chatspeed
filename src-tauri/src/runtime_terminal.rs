//! Tauri-side adapter for the runtime-owned interactive terminal.
//!
//! This module owns no PTY, child process, transcript or terminal session state.
//! It only translates the established Tauri command/event surface into typed
//! `chatspeed-runtime-client` calls and relays runtime SSE events to the workflow
//! window. Runtime loss is fail-closed and resets the local presentation.

use crate::runtime_client::{client_error_wire, runtime_error_wire, RuntimeSupervisor};
use chatspeed_contracts::{
    TerminalCreateRequest, TerminalResizeRequest, TerminalStreamEvent, TerminalWriteRequest,
};
use chatspeed_runtime_client::{ClientError, RuntimeClient, TerminalEventStream};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use tauri::{Emitter, Window};
use tokio::task::JoinHandle;

use crate::terminal::{TerminalSessionMetadata, TerminalShell};

async fn connected_terminal(
    supervisor: &RuntimeSupervisor,
) -> Result<(RuntimeClient, String, String), String> {
    supervisor
        .terminal_connection()
        .await
        .map_err(runtime_error_wire)
}

pub(crate) async fn list_shells(
    supervisor: &RuntimeSupervisor,
) -> Result<Vec<TerminalShell>, String> {
    let (client, client_id, lease_id) = connected_terminal(supervisor).await?;
    client
        .terminal_list_shells(&client_id, &lease_id)
        .await
        .map_err(client_message)
}

pub(crate) async fn list_sessions(
    window: Window,
    supervisor: Arc<RuntimeSupervisor>,
) -> Result<Vec<TerminalSessionMetadata>, String> {
    let (client, client_id, lease_id) = connected_terminal(supervisor.as_ref()).await?;
    let sessions = client
        .terminal_list_sessions(&client_id, &lease_id)
        .await
        .map_err(client_message)?;
    for session in &sessions {
        ensure_stream_relay(
            window.clone(),
            client.clone(),
            client_id.clone(),
            lease_id.clone(),
            session.session_id.clone(),
        )
        .await?;
    }
    Ok(sessions)
}

pub(crate) async fn create(
    window: Window,
    supervisor: Arc<RuntimeSupervisor>,
    cwd: Option<String>,
    shell_path: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
) -> Result<TerminalSessionMetadata, String> {
    let (client, client_id, lease_id) = connected_terminal(supervisor.as_ref()).await?;
    let request = TerminalCreateRequest {
        cwd,
        shell_path,
        cols,
        rows,
    };
    let metadata = client
        .terminal_create(&client_id, &lease_id, &request)
        .await
        .map_err(client_message)?;
    ensure_stream_relay(
        window,
        client,
        client_id,
        lease_id,
        metadata.session_id.clone(),
    )
    .await?;
    Ok(metadata)
}

pub(crate) async fn write(
    supervisor: &RuntimeSupervisor,
    session_id: &str,
    input: String,
) -> Result<(), String> {
    let (client, client_id, lease_id) = connected_terminal(supervisor).await?;
    client
        .terminal_write(
            &client_id,
            &lease_id,
            session_id,
            &TerminalWriteRequest { input },
        )
        .await
        .map_err(client_message)
}

pub(crate) async fn resize(
    supervisor: &RuntimeSupervisor,
    session_id: &str,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let (client, client_id, lease_id) = connected_terminal(supervisor).await?;
    client
        .terminal_resize(
            &client_id,
            &lease_id,
            session_id,
            &TerminalResizeRequest { cols, rows },
        )
        .await
        .map_err(client_message)
}

pub(crate) async fn close(supervisor: &RuntimeSupervisor, session_id: &str) -> Result<(), String> {
    let (client, client_id, lease_id) = connected_terminal(supervisor).await?;
    client
        .terminal_close(&client_id, &lease_id, session_id)
        .await
        .map_err(client_message)
}

/// Relays already attached to a terminal session, keyed by window and session.
///
/// The desktop process outlives a frontend reload. The new page lists the same live sessions
/// again, and attaching a second stream for a session would forward every runtime event more
/// than once, so the next output would be duplicated in the terminal. One relay per session is
/// kept until that relay ends.
fn relays() -> &'static Mutex<HashMap<(String, String), JoinHandle<()>>> {
    static RELAYS: OnceLock<Mutex<HashMap<(String, String), JoinHandle<()>>>> = OnceLock::new();
    RELAYS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock_relays() -> std::sync::MutexGuard<'static, HashMap<(String, String), JoinHandle<()>>> {
    relays()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

async fn ensure_stream_relay(
    window: Window,
    client: RuntimeClient,
    client_id: String,
    lease_id: String,
    session_id: String,
) -> Result<(), String> {
    let key = (window.label().to_string(), session_id.clone());
    if lock_relays().contains_key(&key) {
        return Ok(());
    }

    let stream = client
        .terminal_stream(&client_id, &lease_id, &session_id)
        .await
        .map_err(client_message)?;

    let mut registered = lock_relays();
    if registered.contains_key(&key) {
        // Another reconciliation attached first while this stream was opening.
        return Ok(());
    }
    let cleanup_key = key.clone();
    let handle = tokio::spawn(async move {
        forward_terminal_events(window, stream).await;
        lock_relays().remove(&cleanup_key);
    });
    registered.insert(key, handle);
    Ok(())
}

async fn forward_terminal_events(window: Window, mut stream: TerminalEventStream) {
    loop {
        match stream.next_event().await {
            Ok(Some(envelope)) => match envelope.event {
                TerminalStreamEvent::Output(output) => {
                    if let Err(error) = window.emit("terminal://output", output) {
                        log::warn!(
                            "terminal output relay failed for '{}': {}",
                            window.label(),
                            error
                        );
                        return;
                    }
                }
                TerminalStreamEvent::Exit(exit) => {
                    let _ = window.emit("terminal://exit", exit);
                    return;
                }
                TerminalStreamEvent::Reset { .. } | TerminalStreamEvent::Unavailable { .. } => {
                    let _ = window.emit("terminal://reset", ());
                    return;
                }
            },
            Ok(None) | Err(_) => {
                let _ = window.emit("terminal://reset", ());
                return;
            }
        }
    }
}

fn client_message(error: ClientError) -> String {
    client_error_wire(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chatspeed_contracts::{ErrorReason, UserErrorPresentation};
    use chatspeed_runtime_client::ClientResponseError;

    #[tokio::test]
    async fn disconnected_runtime_fails_closed_without_a_local_pty() {
        let supervisor = RuntimeSupervisor::new();
        let error = list_shells(&supervisor)
            .await
            .expect_err("disconnected runtime must reject terminal access");
        assert!(error.contains("not connected"), "error was {error}");
    }

    #[test]
    fn structured_rejection_keeps_its_presentation_on_the_ipc_wire() {
        let presentation = UserErrorPresentation {
            schema_version: 1,
            message: "The runtime is stopping".to_string(),
            reason: Some(ErrorReason::RuntimeStopping),
            upstream_status: Some(503),
        };
        let wire = client_message(ClientError::Server(ClientResponseError {
            legacy_display: "503 runtime_stopping: The runtime is stopping".to_string(),
            status: 503,
            code: "runtime_stopping".to_string(),
            message: "The runtime is stopping".to_string(),
            presentation: Some(presentation),
        }));
        let value: serde_json::Value = serde_json::from_str(&wire).expect("structured wire");
        assert_eq!(value["error"]["code"], "runtime_stopping");
        assert_eq!(value["error"]["message"], "The runtime is stopping");
        assert_eq!(value["error"]["presentation"]["reason"], "runtime_stopping");
        assert_eq!(value["status"], 503);
    }
}
