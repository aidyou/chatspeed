use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};

use crate::ccproxy::adapter::unified::UnifiedErrorResponse;

pub(super) fn openai_error_response(error: UnifiedErrorResponse) -> Response {
    let (status, error_type) = openai_error_mapping(error.status_code);
    let code = error
        .code
        .unwrap_or_else(|| Value::String(error.status_code.to_string()));
    let body = json!({
        "error": {
            "message": error.message,
            "type": error_type,
            "param": null,
            "code": code
        }
    });

    (status, Json(body)).into_response()
}

pub(super) fn claude_error_response(error: UnifiedErrorResponse) -> Response {
    let (status, error_type) = claude_error_mapping(error.status_code);
    let mut body = json!({
        "type": "error",
        "error": {
            "type": error_type,
            "message": error.message
        }
    });
    if let (Some(request_id), Some(body_object)) = (error.request_id, body.as_object_mut()) {
        body_object.insert("request_id".to_string(), Value::String(request_id));
    }

    (status, Json(body)).into_response()
}

pub(super) fn gemini_error_response(error: UnifiedErrorResponse) -> Response {
    let (status, error_status) = gemini_error_mapping(error.status_code);
    let body = json!({
        "error": {
            "code": status.as_u16(),
            "message": error.message,
            "status": error_status
        }
    });

    (status, Json(body)).into_response()
}

pub(super) fn ollama_error_response(error: UnifiedErrorResponse) -> Response {
    let status = status_code_or_internal(error.status_code);
    (status, Json(json!({ "error": error.message }))).into_response()
}

fn openai_error_mapping(status_code: u16) -> (StatusCode, &'static str) {
    let status = status_code_or_internal(status_code);
    match status_code {
        401 => (StatusCode::UNAUTHORIZED, "authentication_error"),
        403 => (StatusCode::FORBIDDEN, "permission_error"),
        429 => (StatusCode::TOO_MANY_REQUESTS, "rate_limit_error"),
        529 => (StatusCode::SERVICE_UNAVAILABLE, "server_error"),
        _ if status.is_client_error() => (status, "invalid_request_error"),
        _ if status.is_server_error() => (status, "server_error"),
        _ => (status, "upstream_api_error"),
    }
}

fn claude_error_mapping(status_code: u16) -> (StatusCode, &'static str) {
    match status_code {
        401 => (StatusCode::UNAUTHORIZED, "authentication_error"),
        402 => (StatusCode::PAYMENT_REQUIRED, "billing_error"),
        403 => (StatusCode::FORBIDDEN, "permission_error"),
        404 => (StatusCode::NOT_FOUND, "not_found_error"),
        409 => (StatusCode::CONFLICT, "conflict_error"),
        413 => (StatusCode::PAYLOAD_TOO_LARGE, "request_too_large"),
        429 => (StatusCode::TOO_MANY_REQUESTS, "rate_limit_error"),
        408 | 504 => (StatusCode::GATEWAY_TIMEOUT, "timeout_error"),
        503 | 529 => (status_code_or_internal(529), "overloaded_error"),
        500..=599 => (StatusCode::INTERNAL_SERVER_ERROR, "api_error"),
        _ => (StatusCode::BAD_REQUEST, "invalid_request_error"),
    }
}

fn gemini_error_mapping(status_code: u16) -> (StatusCode, &'static str) {
    match status_code {
        401 => (StatusCode::UNAUTHORIZED, "UNAUTHENTICATED"),
        403 => (StatusCode::FORBIDDEN, "PERMISSION_DENIED"),
        404 => (StatusCode::NOT_FOUND, "NOT_FOUND"),
        409 => (StatusCode::CONFLICT, "ABORTED"),
        429 => (StatusCode::TOO_MANY_REQUESTS, "RESOURCE_EXHAUSTED"),
        501 => (StatusCode::NOT_IMPLEMENTED, "UNIMPLEMENTED"),
        408 | 504 => (StatusCode::GATEWAY_TIMEOUT, "DEADLINE_EXCEEDED"),
        502 | 503 | 529 => (StatusCode::SERVICE_UNAVAILABLE, "UNAVAILABLE"),
        500..=599 => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL"),
        _ => (StatusCode::BAD_REQUEST, "INVALID_ARGUMENT"),
    }
}

