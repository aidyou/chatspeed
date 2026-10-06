//! Runtime-only interactive terminal routes on the canonical control plane.
//!
//! The standalone runtime is the single owner of the user terminal PTY. These
//! routes expose that owner as typed `/control/v1/terminal` operations over the
//! same bearer-protected server the workflow, capability, chat and automation
//! routes use, so the desktop client never runs a second PTY owner and never
//! opens the runtime database:
//!
//! - `GET  /control/v1/terminal/shells` lists the offered shells;
//! - `GET  /control/v1/terminal/sessions` lists the caller's own sessions;
//! - `POST /control/v1/terminal/create` opens a session;
//! - `POST /control/v1/terminal/{session_id}/write` forwards raw input;
//! - `POST /control/v1/terminal/{session_id}/resize` resizes the PTY;
//! - `POST /control/v1/terminal/{session_id}/close` retires a session;
//! - `GET  /control/v1/terminal/{session_id}/stream` relays typed SSE events.
//!
//! # Lease binding
//!
//! The bearer token authenticates a control-plane client, but it must not be
//! sufficient to drive a user's shell. Every terminal route therefore also
//! proves the caller's live client lease through two dedicated headers
//! ([`TERMINAL_CLIENT_HEADER`] and [`TERMINAL_LEASE_HEADER`]). The lease proof
//! never travels in a URL query or a request body, so it cannot end up in a log
//! line. A session is bound to the lease that created it and no other client can
//! list, write, resize, close or stream it.
//!
//! # Delivery
//!
//! The stream is a bounded per-session broadcast owned by the terminal manager.
//! There is no transcript and no cursor: a client that attaches after the
//! process ended receives the single terminal envelope immediately, and a client
//! whose bounded broadcast overflowed receives a `reset` envelope instead of a
//! silently corrupted byte stream.
//!
//! This module is compiled only for the desktop-free build; the desktop control
//! plane keeps exactly its previous routes.

use crate::terminal::{TerminalError, TerminalSubscription};
use crate::workflow::react::client::http::dto;
use crate::workflow::react::client::http::server::{ControlPlaneState, RuntimeLeaseError};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chatspeed_contracts::{
    TerminalCloseRequest, TerminalCreateRequest, TerminalListSessionsRequest,
    TerminalListShellsRequest, TerminalResizeRequest, TerminalSessionMetadataDto, TerminalShellDto,
    TerminalStreamEnvelope, TerminalStreamEvent, TerminalWriteRequest,
};
use serde::de::DeserializeOwned;
use std::convert::Infallible;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;

/// `GET /control/v1/terminal/shells` route template.
pub const TERMINAL_SHELLS_PATH: &str = "/control/v1/terminal/shells";
/// `GET /control/v1/terminal/sessions` route template.
pub const TERMINAL_SESSIONS_PATH: &str = "/control/v1/terminal/sessions";
/// `POST /control/v1/terminal/create` route template.
pub const TERMINAL_CREATE_PATH: &str = "/control/v1/terminal/create";
/// `POST /control/v1/terminal/{session_id}/write` route template.
pub const TERMINAL_WRITE_PATH: &str = "/control/v1/terminal/{session_id}/write";
/// `POST /control/v1/terminal/{session_id}/resize` route template.
pub const TERMINAL_RESIZE_PATH: &str = "/control/v1/terminal/{session_id}/resize";
/// `POST /control/v1/terminal/{session_id}/close` route template.
pub const TERMINAL_CLOSE_PATH: &str = "/control/v1/terminal/{session_id}/close";
/// `GET /control/v1/terminal/{session_id}/stream` route template.
pub const TERMINAL_STREAM_PATH: &str = "/control/v1/terminal/{session_id}/stream";

/// Header carrying the owning client id.
///
/// The proof travels here rather than in a URL query or body, so a credential
/// cannot be captured by request logging.
pub const TERMINAL_CLIENT_HEADER: &str = "x-terminal-client";
/// Header carrying the owning lease id.
pub const TERMINAL_LEASE_HEADER: &str = "x-terminal-lease";

/// Bounded capacity of the SSE response pump.
const PUMP_CAPACITY: usize = 64;

