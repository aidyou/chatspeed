//! Loopback Web MCP provider wire contracts (AC-8).
//!
//! The runtime must never link a Tauri WebView, so the two fixed web tools
//! (`web_fetch`, `web_search`) are executed by a dedicated MCP provider the
//! desktop hosts on an ephemeral `127.0.0.1` port. The desktop registers that
//! provider with the runtime over the control plane; the runtime then reaches it
//! as an ordinary streamable-HTTP MCP server.
//!
//! These types are the single source of truth for that handshake:
//!
//! - the registration body carries **only** the loopback port, so a client can
//!   never name an arbitrary host, path or endpoint;
//! - the provider proof token, the client id and the lease id travel in
//!   dedicated headers, never in the URL, query string, body or a log line;
//! - the runtime always derives the endpoint as
//!   `http://127.0.0.1:<port>/mcp`;
//! - the server name and the two public aliases are reserved so an ordinary
//!   user-configured MCP server can never occupy them.

use serde::{Deserialize, Serialize};

/// Wire protocol major for the provider control-plane handshake.
pub const WEB_MCP_PROTOCOL_VERSION: &str = "1";

/// Envelope schema version for the provider DTOs.
pub const WEB_MCP_SCHEMA_VERSION: &str = "1";

/// Reserved canonical MCP server name for the desktop Web MCP provider.
///
/// Ordinary, user-configured MCP servers must never claim this name; the runtime
/// reserves it for the single lease-bound desktop provider.
pub const WEB_MCP_SERVER_NAME: &str = "chatspeed_web";

/// Fixed MCP endpoint path the runtime appends to the loopback authority.
pub const WEB_MCP_ENDPOINT_PATH: &str = "/mcp";

/// The two public tool aliases the provider exposes, exactly.
pub const WEB_MCP_ALIASES: [&str; 2] = ["web_fetch", "web_search"];

/// Loopback host the runtime always dials for the provider.
pub const WEB_MCP_LOOPBACK_HOST: &str = "127.0.0.1";

/// Control-plane route that registers the desktop provider.
pub const WEB_MCP_REGISTER_PATH: &str = "/control/v1/web-mcp/register";

/// Control-plane route that unregisters the desktop provider.
pub const WEB_MCP_UNREGISTER_PATH: &str = "/control/v1/web-mcp/unregister";

/// Control-plane route that reports the current provider slot.
pub const WEB_MCP_STATUS_PATH: &str = "/control/v1/web-mcp/status";

/// Header carrying the opaque provider proof token. Header only.
pub const WEB_MCP_PROOF_HEADER: &str = "X-Web-Mcp-Proof";

/// Header carrying the registering client id.
pub const WEB_MCP_CLIENT_HEADER: &str = "X-Web-Mcp-Client";

/// Header carrying the registering lease id.
pub const WEB_MCP_LEASE_HEADER: &str = "X-Web-Mcp-Lease";

/// Header carrying the desktop instance id the provider bound itself to.
pub const WEB_MCP_INSTANCE_HEADER: &str = "X-Web-Mcp-Instance";

/// Stable code: the provider slot is already owned by another live desktop.
pub const WEB_MCP_CODE_CONFLICT: &str = "conflict";

/// Stable code: the proof token or lease identity did not match.
pub const WEB_MCP_CODE_FORBIDDEN: &str = "forbidden";

/// Stable code: the registration body named an unusable port.
pub const WEB_MCP_CODE_INVALID_PORT: &str = "invalid_port";

/// Stable code: the caller has no live lease.
pub const WEB_MCP_CODE_LEASE_INVALID: &str = "lease_invalid";

/// Stable code: no provider is installed.
pub const WEB_MCP_CODE_UNAVAILABLE: &str = "unavailable";

/// `POST /control/v1/web-mcp/register` request body.
///
/// Only the loopback port is accepted. The runtime derives the endpoint itself,
/// so a client cannot request an arbitrary authority, path or query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct WebMcpProviderRegistration {
    /// The ephemeral loopback TCP port the desktop provider bound.
    pub port: u16,
}

/// `POST /control/v1/web-mcp/register` response body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct WebMcpProviderRegistrationResponse {
    /// Reserved canonical server name the runtime installed.
    pub server_name: String,
    /// Monotonic slot generation; a later registration for the same lease may
    /// replace the provider and bump this value.
    pub generation: u64,
    /// RFC 3339 lease expiry after which the provider is dropped automatically.
    pub expires_at: String,
}

