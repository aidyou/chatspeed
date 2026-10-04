//! User-terminal wire contracts for the runtime control plane (U-7).
//!
//! The standalone runtime is the single owner of the interactive user terminal
//! PTY. The desktop client reaches it only through the typed
//! `/control/v1/terminal` routes declared here, so this module is the single
//! source of truth for their JSON names.
//!
//! Two rules apply:
//! - the interactive-terminal wire names the Tauri frontend already consumes are
//!   preserved byte for byte — [`TerminalSessionMetadataDto`] is the snake_case
//!   `{session_id, shell_name, shell_path, cwd, alive}` document and
//!   [`TerminalOutputEvent`]/[`TerminalExitEvent`] are the exact
//!   `terminal://output` / `terminal://exit` payloads;
//! - the lease proof used to bind a session to its owning client travels only in
//!   dedicated headers, never in a URL query or a request body, so no credential
//!   can end up in a log line.
//!
//! Every request and metadata document is closed (`deny_unknown_fields`): a
//! caller cannot smuggle an unrecognized field through a typed contract.

use serde::{Deserialize, Serialize};

/// Schema version carried by every [`TerminalStreamEnvelope`].
pub const TERMINAL_SCHEMA_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Shells and sessions
// ---------------------------------------------------------------------------

/// An executable shell the runtime offers for an interactive terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct TerminalShellDto {
    pub name: String,
    pub path: String,
    pub is_default: bool,
}

/// The metadata document the frontend consumes for one terminal tab.
///
/// The field names are the existing snake_case wire names and must not change:
/// the desktop adapter forwards this document to the window verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct TerminalSessionMetadataDto {
    pub session_id: String,
    pub shell_name: String,
    pub shell_path: String,
    pub cwd: String,
    pub alive: bool,
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

/// `GET /control/v1/terminal/shells` request.
///
/// The document is empty on purpose: the operation needs no arguments, but the
/// route keeps the same closed typed request contract every other terminal route
/// uses. The owning lease is proven by headers, never by this body.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct TerminalListShellsRequest {}

/// `GET /control/v1/terminal/sessions` request.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct TerminalListSessionsRequest {}

/// `POST /control/v1/terminal/create` request.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct TerminalCreateRequest {
    /// Initial working directory; the runtime falls back to the user home.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Absolute path of the requested shell; the runtime falls back to the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell_path: Option<String>,
    /// Initial columns; the runtime applies its own bounds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cols: Option<u16>,
    /// Initial rows; the runtime applies its own bounds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<u16>,
}

/// `POST /control/v1/terminal/{session_id}/write` request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct TerminalWriteRequest {
    /// Raw keystrokes written to the PTY, forwarded as UTF-8 text.
    pub input: String,
}

/// `POST /control/v1/terminal/{session_id}/resize` request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct TerminalResizeRequest {
    pub cols: u16,
    pub rows: u16,
}

/// `POST /control/v1/terminal/{session_id}/close` request.
///
/// Explicit close is idempotent, so the document carries no fields.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct TerminalCloseRequest {}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// The `terminal://output` payload: one base64-encoded chunk of PTY output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct TerminalOutputEvent {
    pub session_id: String,
    pub data_base64: String,
}

/// The `terminal://exit` payload reported when a session's process ends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct TerminalExitEvent {
    pub session_id: String,
    /// Exit status when the runtime observed one; `None` for a PTY EOF or a
    /// terminated session, matching the existing desktop payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<u32>,
}

/// One typed SSE event of a terminal stream.
///
/// The output and exit variants flatten their payload structs, so a client sees
/// exactly the `{session_id, data_base64}` and `{session_id, exit_code}`
/// documents it already consumes, distinguished by the `kind` tag. `Reset` and
/// `Unavailable` are terminal events: the stream carries no persistent
/// transcript, so a client that lost continuity must reset its view instead of
/// expecting a replay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TerminalStreamEvent {
    /// Incremental PTY output.
    Output(TerminalOutputEvent),
    /// The session's process ended.
    Exit(TerminalExitEvent),
    /// The stream lost continuity (a slow consumer lagged a bounded broadcast);
    /// the client must reset rather than assume it missed only invisible bytes.
    Reset { reason: String },
    /// The session is no longer streamable in this runtime instance.
    Unavailable { reason: String },
}

/// Versioned SSE envelope for one terminal event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct TerminalStreamEnvelope {
    pub schema_version: u32,
    pub session_id: String,
    pub sequence: u64,
    pub event: TerminalStreamEvent,
}

impl TerminalStreamEnvelope {
    /// Builds an envelope with the current schema version.
    pub fn new(session_id: impl Into<String>, sequence: u64, event: TerminalStreamEvent) -> Self {
        Self {
            schema_version: TERMINAL_SCHEMA_VERSION,
            session_id: session_id.into(),
            sequence,
            event,
        }
    }

