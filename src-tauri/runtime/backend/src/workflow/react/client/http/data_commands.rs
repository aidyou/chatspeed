//! Allowlisted runtime-owned data command router.
//!
//! This extension is mounted into the single canonical control plane. It is
//! intentionally an explicit command allowlist, not a generic RPC surface.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Router;

use super::dto;
use super::server::{with_idempotency, ControlPlaneState};

/// Route template for runtime-owned data commands.
pub const DATA_COMMANDS_PATH: &str = "/control/v1/data-commands/{command}";

/// Builds the data-command extension for the canonical control plane.
pub fn data_command_router() -> Router<ControlPlaneState> {
    Router::new().route(DATA_COMMANDS_PATH, post(dispatch_data_command))
}

async fn dispatch_data_command(
    State(state): State<ControlPlaneState>,
    Path(command): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let Some(_kind) = crate::runtime_data::data_command_kind(&command) else {
        return dto::error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("Unknown data command: {command}"),
        );
    };

    with_idempotency(
        &state,
        &headers,
        &body,
        move |state: ControlPlaneState, body: String| async move {
            let params = match parse_body(&body) {
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

fn parse_body(body: &str) -> Result<serde_json::Value, Response> {
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
            format!("Invalid data command JSON: {error}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_body;

    #[test]
    fn data_command_body_requires_object() {
        assert!(parse_body("").is_ok());
        assert!(parse_body(r#"{"key":true}"#).is_ok());
        assert!(parse_body("[]").is_err());
        assert!(parse_body("not-json").is_err());
    }
}