/// The runtime owner's canonical terminal operations.
///
/// The trait exposes exactly the typed operations the routes need and no generic
/// passthrough, so a client cannot reach an unlisted PTY behavior. The desktop
/// control plane never links it.
pub trait RuntimeTerminalPlane: Send + Sync + 'static {
    /// The shells offered for an interactive terminal.
    fn list_shells(&self) -> Vec<TerminalShellDto>;
    /// The sessions owned by `client_id`.
    fn list_sessions(
        &self,
        client_id: &str,
        lease_id: &str,
    ) -> Vec<TerminalSessionMetadataDto>;
    /// Opens a session bound to `(client_id, lease_id)`.
    fn create(
        &self,
        client_id: &str,
        lease_id: &str,
        request: &TerminalCreateRequest,
    ) -> Result<TerminalSessionMetadataDto, TerminalError>;
    /// Forwards raw input to a session owned by `client_id`.
    fn write(
        &self,
        client_id: &str,
        lease_id: &str,
        session_id: &str,
        request: &TerminalWriteRequest,
    ) -> Result<(), TerminalError>;
    /// Resizes a session owned by `client_id`.
    fn resize(
        &self,
        client_id: &str,
        lease_id: &str,
        session_id: &str,
        request: &TerminalResizeRequest,
    ) -> Result<(), TerminalError>;
    /// Retires a session owned by `client_id`; repeating it is harmless.
    fn close(
        &self,
        client_id: &str,
        lease_id: &str,
        session_id: &str,
    ) -> Result<(), TerminalError>;
    /// Subscribes to a session's typed event stream.
    ///
    /// `Ok(None)` means the session is owned but no longer broadcastable.
    fn subscribe(
        &self,
        client_id: &str,
        lease_id: &str,
        session_id: &str,
    ) -> Result<Option<TerminalSubscription>, TerminalError>;
    /// Drops sessions whose registering lease no longer validates.
    ///
    /// Called by the runtime lease sweeper so a released or expired lease cannot
    /// leave an orphaned user PTY running.
    fn sweep_invalid_leases(&self, is_valid: &(dyn Fn(&str, &str) -> bool + Send + Sync));
}

/// Builds the runtime terminal router extension.
pub fn terminal_router() -> Router<ControlPlaneState> {
    Router::new()
        .route(TERMINAL_SHELLS_PATH, get(list_terminal_shells))
        .route(TERMINAL_SESSIONS_PATH, get(list_terminal_sessions))
        .route(TERMINAL_CREATE_PATH, post(create_terminal_session))
        .route(TERMINAL_WRITE_PATH, post(write_terminal_session))
        .route(TERMINAL_RESIZE_PATH, post(resize_terminal_session))
        .route(TERMINAL_CLOSE_PATH, post(close_terminal_session))
        .route(TERMINAL_STREAM_PATH, get(stream_terminal_session))
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn list_terminal_shells(
    State(state): State<ControlPlaneState>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let plane = match require_plane(&state) {
        Ok(plane) => plane,
        Err(response) => return response,
    };
    if let Err(response) = require_live_lease(&state, &headers) {
        return response;
    }
    if let Err(response) = parse_body::<TerminalListShellsRequest>(&body, "terminal shells request")
    {
        return response;
    }
    Json(plane.list_shells()).into_response()
}

async fn list_terminal_sessions(
    State(state): State<ControlPlaneState>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let plane = match require_plane(&state) {
        Ok(plane) => plane,
        Err(response) => return response,
    };
    let (client_id, lease_id) = match require_live_lease_with_id(&state, &headers) {
        Ok(proof) => proof,
        Err(response) => return response,
    };
    if let Err(response) =
        parse_body::<TerminalListSessionsRequest>(&body, "terminal sessions request")
    {
        return response;
    }
    Json(plane.list_sessions(&client_id, &lease_id)).into_response()
}

async fn create_terminal_session(
    State(state): State<ControlPlaneState>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let plane = match require_plane(&state) {
        Ok(plane) => plane,
        Err(response) => return response,
    };
    let (client_id, lease_id) = match require_live_lease_with_id(&state, &headers) {
        Ok(proof) => proof,
        Err(response) => return response,
    };
    let request: TerminalCreateRequest = match parse_body(&body, "terminal create request") {
        Ok(request) => request,
        Err(response) => return response,
    };
    match plane.create(&client_id, &lease_id, &request) {
        Ok(metadata) => Json(metadata).into_response(),
        Err(error) => terminal_error_response(&error),
    }
}

async fn write_terminal_session(
    State(state): State<ControlPlaneState>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let plane = match require_plane(&state) {
        Ok(plane) => plane,
        Err(response) => return response,
    };
    let (client_id, lease_id) = match require_live_lease_with_id(&state, &headers) {
        Ok(proof) => proof,
        Err(response) => return response,
    };
    let request: TerminalWriteRequest = match parse_body(&body, "terminal write request") {
        Ok(request) => request,
        Err(response) => return response,
    };
    match plane.write(&client_id, &lease_id, &session_id, &request) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => terminal_error_response(&error),
    }
}