    /// Whether this event terminates the stream.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.event,
            TerminalStreamEvent::Exit(_)
                | TerminalStreamEvent::Reset { .. }
                | TerminalStreamEvent::Unavailable { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn session_metadata_keeps_the_existing_frontend_wire_names() {
        let metadata = TerminalSessionMetadataDto {
            session_id: "session-1".to_string(),
            shell_name: "bash".to_string(),
            shell_path: "/bin/bash".to_string(),
            cwd: "/workspace".to_string(),
            alive: true,
        };
        assert_eq!(
            serde_json::to_value(&metadata).expect("serialize"),
            json!({
                "session_id": "session-1",
                "shell_name": "bash",
                "shell_path": "/bin/bash",
                "cwd": "/workspace",
                "alive": true,
            })
        );
    }

    #[test]
    fn shell_descriptor_is_snake_case() {
        let shell = TerminalShellDto {
            name: "zsh".to_string(),
            path: "/bin/zsh".to_string(),
            is_default: false,
        };
        assert_eq!(
            serde_json::to_value(&shell).expect("serialize"),
            json!({"name": "zsh", "path": "/bin/zsh", "is_default": false})
        );
    }

    #[test]
    fn closed_requests_reject_unknown_fields() {
        assert!(serde_json::from_str::<TerminalCreateRequest>("{}").is_ok());
        assert!(serde_json::from_str::<TerminalCreateRequest>("{\"extra\": 1}").is_err());
        assert!(serde_json::from_str::<TerminalListShellsRequest>("{}").is_ok());
        assert!(serde_json::from_str::<TerminalListShellsRequest>("{\"extra\": 1}").is_err());
        assert!(serde_json::from_str::<TerminalWriteRequest>("{\"input\": \"ls\\n\"}").is_ok());
        assert!(
            serde_json::from_str::<TerminalWriteRequest>("{\"input\": \"x\", \"y\": 2}").is_err()
        );
    }

    #[test]
    fn create_request_omits_absent_optionals_and_round_trips() {
        let request = TerminalCreateRequest {
            cwd: Some("/workspace".to_string()),
            shell_path: None,
            cols: Some(120),
            rows: None,
        };
        let value = serde_json::to_value(&request).expect("serialize");
        assert_eq!(value, json!({"cwd": "/workspace", "cols": 120}));
        let parsed: TerminalCreateRequest = serde_json::from_value(value).expect("deserialize");
        assert_eq!(parsed, request);
    }

    #[test]
    fn resize_and_close_requests_are_closed() {
        let resize = TerminalResizeRequest { cols: 80, rows: 24 };
        assert_eq!(
            serde_json::to_value(resize).expect("serialize"),
            json!({"cols": 80, "rows": 24})
        );
        assert_eq!(
            serde_json::to_value(TerminalCloseRequest::default()).expect("serialize"),
            json!({})
        );
    }

    #[test]
    fn output_and_exit_events_match_the_existing_event_payloads() {
        let output = TerminalOutputEvent {
            session_id: "session-1".to_string(),
            data_base64: "aGVsbG8=".to_string(),
        };
        assert_eq!(
            serde_json::to_value(&output).expect("serialize"),
            json!({"session_id": "session-1", "data_base64": "aGVsbG8="})
        );

        let exit = TerminalExitEvent {
            session_id: "session-1".to_string(),
            exit_code: None,
        };
        assert_eq!(
            serde_json::to_value(&exit).expect("serialize"),
            json!({"session_id": "session-1"})
        );
        let exit = TerminalExitEvent {
            session_id: "session-1".to_string(),
            exit_code: Some(0),
        };
        assert_eq!(
            serde_json::to_value(&exit).expect("serialize"),
            json!({"session_id": "session-1", "exit_code": 0})
        );
    }

    #[test]
    fn the_stream_envelope_flattens_every_output_and_exit_payload() {
        let output = TerminalStreamEnvelope::new(
            "session-1",
            0,
            TerminalStreamEvent::Output(TerminalOutputEvent {
                session_id: "session-1".to_string(),
                data_base64: "aGVsbG8=".to_string(),
            }),
        );
        assert_eq!(
            serde_json::to_value(&output).expect("serialize"),
            json!({
                "schema_version": 1,
                "session_id": "session-1",
                "sequence": 0,
                "event": {
                    "kind": "output",
                    "session_id": "session-1",
                    "data_base64": "aGVsbG8=",
                },
            })
        );
        assert!(!output.is_terminal());

        let exit = TerminalStreamEnvelope::new(
            "session-1",
            1,
            TerminalStreamEvent::Exit(TerminalExitEvent {
                session_id: "session-1".to_string(),
                exit_code: Some(0),
            }),
        );
        assert_eq!(
            serde_json::to_value(&exit).expect("serialize")["event"],
            json!({"kind": "exit", "session_id": "session-1", "exit_code": 0})
        );
        assert!(exit.is_terminal());
    }

    #[test]
    fn the_stream_envelope_round_trips_and_tags_the_terminal_outcomes() {
        let envelope = TerminalStreamEnvelope::new(
            "session-1",
            7,
            TerminalStreamEvent::Output(TerminalOutputEvent {
                session_id: "session-1".to_string(),
                data_base64: "AA==".to_string(),
            }),
        );
        let value = serde_json::to_value(&envelope).expect("serialize");
        let parsed: TerminalStreamEnvelope = serde_json::from_value(value).expect("deserialize");
        assert_eq!(parsed, envelope);

        let reset = TerminalStreamEnvelope::new(
            "session-1",
            8,
            TerminalStreamEvent::Reset {
                reason: "stream_lagged".to_string(),
            },
        );
        assert_eq!(
            serde_json::to_value(&reset).expect("serialize")["event"],
            json!({"kind": "reset", "reason": "stream_lagged"})
        );
        assert!(reset.is_terminal());

        let unavailable = TerminalStreamEnvelope::new(
            "session-1",
            9,
            TerminalStreamEvent::Unavailable {
                reason: "session_gone".to_string(),
            },
        );
        assert_eq!(
            serde_json::to_value(&unavailable).expect("serialize")["event"],
            json!({"kind": "unavailable", "reason": "session_gone"})
        );
        assert!(unavailable.is_terminal());
    }
}