/// Redacted view of the installed provider slot.
///
/// Never carries the proof token or any derived secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct WebMcpProviderStatus {
    /// Reserved canonical server name.
    pub server_name: String,
    /// Monotonic slot generation.
    pub generation: u64,
    /// Desktop instance id the provider bound itself to.
    pub instance_id: String,
    /// Client id that registered the provider.
    pub client_id: String,
    /// Opaque lease id the provider is bound to.
    pub lease_id: String,
    /// Loopback port the runtime dials.
    pub port: u16,
    /// RFC 3339 lease expiry.
    pub expires_at: String,
}

/// Structured provider control error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct WebMcpProviderError {
    /// Stable machine-readable error code.
    pub code: String,
    /// Human-readable message; must not contain secrets.
    pub message: String,
}

impl WebMcpProviderError {
    /// Builds a structured provider error.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

/// Builds the canonical loopback endpoint the runtime dials for `port`.
///
/// The host, scheme and path are fixed; only the port is variable, so a
/// registration can never redirect the runtime to another authority.
pub fn web_mcp_endpoint(port: u16) -> String {
    format!(
        "http://{}:{}{}",
        WEB_MCP_LOOPBACK_HOST, port, WEB_MCP_ENDPOINT_PATH
    )
}

/// Validates a registration port.
///
/// Port `0` is the ephemeral bind request, never a bound port, so it is not a
/// usable provider authority.
pub fn validate_web_mcp_port(port: u16) -> Result<(), WebMcpProviderError> {
    if port == 0 {
        return Err(WebMcpProviderError::new(
            WEB_MCP_CODE_INVALID_PORT,
            "the provider port must be a bound loopback port and must not be 0",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn registration_body_carries_only_the_port() {
        let registration = WebMcpProviderRegistration { port: 41234 };
        assert_eq!(
            serde_json::to_value(registration).expect("serialize registration"),
            json!({ "port": 41234 })
        );
    }

    #[test]
    fn registration_body_rejects_unknown_fields() {
        let error = serde_json::from_value::<WebMcpProviderRegistration>(json!({
            "port": 41234,
            "url": "http://evil.example/mcp"
        }))
        .expect_err("an unknown field is rejected");
        assert!(error.to_string().contains("unknown field"), "{error}");
    }

    #[test]
    fn endpoint_is_always_loopback_with_fixed_path() {
        assert_eq!(web_mcp_endpoint(5555), "http://127.0.0.1:5555/mcp");
        // No host, scheme or path input exists: the only variable is the port.
        assert!(web_mcp_endpoint(1).starts_with("http://127.0.0.1:"));
        assert!(web_mcp_endpoint(1).ends_with(WEB_MCP_ENDPOINT_PATH));
    }

    #[test]
    fn zero_port_is_rejected() {
        let error = validate_web_mcp_port(0).expect_err("port 0 is not usable");
        assert_eq!(error.code, WEB_MCP_CODE_INVALID_PORT);
        assert!(validate_web_mcp_port(1).is_ok());
    }

    #[test]
    fn status_view_never_serializes_a_secret() {
        let status = WebMcpProviderStatus {
            server_name: WEB_MCP_SERVER_NAME.to_string(),
            generation: 3,
            instance_id: "instance-a".to_string(),
            client_id: "tauri-main".to_string(),
            lease_id: "lease-1".to_string(),
            port: 41234,
            expires_at: "unix-100".to_string(),
        };
        let value = serde_json::to_value(&status).expect("serialize status");
        assert_eq!(
            value,
            json!({
                "server_name": "chatspeed_web",
                "generation": 3,
                "instance_id": "instance-a",
                "client_id": "tauri-main",
                "lease_id": "lease-1",
                "port": 41234,
                "expires_at": "unix-100"
            })
        );
        assert!(!value.to_string().contains("token"));
        assert!(!value.to_string().contains("proof"));
    }

    #[test]
    fn reserved_aliases_are_exactly_the_two_web_tools() {
        assert_eq!(WEB_MCP_ALIASES, ["web_fetch", "web_search"]);
        assert_eq!(WEB_MCP_SERVER_NAME, "chatspeed_web");
    }
}