async fn resize_terminal_session(
    State(state): State<ControlPlaneState>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let plane = match require_plane(&state) {
        Ok(plane) => plane,
        Err(response) => return response,
    };
    let (client_id, lease_id) = match require_live_lease_with_id(&state, &headers) {
        Ok(proof) => proof,
        Err(response) => return response,
    };
    let request: TerminalResizeRequest = match parse_body(&body, "terminal resize request") {
        Ok(request) => request,
        Err(response) => return response,
    };
    match plane.resize(&client_id, &lease_id, &session_id, &request) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => terminal_error_response(&error),
    }
}

async fn close_terminal_session(
    State(state): State<ControlPlaneState>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let plane = match require_plane(&state) {
        Ok(plane) => plane,
        Err(response) => return response,
    };
    let (client_id, lease_id) = match require_live_lease_with_id(&state, &headers) {
        Ok(proof) => proof,
        Err(response) => return response,
    };
    if let Err(response) = parse_body::<TerminalCloseRequest>(&body, "terminal close request") {
        return response;
    }
    match plane.close(&client_id, &lease_id, &session_id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => terminal_error_response(&error),
    }
}

async fn stream_terminal_session(
    State(state): State<ControlPlaneState>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let plane = match require_plane(&state) {
        Ok(plane) => plane,
        Err(response) => return response,
    };
    let (client_id, lease_id) = match require_live_lease_with_id(&state, &headers) {
        Ok(proof) => proof,
        Err(response) => return response,
    };
    let subscription = match plane.subscribe(&client_id, &lease_id, &session_id) {
        Ok(Some(subscription)) => subscription,
        Ok(None) => return unavailable_stream(&session_id, "terminal_session_unavailable"),
        Err(error) => return terminal_error_response(&error),
    };

    // The pump is transport-neutral (it yields typed envelopes), so the bounded
    // broadcast contract is testable without inspecting an SSE frame.
    let (tx, rx) = mpsc::channel::<TerminalStreamEnvelope>(PUMP_CAPACITY);
    tokio::spawn(pump_terminal_stream(session_id, subscription, tx));
    let events =
        ReceiverStream::new(rx).map(|envelope| Ok::<Event, Infallible>(envelope_event(&envelope)));
    Sse::new(events)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// Forwards one session's typed events, always ending on a terminal envelope.
///
/// The bounded broadcast is the only source; when it lags the observer is told
/// to reset rather than receiving an incomplete byte stream.
async fn pump_terminal_stream(
    session_id: String,
    subscription: TerminalSubscription,
    tx: mpsc::Sender<TerminalStreamEnvelope>,
) {
    if let Some(terminal) = subscription.pending_terminal {
        let _ = tx.send(terminal).await;
        return;
    }

    let mut receiver = subscription.receiver;
    let mut last_sequence = 0_u64;
    loop {
        match receiver.recv().await {
            Ok(envelope) => {
                last_sequence = envelope.sequence;
                let terminal = envelope.is_terminal();
                if tx.send(envelope).await.is_err() {
                    return;
                }
                if terminal {
                    return;
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => {
                let reset = TerminalStreamEnvelope::new(
                    session_id,
                    last_sequence.saturating_add(1),
                    TerminalStreamEvent::Reset {
                        reason: "stream_lagged".to_string(),
                    },
                );
                let _ = tx.send(reset).await;
                return;
            }
            Err(broadcast::error::RecvError::Closed) => return,
        }
    }
}

/// One-shot SSE that immediately reports a session that cannot be streamed.
fn unavailable_stream(session_id: &str, reason: &str) -> Response {
    let envelope = TerminalStreamEnvelope::new(
        session_id.to_string(),
        0,
        TerminalStreamEvent::Unavailable {
            reason: reason.to_string(),
        },
    );
    let (tx, rx) = mpsc::channel::<TerminalStreamEnvelope>(1);
    tokio::spawn(async move {
        let _ = tx.send(envelope).await;
    });
    let events =
        ReceiverStream::new(rx).map(|envelope| Ok::<Event, Infallible>(envelope_event(&envelope)));
    Sse::new(events)
        .keep_alive(KeepAlive::default())
        .into_response()
}

fn envelope_event(envelope: &TerminalStreamEnvelope) -> Event {
    let data = serde_json::to_string(envelope).unwrap_or_else(|_| "{}".to_string());
    Event::default()
        .id(envelope.sequence.to_string())
        .data(data)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn require_plane(state: &ControlPlaneState) -> Result<Arc<dyn RuntimeTerminalPlane>, Response> {
    state
        .terminal
        .clone()
        .ok_or_else(terminal_unavailable_response)
}

/// Stable answer for a control plane whose process owns no runtime terminal.
fn terminal_unavailable_response() -> Response {
    dto::error_response(
        StatusCode::SERVICE_UNAVAILABLE,
        "runtime_unavailable",
        "This control plane does not own the runtime terminal".to_string(),
    )
}

/// Resolves the caller's live lease from the dedicated proof headers.
fn require_live_lease(state: &ControlPlaneState, headers: &HeaderMap) -> Result<String, Response> {
    require_live_lease_with_id(state, headers).map(|(client_id, _lease_id)| client_id)
}

fn require_live_lease_with_id(
    state: &ControlPlaneState,
    headers: &HeaderMap,
) -> Result<(String, String), Response> {
    let client_id = header_value(headers, TERMINAL_CLIENT_HEADER)?;
    let lease_id = header_value(headers, TERMINAL_LEASE_HEADER)?;
    let Some(runtime) = state.runtime.as_ref() else {
        return Err(dto::error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "This control plane does not own a client lease lifecycle".to_string(),
        ));
    };
    match runtime.validate_lease(&client_id, &lease_id) {
        Ok(_) => Ok((client_id, lease_id)),
        Err(RuntimeLeaseError::InvalidInput(message)) => Err(dto::error_response(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            message,
        )),
        Err(RuntimeLeaseError::NotFound(_)) => Err(dto::error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "the terminal lease proof is no longer valid".to_string(),
        )),
    }
}

fn header_value(headers: &HeaderMap, name: &'static str) -> Result<String, Response> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            dto::error_response(
                StatusCode::FORBIDDEN,
                "forbidden",
                format!("missing terminal lease proof header `{name}`"),
            )
        })
}