fn status_code_or_internal(status_code: u16) -> StatusCode {
    StatusCode::from_u16(status_code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

#[cfg(test)]
mod tests {
    use super::{
        claude_error_response, gemini_error_response, ollama_error_response, openai_error_response,
    };
    use crate::ccproxy::adapter::unified::UnifiedErrorResponse;
    use axum::body::to_bytes;
    use http::StatusCode;
    use serde_json::{json, Value};

    fn error(status_code: u16) -> UnifiedErrorResponse {
        UnifiedErrorResponse {
            status_code,
            message: "Capacity exhausted".to_string(),
            error_type: Some("Service Unavailable".to_string()),
            code: Some(json!(status_code)),
            request_id: Some("req_test".to_string()),
        }
    }

    async fn response_json(response: axum::response::Response) -> Value {
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body should be readable");
        serde_json::from_slice(&body).expect("response body should be valid JSON")
    }

    #[tokio::test]
    async fn maps_openai_overload_to_server_error() {
        let response = openai_error_response(error(529));
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            response_json(response).await,
            json!({
                "error": {
                    "message": "Capacity exhausted",
                    "type": "server_error",
                    "param": null,
                    "code": 529
                }
            })
        );
    }

    #[tokio::test]
    async fn maps_backend_503_to_claude_overloaded_error() {
        let response = claude_error_response(error(503));
        assert_eq!(response.status().as_u16(), 529);
        assert_eq!(
            response_json(response).await,
            json!({
                "type": "error",
                "error": {
                    "type": "overloaded_error",
                    "message": "Capacity exhausted"
                },
                "request_id": "req_test"
            })
        );
    }

    #[tokio::test]
    async fn maps_backend_529_to_gemini_unavailable_error() {
        let response = gemini_error_response(error(529));
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            response_json(response).await,
            json!({
                "error": {
                    "code": 503,
                    "message": "Capacity exhausted",
                    "status": "UNAVAILABLE"
                }
            })
        );
    }

    #[tokio::test]
    async fn emits_ollama_error_shape() {
        let response = ollama_error_response(error(503));
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            response_json(response).await,
            json!({ "error": "Capacity exhausted" })
        );
    }

    #[test]
    fn maps_all_documented_claude_error_statuses() {
        let cases = [
            (400, 400, "invalid_request_error"),
            (401, 401, "authentication_error"),
            (402, 402, "billing_error"),
            (403, 403, "permission_error"),
            (404, 404, "not_found_error"),
            (409, 409, "conflict_error"),
            (413, 413, "request_too_large"),
            (429, 429, "rate_limit_error"),
            (500, 500, "api_error"),
            (504, 504, "timeout_error"),
            (529, 529, "overloaded_error"),
        ];

        for (upstream_status, expected_status, expected_type) in cases {
            let (status, error_type) = super::claude_error_mapping(upstream_status);
            assert_eq!(status.as_u16(), expected_status);
            assert_eq!(error_type, expected_type);
        }
    }

    #[test]
    fn maps_common_gemini_error_statuses() {
        let cases = [
            (400, 400, "INVALID_ARGUMENT"),
            (403, 403, "PERMISSION_DENIED"),
            (429, 429, "RESOURCE_EXHAUSTED"),
            (500, 500, "INTERNAL"),
            (503, 503, "UNAVAILABLE"),
            (504, 504, "DEADLINE_EXCEEDED"),
        ];

        for (upstream_status, expected_status, expected_type) in cases {
            let (status, error_type) = super::gemini_error_mapping(upstream_status);
            assert_eq!(status.as_u16(), expected_status);
            assert_eq!(error_type, expected_type);
        }
    }
}
