//! Runtime-only control-plane surface for the desktop loopback Web MCP provider
//! (AC-8).
//!
//! The desktop hosts the two fixed web tools (`web_fetch`, `web_search`) as a
//! dedicated MCP server on an ephemeral `127.0.0.1` port. It registers that
//! provider here; the runtime then reaches it as an ordinary streamable-HTTP MCP
//! server. This module owns only the control-plane half:
//!
//! - the registration body carries **only** the port, so the runtime derives the
//!   endpoint itself and a client can never name an arbitrary authority;
//! - the provider proof token, the client id, the lease id and the desktop
//!   instance id travel in dedicated headers, never in the URL, query or body;
//! - the caller must prove a live `tauri` lease, so a registry-less process can
//!   never install a provider;
//! - a single slot is enforced by the injected [`RuntimeWebMcpPlane`], and a
//!   second live desktop is rejected with a structured `conflict` instead of an
//!   implicit reroute.

use async_trait::async_trait;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chatspeed_contracts::{
    validate_web_mcp_port, ClientLease, WebMcpProviderError, WebMcpProviderRegistration,
    WebMcpProviderRegistrationResponse, WebMcpProviderStatus, WEB_MCP_CLIENT_HEADER,
    WEB_MCP_CODE_CONFLICT, WEB_MCP_CODE_FORBIDDEN, WEB_MCP_CODE_LEASE_INVALID,
    WEB_MCP_CODE_UNAVAILABLE, WEB_MCP_INSTANCE_HEADER, WEB_MCP_LEASE_HEADER, WEB_MCP_PROOF_HEADER,
    WEB_MCP_REGISTER_PATH, WEB_MCP_STATUS_PATH, WEB_MCP_UNREGISTER_PATH,
};

use super::dto;
use super::server::{ControlPlaneState, RuntimeLeaseError};

/// Fixed client kind the provider may be registered from. The runtime resolves
/// the lease itself, so a client cannot claim the kind in a body.
const WEB_MCP_CLIENT_KIND: &str = "tauri";

/// A narrow lease-validity oracle the provider slot is swept against.
///
/// A plain closure cannot be passed into the async trait method without a
/// self-referential lifetime, so the sweeper hands over one of these instead.
pub trait WebProviderLeaseCheck: Send + Sync {
    /// Whether `(client_id, lease_id)` still resolves to a live lease.
    fn is_valid(&self, client_id: &str, lease_id: &str) -> bool;
}

/// The runtime-owned single-slot provider lifecycle.
///
/// Implemented by the standalone runtime owner; the desktop in-process control
/// plane never mounts these routes because it owns no lease lifecycle.
#[async_trait]
pub trait RuntimeWebMcpPlane: Send + Sync + 'static {
    /// Installs (or idempotently re-confirms) the desktop provider slot.
    ///
    /// `lease` is the live lease the caller has already proven. A different live
    /// lease must be rejected with [`WEB_MCP_CODE_CONFLICT`] rather than
    /// silently replacing the incumbent.
    async fn register_provider(
        &self,
        registration: &WebMcpProviderRegistration,
        lease: &ClientLease,
        proof: &str,
        instance_id: &str,
    ) -> Result<WebMcpProviderRegistrationResponse, WebMcpProviderError>;

    /// Removes the provider slot when `proof` and `lease` match the incumbent.
    ///
    /// Removing an already-absent slot for the same caller is a no-op.
    async fn unregister_provider(
        &self,
        lease: &ClientLease,
        proof: &str,
    ) -> Result<(), WebMcpProviderError>;

    /// Redacted status of the current slot, if any.
    fn provider_status(&self) -> Option<WebMcpProviderStatus>;

    /// Drops the slot when its lease is no longer valid.
    async fn sweep_invalid_leases(&self, is_valid: &dyn WebProviderLeaseCheck);
}

/// Mounts the provider control routes.
///
/// Added under the shared bearer middleware and body limit, and only when the
/// process owns a runtime lease lifecycle.
pub fn web_mcp_router() -> Router<ControlPlaneState> {
    Router::new()
        .route(WEB_MCP_REGISTER_PATH, post(register_web_provider))
        .route(WEB_MCP_UNREGISTER_PATH, post(unregister_web_provider))
        .route(WEB_MCP_STATUS_PATH, get(web_provider_status))
}

/// The four header-only proof values a provider control call carries.
struct ProviderProofHeaders {
    proof: String,
    client_id: String,
    lease_id: String,
    instance_id: String,
}

fn header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn proof_headers(headers: &HeaderMap) -> Result<ProviderProofHeaders, Response> {
    let field = |name: &str| {
        header_value(headers, name).ok_or_else(|| {
            dto::error_response(
                StatusCode::FORBIDDEN,
                WEB_MCP_CODE_FORBIDDEN,
                format!("missing required provider proof header `{name}`"),
            )
        })
    };
    Ok(ProviderProofHeaders {
        proof: field(WEB_MCP_PROOF_HEADER)?,
        client_id: field(WEB_MCP_CLIENT_HEADER)?,
        lease_id: field(WEB_MCP_LEASE_HEADER)?,
        instance_id: field(WEB_MCP_INSTANCE_HEADER)?,
    })
}

