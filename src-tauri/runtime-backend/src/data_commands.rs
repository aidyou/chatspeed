//! Allowlisted data-command router for the runtime control plane.
//!
//! Every runtime-owned data command (agent ordering/tool metadata, proxy groups,
//! notes, conversations/messages, sandbox schemes, sensitive config, ChatHubs,
//! ccproxy statistics, configuration transfer, settings/models/skills/backups)
//! is served from one explicit route,
//! `POST /control/v1/data-commands/{command}`, whose body is a strongly typed
//! parameter object and whose response preserves the historical Tauri camelCase
//! JSON verbatim.
//!
//! The command set is the allowlist in [`crate::runtime_data::data_command_kind`];
//! an arbitrary path segment can never reach a handler. Mutations run through
//! the canonical server `with_idempotency` (no second journal), so they inherit
//! the same single-flight and replay semantics as every other control-plane
//! mutation, while reads execute directly because the desktop client sends no
//! `Idempotency-Key` for them.
//!
//! # Mounting
//!
//! The canonical server merges this extension into its state-typed router before
//! the auth / body-limit layers and `with_state`, in the desktop-free build:
//!
//! ```ignore
//! #[cfg(not(feature = "desktop"))]
//! let router = router.merge(
//!     chatspeed_runtime_backend::data_commands::data_command_router(),
//! );
//! ```

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Router;

use crate::workflow::react::client::http::dto;
use crate::workflow::react::client::http::server::{with_idempotency, ControlPlaneState};

/// The data-command route template.
pub const DATA_COMMANDS_PATH: &str = "/control/v1/data-commands/{command}";

/// Builds the router extension for the allowlisted data commands.
pub fn data_command_router() -> Router<ControlPlaneState> {
    Router::new().route(DATA_COMMANDS_PATH, post(dispatch_data_command_handler))
}

/// `POST /control/v1/data-commands/{command}`.
async fn dispatch_data_command_handler(
    State(state): State<ControlPlaneState>,
    Path(command): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Response {
    if crate::runtime_data::data_command_kind(&command).is_none() {
        return dto::error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("Unknown data command: {command}"),
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
            match crate::runtime_data::dispatch_data_command(state.svc.as_ref(), &command, params)
                .await
            {
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
            "The data command body must be a JSON object".to_string(),
        )),
        Err(error) => Err(dto::error_response(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            format!("Invalid data command body: {error}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
        let params = parse_command_body(r#"{"id":42}"#).expect("object body");
        assert_eq!(params["id"], json!(42));
    }

    #[test]
    fn the_allowlist_rejects_unknown_commands() {
        assert!(crate::runtime_data::data_command_kind("get_note").is_some());
        assert!(crate::runtime_data::data_command_kind("drop_everything").is_none());
    }
}
