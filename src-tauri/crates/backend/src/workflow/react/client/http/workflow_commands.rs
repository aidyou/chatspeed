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
//! inherits the same bearer auth and the same instance-local idempotency as every
//! other mutation. The canonical server gives this route family the bounded
//! large-payload body limit, because it is the route that carries
//! `add_workflow_message`; read-only commands are excluded from the replay cache.
//!
//! # Mounting
//!
//! The canonical server mounts this extension with one line, before the auth /
//! body-limit layers and `with_state`:
//!
//! ```ignore
//! let router = Router::new()
//!     // ... existing routes ...
//!     .merge(
//!         crate::workflow::react::client::http::workflow_commands::workflow_command_router()
//!             .layer(DefaultBodyLimit::max(LARGE_PAYLOAD_BODY_BYTES)),
//!     );
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
use super::server::{with_idempotency_named, ControlPlaneState};

/// The compatibility command route template.
pub const WORKFLOW_COMMANDS_PATH: &str = "/control/v1/workflow-commands/{command}";

/// Builds the router extension for the allowlisted workflow commands.
///
/// Returns a `Router<ControlPlaneState>` so the canonical server can `merge` it
/// into its own state-typed router before `with_state`.
pub fn workflow_command_router() -> Router<ControlPlaneState> {
    Router::new().route(WORKFLOW_COMMANDS_PATH, post(dispatch_workflow_command))
}

/// Read-only commands that must not pass through the mutation idempotency
/// replay cache.
///
/// A read can legitimately return a JSON body larger than the cache's bounded
/// replay size; routing it through the cache would truncate that body into an
/// empty success and would also turn a reused key with different parameters into
/// a spurious `409 conflict`. Every entry here was verified to perform no durable
/// mutation, so bypassing the cache is safe. Mutating commands stay on
/// `with_idempotency`.
const READ_ONLY_WORKFLOW_COMMANDS: [&str; 11] = [
    "get_earlier_workflow_message_page",
    "get_earlier_workflow_messages",
    "list_pending_sub_agent_approvals",
    "get_workflow_agent_config",
    "workflow_get_tasks",
    "search_workspace_files",
    "get_system_skills",
    "get_workflow_session_key",
    "get_auto_approved_tools",
    "get_workflow_dispatcher_metrics",
    "get_workflow_efficiency_report",
];

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

    if READ_ONLY_WORKFLOW_COMMANDS.contains(&command.as_str()) {
        let params = match parse_command_body(&body) {
            Ok(params) => params,
            Err(response) => return response,
        };
        return match state.svc.workflow_command(&command, params).await {
            Ok(value) => axum::Json::<serde_json::Value>(value).into_response(),
            Err(error) => dto::application_error_response(&error),
        };
    }

    with_idempotency_named(
        &state,
        &headers,
        &body,
        "workflow.command",
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
    fn history_pages_bypass_mutation_idempotency_replay() {
        assert!(READ_ONLY_WORKFLOW_COMMANDS.contains(&"get_earlier_workflow_message_page"));
        assert!(READ_ONLY_WORKFLOW_COMMANDS.contains(&"get_earlier_workflow_messages"));
        assert!(!READ_ONLY_WORKFLOW_COMMANDS.contains(&"update_workflow_status"));
    }

    #[test]
    fn every_read_only_command_bypasses_the_cache_and_no_mutation_does() {
        // Each bypass entry is a real allowlisted command.
        for command in READ_ONLY_WORKFLOW_COMMANDS {
            assert!(
                crate::commands::workflow::is_known_workflow_command(command),
                "{command} is not an allowlisted workflow command"
            );
        }

        // Every verified pure read is bypassed.
        for read in [
            "get_earlier_workflow_message_page",
            "get_earlier_workflow_messages",
            "list_pending_sub_agent_approvals",
            "get_workflow_agent_config",
            "workflow_get_tasks",
            "search_workspace_files",
            "get_system_skills",
            "get_workflow_session_key",
            "get_auto_approved_tools",
            "get_workflow_dispatcher_metrics",
            "get_workflow_efficiency_report",
        ] {
            assert!(
                READ_ONLY_WORKFLOW_COMMANDS.contains(&read),
                "{read} must bypass the replay cache"
            );
        }

        // No command with a durable effect is treated as a read.
        for mutation in [
            "add_workflow_message",
            "update_workflow_status",
            "update_workflow_todo_list",
            "workflow_begin_new_context_frame",
            "workflow_approve_plan",
            "delete_workflow",
            "delete_last_workflow_message",
            "remove_auto_approved_tool",
            "remove_shell_policy_item",
            "update_workflow_agent_id",
        ] {
            assert!(
                !READ_ONLY_WORKFLOW_COMMANDS.contains(&mutation),
                "{mutation} must stay on with_idempotency"
            );
        }
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
