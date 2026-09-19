//! HTTP v1 DTOs and error mapping for the workflow control plane.
//!
//! The HTTP wire is canonical snake_case (decision D-7). Tauri keeps its
//! camelCase wire; conversion happens only at adapter boundaries. IDs and
//! cursors are always strings on the wire.

use crate::workflow::react::application::{ApplicationError, ApplicationErrorKind};
use crate::workflow::react::experiment::ExperimentRunSpecV1;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Control-plane protocol version (v1).
pub const PROTOCOL_VERSION: &str = "1";

/// Strict request body for `POST /control/v1/experiments:run`. The agent and
/// prompt are supplied here; the frozen budget lives in the strict spec. The
/// backend mints all scope/effect/attempt identity, so the body can never
/// carry it (INV-2). Unknown fields are rejected.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct ExperimentRunHttpRequest {
    pub agent_id: String,
    pub prompt: String,
    pub spec: ExperimentRunSpecV1,
}

/// `GET /control/v1/meta` response.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct MetaResponse {
    pub service: &'static str,
    pub protocol_version: &'static str,
    pub schema_version: u32,
    pub server_instance_id: String,
    pub pid: u32,
}

/// Stable error envelope: `{"error": {"code": "...", "message": "..."}}`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ErrorEnvelope {
    pub error: ErrorDetail,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ErrorDetail {
    pub code: &'static str,
    pub message: String,
}

/// Builds a stable error response from a domain error.
pub fn error_response(status: StatusCode, code: &'static str, message: String) -> Response {
    let body = ErrorEnvelope {
        error: ErrorDetail { code, message },
    };
    (status, axum::Json(body)).into_response()
}

/// Maps a domain [`ApplicationError`] to a stable HTTP status and code.
pub fn application_error_response(error: &ApplicationError) -> Response {    let (status, code) = match error.kind {
        ApplicationErrorKind::NotFound => (StatusCode::NOT_FOUND, "not_found"),
        ApplicationErrorKind::InvalidInput => (StatusCode::BAD_REQUEST, "invalid_input"),
        ApplicationErrorKind::Conflict => (StatusCode::CONFLICT, "conflict"),
        ApplicationErrorKind::State => (StatusCode::CONFLICT, "invalid_state"),
        ApplicationErrorKind::Gateway => (StatusCode::CONFLICT, "gateway_unavailable"),
        ApplicationErrorKind::Internal => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
    };
    error_response(status, code, error.message.clone())
}

