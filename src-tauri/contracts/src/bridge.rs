//! Client WebView capability bridge wire contracts (U-7).
//!
//! The runtime must never link a Tauri WebView, so a capability the desktop
//! owns (`web_fetch`, `web_search`) is reachable only through an explicit
//! client bridge that the desktop opens into the runtime. These types are the
//! single source of truth for that bridge: the desktop declares a closed,
//! web-only capability set, registers a short-lived opaque session bound to a
//! live client lease, pulls typed work envelopes, and returns typed results.
//!
//! The bridge is deliberately not a generic RPC: the capability set is a fixed
//! allowlist, every request carries a capability plus schema version plus a
//! request id, and unknown fields are rejected on the request-shaped DTOs. The
//! session secret travels in a dedicated header, never a URL or body.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Wire protocol major for the bridge transport.
pub const BRIDGE_PROTOCOL_VERSION: &str = "1";

/// Schema version for the bridge envelope DTOs.
pub const BRIDGE_SCHEMA_VERSION: &str = "1";

/// Closed allowlist of capabilities a client bridge may declare or execute.
///
/// A client bridge is web-only by contract, so it can never grow into a
/// general-purpose execution surface.
pub const BRIDGE_CAPABILITY_ALLOWLIST: [&str; 2] = ["web_fetch", "web_search"];

/// One capability a client bridge declares, with the schema version it speaks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct ClientBridgeCapability {
    /// Stable capability name; must be in [`BRIDGE_CAPABILITY_ALLOWLIST`].
    pub name: String,
    /// Schema version the client will accept for this capability.
    pub schema_version: String,
}

/// The fixed declaration a client bridge publishes at registration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct ClientBridgeDeclaration {
    /// Bridge transport protocol major the client speaks.
    pub protocol_version: String,
    /// Bridge envelope schema version the client speaks.
    pub schema_version: String,
    /// The declared capabilities; a subset of the allowlist.
    pub capabilities: Vec<ClientBridgeCapability>,
}

/// `POST /control/v1/client-bridge/register` request body.
///
/// The caller proves identity with the bearer token, its registered client id
/// and the opaque lease id the runtime issued for that lease. The runtime
/// resolves the lease itself and requires a fixed `tauri` client kind, so a
/// body-supplied kind is never trusted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct ClientBridgeRegistration {
    /// Registered client id this bridge belongs to.
    pub client_id: String,
    /// Opaque lease id the runtime issued when the client registered.
    pub lease_id: String,
    /// The closed capability declaration.
    pub declaration: ClientBridgeDeclaration,
}

/// `POST /control/v1/client-bridge/register` response body.
///
/// `session_token` is an opaque secret the client must present in the
/// `X-Bridge-Session` header on every later bridge request. It is never echoed
/// in a URL, a body or a log line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ClientBridgeRegistrationResponse {
    /// Opaque, non-secret session id; used as the `{session_id}` path segment.
    pub session_id: String,
    /// Opaque session secret bound to the client id and lease id.
    pub session_token: String,
    /// RFC 3339 expiry after which the session is no longer accepted.
    pub expires_at: String,
    /// Bridge protocol major the runtime accepted.
    pub protocol_version: String,
    /// Stable fingerprint of the accepted declaration for auditability.
    pub declaration_fingerprint: String,
}

/// One typed capability invocation the runtime dispatches to a client bridge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct ClientCapabilityInvocation {
    /// Runtime-generated request id; the client must echo it on the result.
    pub request_id: String,
    /// Capability to execute; must be declared and in the allowlist.
    pub capability: String,
    /// Schema version the arguments conform to.
    pub schema_version: String,
    /// The exact typed arguments object for the capability.
    pub arguments: Value,
    /// RFC 3339 deadline after which the invocation must not be started.
    pub deadline: String,
}

/// Lifecycle status of one capability invocation result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientCapabilityStatus {
    /// The capability ran and produced a result.
    Ok,
    /// The capability failed with a structured error.
    Error,
    /// The invocation was cancelled before it produced a result.
    Cancelled,
}

