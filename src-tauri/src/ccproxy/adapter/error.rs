use http::StatusCode;
use reqwest::header::HeaderMap;
use serde_json::Value;

use crate::ccproxy::{adapter::unified::UnifiedErrorResponse, types::ChatProtocol};

pub fn normalize_backend_error(
    backend_protocol: &ChatProtocol,
    status_code: StatusCode,
    headers: &HeaderMap,
    body: &[u8],
) -> UnifiedErrorResponse {
    let body_text = String::from_utf8_lossy(body).trim().to_string();
    let parsed_body = serde_json::from_slice::<Value>(body).ok();
    let error_value = parsed_body.as_ref().and_then(|value| value.get("error"));

    let message = error_value
        .and_then(error_message)
        .or_else(|| {
            parsed_body
                .as_ref()
                .and_then(|value| value.get("message"))
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
        .filter(|message| !message.is_empty())
        .unwrap_or_else(|| {
            if body_text.is_empty() {
                status_code
                    .canonical_reason()
                    .unwrap_or_default()
                    .to_string()
            } else {
                body_text
            }
        });

    let error_type = match backend_protocol {
        ChatProtocol::Gemini => error_value
            .and_then(|error| error.get("status"))
            .and_then(Value::as_str),
        ChatProtocol::OpenAI | ChatProtocol::HuggingFace | ChatProtocol::Claude => error_value
            .and_then(|error| error.get("type"))
            .and_then(Value::as_str),
        ChatProtocol::Ollama => None,
    }
    .map(ToString::to_string);

    let code = error_value.and_then(|error| error.get("code")).cloned();
    let request_id = parsed_body
        .as_ref()
        .and_then(|value| value.get("request_id"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .or_else(|| request_id_from_headers(headers));

    UnifiedErrorResponse {
        status_code: status_code.as_u16(),
        message,
        error_type,
        code,
        request_id,
    }
}

fn error_message(error: &Value) -> Option<String> {
    match error {
        Value::String(message) => Some(message.clone()),
        Value::Object(_) => error
            .get("message")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        _ => None,
    }
}

fn request_id_from_headers(headers: &HeaderMap) -> Option<String> {
    ["request-id", "x-request-id", "x-amzn-requestid"]
        .into_iter()
        .find_map(|name| {
            headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(ToString::to_string)
        })
}

#[cfg(test)]
mod tests {
    use super::normalize_backend_error;
    use crate::ccproxy::{
        adapter::output::{ClaudeOutputAdapter, OutputAdapterEnum},
        types::ChatProtocol,
    };
    use axum::body::to_bytes;
    use http::StatusCode;
    use reqwest::header::{HeaderMap, HeaderValue};
    use serde_json::json;

    #[test]
    fn normalizes_openai_error() {
        let body = br#"{"error":{"message":"Worker limit reached","type":"Service Unavailable","code":503}}"#;
        let error = normalize_backend_error(
            &ChatProtocol::OpenAI,
            StatusCode::SERVICE_UNAVAILABLE,
            &HeaderMap::new(),
            body,
        );

        assert_eq!(error.status_code, 503);
        assert_eq!(error.message, "Worker limit reached");
        assert_eq!(error.error_type.as_deref(), Some("Service Unavailable"));
        assert_eq!(error.code, Some(json!(503)));
    }

    #[test]
    fn normalizes_claude_error_and_request_id() {
        let body = br#"{"type":"error","error":{"type":"rate_limit_error","message":"Too many requests"},"request_id":"req_body"}"#;
        let error = normalize_backend_error(
            &ChatProtocol::Claude,
            StatusCode::TOO_MANY_REQUESTS,
            &HeaderMap::new(),
            body,
        );

        assert_eq!(error.message, "Too many requests");
        assert_eq!(error.error_type.as_deref(), Some("rate_limit_error"));
        assert_eq!(error.request_id.as_deref(), Some("req_body"));
    }

    #[test]
    fn normalizes_gemini_error() {
        let body =
            br#"{"error":{"code":503,"message":"Capacity exhausted","status":"UNAVAILABLE"}}"#;
        let error = normalize_backend_error(
            &ChatProtocol::Gemini,
            StatusCode::SERVICE_UNAVAILABLE,
            &HeaderMap::new(),
            body,
        );

        assert_eq!(error.message, "Capacity exhausted");
        assert_eq!(error.error_type.as_deref(), Some("UNAVAILABLE"));
        assert_eq!(error.code, Some(json!(503)));
    }

    #[test]
    fn normalizes_ollama_error_and_header_request_id() {
        let mut headers = HeaderMap::new();
        headers.insert("x-request-id", HeaderValue::from_static("req_header"));
        let error = normalize_backend_error(
            &ChatProtocol::Ollama,
            StatusCode::INTERNAL_SERVER_ERROR,
            &headers,
            br#"{"error":"model runner failed"}"#,
        );

        assert_eq!(error.message, "model runner failed");
        assert_eq!(error.error_type, None);
        assert_eq!(error.request_id.as_deref(), Some("req_header"));
    }

    #[tokio::test]
    async fn converts_openai_503_to_claude_overload_response() {
        let body = br#"{"error":{"message":"ResourceExhausted: Worker local total request limit reached (65/48)","type":"Service Unavailable","code":503}}"#;
        let error = normalize_backend_error(
            &ChatProtocol::OpenAI,
            StatusCode::SERVICE_UNAVAILABLE,
            &HeaderMap::new(),
            body,
        );
        let response = OutputAdapterEnum::Claude(ClaudeOutputAdapter).adapt_error_response(error);

        assert_eq!(response.status().as_u16(), 529);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body should be readable");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body)
                .expect("response body should be valid JSON"),
            json!({
                "type": "error",
                "error": {
                    "type": "overloaded_error",
                    "message": "ResourceExhausted: Worker local total request limit reached (65/48)"
                }
            })
        );
    }
}