fn validate_lease(
    state: &ControlPlaneState,
    client_id: &str,
    lease_id: &str,
) -> Result<ClientLease, Response> {
    let Some(runtime) = state.runtime.as_ref() else {
        return Err(runtime_absent_response());
    };
    match runtime.validate_lease(client_id, lease_id) {
        Ok(lease) => Ok(lease),
        Err(RuntimeLeaseError::NotFound(client_id)) => Err(dto::error_response(
            StatusCode::NOT_FOUND,
            WEB_MCP_CODE_LEASE_INVALID,
            format!("No active lease for client `{client_id}`"),
        )),
        Err(RuntimeLeaseError::InvalidInput(message)) => Err(dto::error_response(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            message,
        )),
    }
}

/// Maps a structured provider error onto a stable control-plane envelope.
fn provider_error_response(error: WebMcpProviderError) -> Response {
    let (status, code): (StatusCode, &'static str) = match error.code.as_str() {
        WEB_MCP_CODE_CONFLICT => (StatusCode::CONFLICT, WEB_MCP_CODE_CONFLICT),
        WEB_MCP_CODE_FORBIDDEN => (StatusCode::FORBIDDEN, WEB_MCP_CODE_FORBIDDEN),
        WEB_MCP_CODE_LEASE_INVALID => (StatusCode::FORBIDDEN, WEB_MCP_CODE_LEASE_INVALID),
        WEB_MCP_CODE_UNAVAILABLE => (StatusCode::NOT_FOUND, WEB_MCP_CODE_UNAVAILABLE),
        _ => (StatusCode::BAD_REQUEST, "invalid_input"),
    };
    dto::error_response(status, code, error.message)
}

fn runtime_absent_response() -> Response {
    dto::error_response(
        StatusCode::NOT_FOUND,
        WEB_MCP_CODE_UNAVAILABLE,
        "This control plane does not own a Web MCP provider lifecycle".to_string(),
    )
}

/// `POST /control/v1/web-mcp/register` — installs the provider slot.
async fn register_web_provider(
    State(state): State<ControlPlaneState>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let registration: WebMcpProviderRegistration = match serde_json::from_str(&body) {
        Ok(registration) => registration,
        Err(error) => {
            return dto::error_response(
                StatusCode::BAD_REQUEST,
                "invalid_input",
                format!("Invalid Web MCP provider registration: {error}"),
            )
        }
    };
    if let Err(error) = validate_web_mcp_port(registration.port) {
        return provider_error_response(error);
    }
    let proof = match proof_headers(&headers) {
        Ok(proof) => proof,
        Err(response) => return response,
    };
    let lease = match validate_lease(&state, &proof.client_id, &proof.lease_id) {
        Ok(lease) => lease,
        Err(response) => return response,
    };
    if lease.client_kind != WEB_MCP_CLIENT_KIND {
        return dto::error_response(
            StatusCode::FORBIDDEN,
            WEB_MCP_CODE_FORBIDDEN,
            format!(
                "client kind `{}` may not register a Web MCP provider",
                lease.client_kind
            ),
        );
    }
    let Some(plane) = state.web_provider.as_ref() else {
        return runtime_absent_response();
    };
    match plane
        .register_provider(&registration, &lease, &proof.proof, &proof.instance_id)
        .await
    {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => provider_error_response(error),
    }
}

/// `POST /control/v1/web-mcp/unregister` — removes the provider slot.
async fn unregister_web_provider(
    State(state): State<ControlPlaneState>,
    headers: HeaderMap,
) -> Response {
    let proof = match proof_headers(&headers) {
        Ok(proof) => proof,
        Err(response) => return response,
    };
    let lease = match validate_lease(&state, &proof.client_id, &proof.lease_id) {
        Ok(lease) => lease,
        Err(response) => return response,
    };
    let Some(plane) = state.web_provider.as_ref() else {
        return runtime_absent_response();
    };
    match plane.unregister_provider(&lease, &proof.proof).await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response(),
        Err(error) => provider_error_response(error),
    }
}

/// `GET /control/v1/web-mcp/status` — redacted provider slot view.
async fn web_provider_status(State(state): State<ControlPlaneState>) -> Response {
    let Some(plane) = state.web_provider.as_ref() else {
        return runtime_absent_response();
    };
    match plane.provider_status() {
        Some(status) => (StatusCode::OK, Json(status)).into_response(),
        None => dto::error_response(
            StatusCode::NOT_FOUND,
            WEB_MCP_CODE_UNAVAILABLE,
            "no Web MCP provider is registered".to_string(),
        ),
    }
}