fn parse_body<T: DeserializeOwned>(body: &str, what: &str) -> Result<T, Response> {
    let body = if body.trim().is_empty() { "{}" } else { body };
    serde_json::from_str(body).map_err(|error| {
        dto::error_response(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            format!("Invalid {what}: {error}"),
        )
    })
}

/// Maps a terminal failure onto a stable control-plane error envelope.
fn terminal_error_response(error: &TerminalError) -> Response {
    let (status, code) = match error {
        TerminalError::SessionNotFound => (StatusCode::NOT_FOUND, "not_found"),
        TerminalError::SessionExited => (StatusCode::CONFLICT, "conflict"),
        TerminalError::Forbidden(_) => (StatusCode::FORBIDDEN, "forbidden"),
        TerminalError::ShellUnavailable
        | TerminalError::InvalidSize
        | TerminalError::InvalidCwd(_) => (StatusCode::BAD_REQUEST, "invalid_input"),
        TerminalError::PtyOpenFailed(_)
        | TerminalError::SpawnFailed(_)
        | TerminalError::WriterFailed(_)
        | TerminalError::ReaderFailed(_)
        | TerminalError::WriteFailed(_)
        | TerminalError::ResizeFailed(_) => (StatusCode::INTERNAL_SERVER_ERROR, "terminal_failure"),
    };
    dto::error_response(status, code, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::TerminalStreamBroker;
    use chatspeed_contracts::{TerminalExitEvent, TerminalOutputEvent};

    #[test]
    fn the_route_templates_match_the_documented_control_plane_paths() {
        assert_eq!(TERMINAL_SHELLS_PATH, "/control/v1/terminal/shells");
        assert_eq!(TERMINAL_SESSIONS_PATH, "/control/v1/terminal/sessions");
        assert_eq!(TERMINAL_CREATE_PATH, "/control/v1/terminal/create");
        assert_eq!(
            TERMINAL_WRITE_PATH,
            "/control/v1/terminal/{session_id}/write"
        );
        assert_eq!(
            TERMINAL_RESIZE_PATH,
            "/control/v1/terminal/{session_id}/resize"
        );
        assert_eq!(
            TERMINAL_CLOSE_PATH,
            "/control/v1/terminal/{session_id}/close"
        );
        assert_eq!(
            TERMINAL_STREAM_PATH,
            "/control/v1/terminal/{session_id}/stream"
        );
    }

    #[test]
    fn a_missing_lease_proof_header_is_rejected() {
        let headers = HeaderMap::new();
        let response = header_value(&headers, TERMINAL_CLIENT_HEADER).expect_err("missing");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let mut headers = HeaderMap::new();
        headers.insert(TERMINAL_CLIENT_HEADER, "client-a".parse().expect("header"));
        // The lease header is still required even when the client header is set.
        assert!(header_value(&headers, TERMINAL_LEASE_HEADER).is_err());
    }

    #[test]
    fn terminal_errors_map_to_stable_status_codes() {
        assert_eq!(
            terminal_error_response(&TerminalError::SessionNotFound).status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            terminal_error_response(&TerminalError::SessionExited).status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            terminal_error_response(&TerminalError::Forbidden("no".to_string())).status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            terminal_error_response(&TerminalError::InvalidSize).status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            terminal_error_response(&TerminalError::SpawnFailed("boom".to_string())).status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[tokio::test]
    async fn the_pump_forwards_output_then_terminates_on_the_exit_event() {
        let broker = TerminalStreamBroker::new();
        let entry = broker.entry("session-1");
        let subscription = entry.subscribe();
        let (tx, mut rx) = mpsc::channel::<TerminalStreamEnvelope>(8);

        entry.publish(TerminalStreamEvent::Output(TerminalOutputEvent {
            session_id: "session-1".to_string(),
            data_base64: "aGVsbG8=".to_string(),
        }));
        entry.publish_terminal(TerminalStreamEvent::Exit(TerminalExitEvent {
            session_id: "session-1".to_string(),
            exit_code: None,
        }));

        pump_terminal_stream("session-1".to_string(), subscription, tx).await;

        let first = rx.recv().await.expect("first envelope");
        assert_eq!(
            first.event,
            TerminalStreamEvent::Output(TerminalOutputEvent {
                session_id: "session-1".to_string(),
                data_base64: "aGVsbG8=".to_string(),
            })
        );
        let terminal = rx.recv().await.expect("terminal envelope");
        assert!(terminal.is_terminal());
        assert_eq!(
            terminal.event,
            TerminalStreamEvent::Exit(TerminalExitEvent {
                session_id: "session-1".to_string(),
                exit_code: None,
            })
        );
        assert!(
            rx.recv().await.is_none(),
            "the pump must end after the terminal event"
        );
    }

    #[tokio::test]
    async fn the_pump_reports_a_reset_when_the_bounded_broadcast_lags() {
        let broker = TerminalStreamBroker::new();
        let entry = broker.entry("session-1");
        let subscription = entry.subscribe();
        // Overflow the bounded broadcast before the pump consumes, so its first
        // receive reports a lag rather than delivering corrupt output.
        for _ in 0..512 {
            entry.publish(TerminalStreamEvent::Output(TerminalOutputEvent {
                session_id: "session-1".to_string(),
                data_base64: "AA==".to_string(),
            }));
        }
        let (tx, mut rx) = mpsc::channel::<TerminalStreamEnvelope>(8);
        pump_terminal_stream("session-1".to_string(), subscription, tx).await;

        let reset = rx.recv().await.expect("lag envelope");
        assert!(matches!(reset.event, TerminalStreamEvent::Reset { .. }));
        assert!(reset.is_terminal());
        assert!(rx.recv().await.is_none());
    }

    #[tokio::test]
    async fn a_late_subscriber_observes_the_already_published_exit() {
        let broker = TerminalStreamBroker::new();
        let entry = broker.entry("session-1");
        entry.publish_terminal(TerminalStreamEvent::Exit(TerminalExitEvent {
            session_id: "session-1".to_string(),
            exit_code: None,
        }));
        let subscription = entry.subscribe();
        let (tx, mut rx) = mpsc::channel::<TerminalStreamEnvelope>(8);
        pump_terminal_stream("session-1".to_string(), subscription, tx).await;

        let exit = rx.recv().await.expect("exit envelope");
        assert_eq!(
            exit.event,
            TerminalStreamEvent::Exit(TerminalExitEvent {
                session_id: "session-1".to_string(),
                exit_code: None,
            })
        );
        assert!(rx.recv().await.is_none());
    }
}
