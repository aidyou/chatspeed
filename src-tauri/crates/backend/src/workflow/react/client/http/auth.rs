//! Bearer authentication and local-browser rejection for the control plane.
//!
//! Security rules (INV-7):
//! - every request must carry `Authorization: Bearer <token>` with the
//!   per-instance token from the discovery document;
//! - requests with an `Origin` header (browsers) are rejected: a web page must
//!   never be able to read the local token or drive the control plane;
//! - tokens passed in the URL query string are rejected so tokens never end up
//!   in logs or history;
//! - failures return a stable error envelope without echoing the token.

use super::dto;
use super::server::ControlPlaneState;
use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::Response;

/// Query parameter names that must never carry a token.
const FORBIDDEN_TOKEN_QUERY_KEYS: [&str; 3] = ["token", "access_token", "auth"];

/// Constant-time equality comparison for secret material.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn reject(status: StatusCode, code: &'static str, message: String) -> Response {
    dto::error_response(status, code, message)
}

/// Auth middleware: validates bearer token, rejects browsers and URL tokens.
pub async fn require_bearer(
    State(state): State<ControlPlaneState>,
    request: Request,
    next: Next,
) -> Response {
    // Browsers are rejected before anything else: a web page must not be able
    // to probe the control plane even with a leaked token.
    if request.headers().contains_key(header::ORIGIN) {
        return reject(
            StatusCode::FORBIDDEN,
            "origin_forbidden",
            "Browser requests with an Origin header are not allowed on the local control plane"
                .to_string(),
        );
    }

    // Reject tokens in the query string; they must never enter URLs or logs.
    if let Some(query) = request.uri().query() {
        for pair in query.split('&') {
            let name = pair.split('=').next().unwrap_or("");
            if FORBIDDEN_TOKEN_QUERY_KEYS.contains(&name.to_ascii_lowercase().as_str()) {
                return reject(
                    StatusCode::BAD_REQUEST,
                    "token_in_url_forbidden",
                    "Credentials must be sent via the Authorization header, not the URL"
                        .to_string(),
                );
            }
        }
    }

    let authorized = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(|token| constant_time_eq(token, &state.token))
        .unwrap_or(false);

    if !authorized {
        return reject(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Missing or invalid bearer token".to_string(),
        );
    }

    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_matches_only_equal_inputs() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "abcd"));
        assert!(!constant_time_eq("", "a"));
        assert!(constant_time_eq("", ""));
    }
}
