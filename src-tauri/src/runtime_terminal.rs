//! Tauri-side adapter for the runtime-owned interactive terminal.
//!
//! This module owns no PTY, child process, transcript or terminal session state.
//! It only translates the established Tauri command/event surface into typed
//! `chatspeed-runtime-client` calls and relays runtime SSE events to the workflow
//! window. Runtime loss is fail-closed and resets the local presentation.

use crate::runtime_client::RuntimeSupervisor;
use chatspeed_contracts::{
    TerminalCreateRequest, TerminalResizeRequest, TerminalStreamEvent, TerminalWriteRequest,
};
use chatspeed_runtime_client::{ClientError, RuntimeClient, TerminalEventStream};
use std::sync::Arc;
use tauri::{Emitter, Window};

use crate::terminal::{TerminalSessionMetadata, TerminalShell};

async fn connected_terminal(
    supervisor: &RuntimeSupervisor,
) -> Result<(RuntimeClient, String, String), String> {
    supervisor
        .terminal_connection()
        .await
        .map_err(|error| error.to_string())
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
        let stream = client
            .terminal_stream(&client_id, &lease_id, &session.session_id)
            .await
            .map_err(client_message)?;
        tokio::spawn(forward_terminal_events(window.clone(), stream));
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
    start_stream_relay(
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

async fn start_stream_relay(
    window: Window,
    client: RuntimeClient,
    client_id: String,
    lease_id: String,
    session_id: String,
) -> Result<(), String> {
    let stream = client
        .terminal_stream(&client_id, &lease_id, &session_id)
        .await
        .map_err(client_message)?;
    tokio::spawn(forward_terminal_events(window, stream));
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
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn disconnected_runtime_fails_closed_without_a_local_pty() {
        let supervisor = RuntimeSupervisor::new();
        let error = list_shells(&supervisor)
            .await
            .expect_err("disconnected runtime must reject terminal access");
        assert!(error.contains("not connected"), "error was {error}");
    }
}