/// A structured, redacted capability error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct ClientCapabilityError {
    /// Stable machine-readable error code.
    pub code: String,
    /// Human-readable message; must not contain secrets.
    pub message: String,
}

/// `POST /control/v1/client-bridge/{session_id}/result` request body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct ClientCapabilityResult {
    /// Request id this result answers; ownership is validated by the runtime.
    pub request_id: String,
    /// Terminal status of the invocation.
    pub status: ClientCapabilityStatus,
    /// Typed result payload when `status` is `ok`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Structured error when `status` is `error` or `cancelled`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ClientCapabilityError>,
}

/// One work envelope delivered over the bridge SSE stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ClientBridgeWorkEnvelope {
    /// Bridge envelope schema version.
    pub schema_version: String,
    /// The invocation the client should execute.
    pub invocation: ClientCapabilityInvocation,
}

/// `POST /control/v1/client-bridge/{session_id}/cancel` request body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct ClientBridgeCancelRequest {
    /// Request id the client is cancelling.
    pub request_id: String,
}

/// `POST /control/v1/client-bridge/{session_id}/unregister` request body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct ClientBridgeUnregisterRequest {
    /// Optional non-secret reason recorded for diagnostics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn registration_fixture_uses_stable_snake_case_wire_names() {
        let registration = ClientBridgeRegistration {
            client_id: "tauri-main".to_string(),
            lease_id: "lease-1".to_string(),
            declaration: ClientBridgeDeclaration {
                protocol_version: BRIDGE_PROTOCOL_VERSION.to_string(),
                schema_version: BRIDGE_SCHEMA_VERSION.to_string(),
                capabilities: vec![ClientBridgeCapability {
                    name: "web_fetch".to_string(),
                    schema_version: "1".to_string(),
                }],
            },
        };
        assert_eq!(
            serde_json::to_value(&registration).expect("serialize registration"),
            json!({
                "client_id": "tauri-main",
                "lease_id": "lease-1",
                "declaration": {
                    "protocol_version": "1",
                    "schema_version": "1",
                    "capabilities": [{"name": "web_fetch", "schema_version": "1"}]
                }
            })
        );
    }

    #[test]
    fn invocation_and_result_fixtures_round_trip() {
        let invocation = ClientCapabilityInvocation {
            request_id: "req-1".to_string(),
            capability: "web_search".to_string(),
            schema_version: "1".to_string(),
            arguments: json!({"query": "rust"}),
            deadline: "unix-100".to_string(),
        };
        let value = serde_json::to_value(&invocation).expect("serialize invocation");
        assert_eq!(
            value,
            json!({
                "request_id": "req-1",
                "capability": "web_search",
                "schema_version": "1",
                "arguments": {"query": "rust"},
                "deadline": "unix-100"
            })
        );
        assert_eq!(
            serde_json::from_value::<ClientCapabilityInvocation>(value).expect("decode"),
            invocation
        );

        let result = ClientCapabilityResult {
            request_id: "req-1".to_string(),
            status: ClientCapabilityStatus::Ok,
            result: Some(json!({"items": []})),
            error: None,
        };
        let value = serde_json::to_value(&result).expect("serialize result");
        assert_eq!(
            value,
            json!({
                "request_id": "req-1",
                "status": "ok",
                "result": {"items": []}
            })
        );
    }

    #[test]
    fn request_dtos_reject_unknown_fields() {
        assert!(serde_json::from_value::<ClientBridgeRegistration>(json!({
            "client_id": "a",
            "lease_id": "b",
            "declaration": {
                "protocol_version": "1",
                "schema_version": "1",
                "capabilities": []
            },
            "client_kind": "tauri"
        }))
        .is_err());
        assert!(serde_json::from_value::<ClientCapabilityInvocation>(json!({
            "request_id": "r",
            "capability": "web_fetch",
            "schema_version": "1",
            "arguments": {},
            "deadline": "unix-1",
            "extra": true
        }))
        .is_err());
        assert!(serde_json::from_value::<ClientBridgeCancelRequest>(json!({
            "request_id": "r",
            "extra": 1
        }))
        .is_err());
    }
}
