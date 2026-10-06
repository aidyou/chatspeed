//! Allowlisted compatibility command router for the workflow control plane.
//!
//! The seven canonical workflow commands already have first-class
//! `/control/v1/workflows...` routes. The remaining workflow commands are
//! served from one explicit, allowlisted route,
//! `POST /control/v1/workflow-commands/{command}`, whose body is a strongly
//! typed parameter object and whose response preserves the historical Tauri
//! camelCase JSON verbatim.
//!
//! This is a desktop compatibility adapter, not a plugin API and not a second
//! control plane: the router is mounted into the one canonical server, so it
//! inherits the same bearer auth, the same body limit and the same
//! instance-local idempotency as every other mutation.
//!
//! # Mounting
//!
//! The canonical server mounts this extension with one line, before the auth /
//! body-limit layers and `with_state`:
//!
//! ```ignore
//! let router = Router::new()
//!     // ... existing routes ...
//!     .merge(crate::workflow::react::client::http::workflow_commands::workflow_command_router());
//! ```
//!
//! The handler uses the server's `with_idempotency`, which must be visible to
//! this module (`pub(crate)`); see the unit handoff notes.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Router;

use super::dto;
use super::server::{with_idempotency, ControlPlaneState};

/// The compatibility command route template.
pub const WORKFLOW_COMMANDS_PATH: &str = "/control/v1/workflow-commands/{command}";

/// Builds the router extension for the allowlisted workflow commands.
///
/// Returns a `Router<ControlPlaneState>` so the canonical server can `merge` it
/// into its own state-typed router before `with_state`.
pub fn workflow_command_router() -> Router<ControlPlaneState> {
    Router::new().route(WORKFLOW_COMMANDS_PATH, post(dispatch_workflow_command))
}

/// `POST /control/v1/workflow-commands/{command}`.
///
/// The command is validated against the allowlist first, so an arbitrary path
/// segment can never reach a handler. Mutations then run through the canonical
/// idempotency wrapper, and the response keeps the Tauri camelCase shape.
async fn dispatch_workflow_command(
    State(state): State<ControlPlaneState>,
    Path(command): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Response {
    if !crate::commands::workflow::is_known_workflow_command(&command) {
        return dto::error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("Unknown workflow command: {command}"),
        );
    }

    with_idempotency(
        &state,
        &headers,
        &body,
        move |state: ControlPlaneState, body: String| async move {
            let params = match parse_command_body(&body) {
                Ok(params) => params,
                Err(response) => return response,
            };
            match state.svc.workflow_command(&command, params).await {
                Ok(value) => axum::Json::<serde_json::Value>(value).into_response(),
                Err(error) => dto::application_error_response(&error),
            }
        },
    )
    .await
}

/// Parses the request body into a parameter object.
///
/// An empty body is the parameter-less command form; any other body must be a
/// JSON object, since every allowlisted command takes named parameters.
fn parse_command_body(body: &str) -> Result<serde_json::Value, Response> {
    if body.trim().is_empty() {
        return Ok(serde_json::json!({}));
    }
    match serde_json::from_str::<serde_json::Value>(body) {
        Ok(serde_json::Value::Object(map)) => Ok(serde_json::Value::Object(map)),
        Ok(_) => Err(dto::error_response(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            "The workflow command body must be a JSON object".to_string(),
        )),
        Err(error) => Err(dto::error_response(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            format!("Invalid workflow command body: {error}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_allowlist_covers_every_remaining_command_and_rejects_unknown_names() {
        for command in crate::commands::workflow::WORKFLOW_COMMANDS {
            assert!(crate::commands::workflow::is_known_workflow_command(
                command
            ));
        }
        assert!(!crate::commands::workflow::is_known_workflow_command(
            "drop_everything"
        ));
        assert!(!crate::commands::workflow::is_known_workflow_command(""));
    }

    #[test]
    fn an_empty_body_is_the_parameterless_form() {
        assert_eq!(parse_command_body("").expect("empty body"), json!({}));
        assert_eq!(parse_command_body("   ").expect("blank body"), json!({}));
    }

    #[test]
    fn a_non_object_body_is_rejected() {
        assert!(parse_command_body("[1,2,3]").is_err());
        assert!(parse_command_body("\"nope\"").is_err());
        assert!(parse_command_body("{not json}").is_err());
    }

    #[test]
    fn an_object_body_is_accepted_verbatim() {
        let params = parse_command_body(r#"{"session_id":"s1"}"#).expect("object body");
        assert_eq!(params["session_id"], json!("s1"));
    }
}