/// Maps a capability error to a stable HTTP status and code.
///
/// The capability error code is preserved on the wire as the envelope `code`,
/// so a client branches on the same value the service produced. The message is
/// the redacted one: a capability error can quote an upstream failure and must
/// never be able to leak a secret through the HTTP plane (AC-13).
pub fn capability_error_response(
    error: &crate::capability::error::CapabilityError,
) -> Response {
    let (status, code) = match error.code() {
        crate::capability::error::code::INVALID_REQUEST => {
            (StatusCode::BAD_REQUEST, "invalid_request")
        }
        crate::capability::error::code::IDEMPOTENCY_KEY_REQUIRED => {
            (StatusCode::BAD_REQUEST, "idempotency_key_required")
        }
        crate::capability::error::code::IDEMPOTENCY_KEY_CONFLICT => {
            (StatusCode::CONFLICT, "idempotency_key_conflict")
        }
        crate::capability::error::code::OPERATION_NOT_FOUND => {
            (StatusCode::NOT_FOUND, "operation_not_found")
        }
        crate::capability::error::code::NOT_FOUND => (StatusCode::NOT_FOUND, "not_found"),
        crate::capability::error::code::UNSUPPORTED_TARGET => {
            (StatusCode::UNPROCESSABLE_ENTITY, "unsupported_target")
        }
        crate::capability::error::code::UNSUPPORTED_ADAPTER => {
            (StatusCode::UNPROCESSABLE_ENTITY, "unsupported_adapter")
        }
        crate::capability::error::code::CHECK_BLOCKED => {
            (StatusCode::UNPROCESSABLE_ENTITY, "check_blocked")
        }
        crate::capability::error::code::CHECK_INCONCLUSIVE => {
            (StatusCode::UNPROCESSABLE_ENTITY, "check_inconclusive")
        }
        crate::capability::error::code::REFUSED => (StatusCode::CONFLICT, "refused"),
        crate::capability::error::code::FORBIDDEN => (StatusCode::FORBIDDEN, "forbidden"),
        crate::capability::error::code::NEEDS_RECONCILE => {
            (StatusCode::CONFLICT, "needs_reconcile")
        }
        // An interrupted operation is a recoverable answer about the operation,
        // not a server fault: reporting it as 500 would hide the retry path.
        crate::capability::error::code::INTERRUPTED_BEFORE_EFFECT => (
            StatusCode::CONFLICT,
            "interrupted_before_effect",
        ),
        crate::capability::error::code::EFFECT_STATE_UNKNOWN => {
            (StatusCode::CONFLICT, "effect_state_unknown")
        }
        // No runtime in this process means the effect cannot be performed or
        // observed; 503 says "try a process that owns the runtime".
        crate::capability::error::code::RUNTIME_UNAVAILABLE => {
            (StatusCode::SERVICE_UNAVAILABLE, "runtime_unavailable")
        }
        crate::capability::error::code::BUSY => (StatusCode::CONFLICT, "busy"),
        crate::capability::error::code::PARTIAL => (StatusCode::CONFLICT, "partial"),
        crate::capability::error::code::STORE_ERROR => {
            (StatusCode::INTERNAL_SERVER_ERROR, "store_error")
        }
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
    };
    error_response(status, code, error.redacted_message())
}

/// Converts camelCase JSON keys to snake_case, recursively.
///
/// Used at the HTTP boundary for payloads that come from Tauri-shaped
/// structures (`Workflow`, `WorkflowEventRecord`, snapshot JSON). Values are
/// never modified — only object keys are normalized.
pub fn to_snake_case_keys(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut converted = Map::with_capacity(map.len());
            for (key, val) in map {
                converted.insert(camel_to_snake(&key), to_snake_case_keys(val));
            }
            Value::Object(converted)
        }
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(to_snake_case_keys)
                .collect::<Vec<_>>(),
        ),
        other => other,
    }
}

/// Converts a single camelCase identifier to snake_case.
pub fn camel_to_snake(input: &str) -> String {
    let mut output = String::with_capacity(input.len() + 4);
    for (index, ch) in input.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if index > 0 {
                output.push('_');
            }
            output.push(ch.to_ascii_lowercase());
        } else {
            output.push(ch);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn camel_to_snake_converts_identifiers() {
        assert_eq!(camel_to_snake("sessionId"), "session_id");
        assert_eq!(camel_to_snake("agentId"), "agent_id");
        assert_eq!(camel_to_snake("already_snake"), "already_snake");
        assert_eq!(camel_to_snake("id"), "id");
    }

    #[test]
    fn to_snake_case_keys_converts_nested_objects() {
        let value = json!({
            "sessionId": "s1",
            "nested": { "toolCallId": "t1", "keep": [ {"innerKey": 1} ] },
            "id": 7
        });
        let converted = to_snake_case_keys(value);
        assert_eq!(
            converted,
            json!({
                "session_id": "s1",
                "nested": { "tool_call_id": "t1", "keep": [ {"inner_key": 1} ] },
                "id": 7
            })
        );
    }

    #[test]
    fn application_errors_map_to_stable_status_codes() {
        let response = application_error_response(&ApplicationError::not_found("missing"));
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let response = application_error_response(&ApplicationError::invalid_input("bad"));
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let response = application_error_response(&ApplicationError::internal("boom"));
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
