//! Runtime-only client WebView capability bridge (U-7).
//!
//! The runtime never links a Tauri WebView, so a capability the desktop owns is
//! reachable only through a bridge the desktop opens into the runtime. This
//! module is the runtime half of that bridge and is compiled only in the
//! desktop-free build: it references no Tauri, Wry or GTK type.
//!
//! The mechanism is client-pull. The runtime opens no connection to any client
//! address. Instead the desktop:
//!
//! 1. registers a short-lived session bound to its live client lease
//!    ([`ClientBridgeRegistry::register`]);
//! 2. pulls typed work envelopes from a per-session bounded SSE stream;
//! 3. posts one typed result (or cancel) per request id, proving ownership.
//!
//! The bridge is deliberately not a generic RPC: the capability set is the
//! fixed [`BRIDGE_CAPABILITY_ALLOWLIST`], the schema of each capability is
//! closed, and every route proves the bearer token plus the opaque session
//! secret plus the live lease on each call. Delivery is at-most-once: a
//! disconnect, cancel or lease expiry fails the pending invocation instead of
//! re-dispatching an already delivered request.

use std::collections::HashMap;
use std::convert::Infallible;
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chatspeed_contracts::{
    ClientBridgeCancelRequest, ClientBridgeCapability, ClientBridgeDeclaration,
    ClientBridgeRegistration, ClientBridgeRegistrationResponse, ClientBridgeUnregisterRequest,
    ClientBridgeWorkEnvelope, ClientCapabilityError, ClientCapabilityInvocation,
    ClientCapabilityResult, ClientLease, BRIDGE_CAPABILITY_ALLOWLIST, BRIDGE_PROTOCOL_VERSION,
    BRIDGE_SCHEMA_VERSION,
};
use rand::Rng;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, oneshot};
use tokio_stream::wrappers::ReceiverStream;

use super::dto;
use super::server::{
    ClientCapabilityView, ControlPlaneState, RuntimeControlPlane, RuntimeLeaseError,
};

/// Header the desktop presents its opaque bridge session secret in.
///
/// The secret travels only here: never in a URL path, query string or body.
pub const BRIDGE_SESSION_HEADER: &str = "x-bridge-session";

/// Fixed client kind a bridge may be opened for. The runtime resolves the
/// lease itself; a body-supplied kind is never trusted.
pub const BRIDGE_CLIENT_KIND: &str = "tauri";

/// Default per-session work-queue capacity (bounded).
const DEFAULT_QUEUE_CAPACITY: usize = 32;

/// Default per-session concurrent in-flight limit (bounded).
const DEFAULT_MAX_IN_FLIGHT: usize = 4;

/// Default session lifetime. A session is also invalidated as soon as its
/// underlying lease is no longer valid.
const DEFAULT_SESSION_TTL: Duration = Duration::from_secs(300);

/// Interval between SSE keepalive comments.
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);

/// `POST` route that opens a bridge session.
pub const CLIENT_BRIDGE_REGISTER_PATH: &str = "/control/v1/client-bridge/register";

/// Mounts the bridge routes.
///
/// These routes are added under the same bearer middleware and body limit as
/// every other control-plane route, and only when the process owns a runtime
/// extension (a lease lifecycle). A capability bridge with no runtime owner
/// could not prove a lease, so it is never mounted.
pub fn client_bridge_router() -> Router<ControlPlaneState> {
    Router::new()
        .route(CLIENT_BRIDGE_REGISTER_PATH, post(register_bridge))
        .route(
            "/control/v1/client-bridge/{session_id}/events",
            get(stream_bridge_events),
        )
        .route(
            "/control/v1/client-bridge/{session_id}/result",
            post(submit_bridge_result),
        )
        .route(
            "/control/v1/client-bridge/{session_id}/cancel",
            post(cancel_bridge_request),
        )
        .route(
            "/control/v1/client-bridge/{session_id}/unregister",
            post(unregister_bridge),
        )
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// A bridge operation failure, before it is mapped onto the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeError {
    /// The caller is not allowed to open or use the bridge.
    Forbidden(String),
    /// The request body or declaration is malformed.
    InvalidInput(String),
    /// The session or request id is unknown.
    NotFound(String),
    /// The session is already attached to a reader.
    Conflict(String),
    /// The bounded queue or in-flight limit is full.
    Busy(String),
    /// No live bridge declares the capability.
    Unavailable(String),
}

impl BridgeError {
    /// Maps the failure onto a stable control-plane error envelope.
    pub(crate) fn into_http_response(self) -> Response {
        let (status, code) = match &self {
            BridgeError::Forbidden(_) => (StatusCode::FORBIDDEN, "forbidden"),
            BridgeError::InvalidInput(_) => (StatusCode::BAD_REQUEST, "invalid_input"),
            BridgeError::NotFound(_) => (StatusCode::NOT_FOUND, "not_found"),
            BridgeError::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
            BridgeError::Busy(_) => (StatusCode::SERVICE_UNAVAILABLE, "bridge_busy"),
            BridgeError::Unavailable(_) => (StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
        };
        let message = match self {
            BridgeError::Forbidden(message)
            | BridgeError::InvalidInput(message)
            | BridgeError::NotFound(message)
            | BridgeError::Conflict(message)
            | BridgeError::Busy(message)
            | BridgeError::Unavailable(message) => message,
        };
        dto::error_response(status, code, message)
    }
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Bounded limits and lifetimes for a bridge registry.
#[derive(Debug, Clone, Copy)]
pub struct BridgeLimits {
    /// Maximum queued-but-unpulled work envelopes per session.
    pub queue_capacity: usize,
    /// Maximum concurrent in-flight invocations per session.
    pub max_in_flight: usize,
    /// Session lifetime; combined with the live-lease check on every call.
    pub session_ttl: Duration,
}

impl Default for BridgeLimits {
    fn default() -> Self {
        Self {
            queue_capacity: DEFAULT_QUEUE_CAPACITY,
            max_in_flight: DEFAULT_MAX_IN_FLIGHT,
            session_ttl: DEFAULT_SESSION_TTL,
        }
    }
}

/// Terminal outcome of one dispatched invocation, delivered to the waiter.
#[derive(Debug)]
pub enum BridgeOutcome {
    /// The client returned a typed result.
    Completed(ClientCapabilityResult),
    /// The invocation failed before a result existed (cancel, disconnect).
    Failed(ClientCapabilityError),
}

/// Opaque, non-secret session identity returned to a registered client.
#[derive(Debug, Clone)]
pub struct RegisteredSession {
    /// Opaque session id used in the bridge route path.
    pub session_id: String,
    /// Opaque session secret; only the registry's hash is retained.
    pub session_token: String,
    /// RFC 3339 expiry of the session.
    pub expires_at: String,
    /// Stable fingerprint of the accepted declaration.
    pub declaration_fingerprint: String,
}

struct BridgeSession {
    token_hash: [u8; 32],
    client_id: String,
    lease_id: String,
    capabilities: Vec<ClientBridgeCapability>,
    expires_at: Instant,
    work_tx: Option<mpsc::Sender<ClientBridgeWorkEnvelope>>,
    work_rx: Option<mpsc::Receiver<ClientBridgeWorkEnvelope>>,
    in_flight: HashMap<String, oneshot::Sender<BridgeOutcome>>,
}

impl BridgeSession {
    fn accepts(&self, capability: &str, schema_version: &str) -> bool {
        self.capabilities
            .iter()
            .any(|entry| entry.name == capability && entry.schema_version == schema_version)
    }

    /// Removes and returns every in-flight waiter so they fail closed.
    fn drain_waiters(&mut self) -> Vec<oneshot::Sender<BridgeOutcome>> {
        self.in_flight.drain().map(|(_, sender)| sender).collect()
    }
}

#[derive(Default)]
struct BridgeState {
    sessions: HashMap<String, BridgeSession>,
    by_client: HashMap<String, String>,
}

/// Instance-local registry of live client bridges.
///
/// The registry is the single authority for which capabilities are currently
/// bridgeable. It is intentionally synchronous under a short-lived mutex; the
/// only operation that awaits is the caller waiting on the returned receiver.
pub struct ClientBridgeRegistry {
    limits: BridgeLimits,
    inner: std::sync::Mutex<BridgeState>,
}

impl ClientBridgeRegistry {
    /// Creates an empty registry with explicit limits.
    pub fn new(limits: BridgeLimits) -> Self {
        Self {
            limits,
            inner: std::sync::Mutex::new(BridgeState::default()),
        }
    }

    /// Creates an empty registry with the default limits.
    pub fn with_defaults() -> Self {
        Self::new(BridgeLimits::default())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BridgeState> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Validates a declaration against the closed allowlist and schema version.
    fn validate_declaration(declaration: &ClientBridgeDeclaration) -> Result<(), BridgeError> {
        if declaration.protocol_version != BRIDGE_PROTOCOL_VERSION {
            return Err(BridgeError::InvalidInput(format!(
                "unsupported bridge protocol version `{}`",
                declaration.protocol_version
            )));
        }
        if declaration.schema_version != BRIDGE_SCHEMA_VERSION {
            return Err(BridgeError::InvalidInput(format!(
                "unsupported bridge schema version `{}`",
                declaration.schema_version
            )));
        }
        let mut seen: Vec<&str> = Vec::new();
        for capability in &declaration.capabilities {
            if !BRIDGE_CAPABILITY_ALLOWLIST.contains(&capability.name.as_str()) {
                return Err(BridgeError::Forbidden(format!(
                    "`{}` is not an allowlisted client capability",
                    capability.name
                )));
            }
            if capability.schema_version != BRIDGE_SCHEMA_VERSION {
                return Err(BridgeError::InvalidInput(format!(
                    "unsupported schema version `{}` for `{}`",
                    capability.schema_version, capability.name
                )));
            }
            if seen.contains(&capability.name.as_str()) {
                return Err(BridgeError::InvalidInput(format!(
                    "capability `{}` is declared twice",
                    capability.name
                )));
            }
            seen.push(capability.name.as_str());
        }
        Ok(())
    }

    /// Stable fingerprint of a declaration, derived from its canonical JSON.
    fn declaration_fingerprint(declaration: &ClientBridgeDeclaration) -> String {
        let canonical = serde_json::to_string(declaration).unwrap_or_default();
        hex::encode(Sha256::digest(canonical.as_bytes()))
    }

    /// Registers a session for a validated lease and declaration.
    ///
    /// Any previous session for the same client is replaced (its in-flight
    /// waiters fail closed). The declaration must be a subset of the allowlist.
    pub fn register(
        &self,
        lease: &ClientLease,
        declaration: &ClientBridgeDeclaration,
    ) -> Result<RegisteredSession, BridgeError> {
        Self::validate_declaration(declaration)?;

        let session_id = generate_token();
        let session_token = generate_token();
        let token_hash: [u8; 32] = Sha256::digest(session_token.as_bytes()).into();
        let fingerprint = Self::declaration_fingerprint(declaration);
        let expires_at = Instant::now() + self.limits.session_ttl;
        let expires_at_text = rfc3339_after(self.limits.session_ttl);
        let (work_tx, work_rx) = mpsc::channel(self.limits.queue_capacity.max(1));

        let mut state = self.lock();
        if let Some(previous) = state.by_client.remove(&lease.client_id) {
            if let Some(mut old) = state.sessions.remove(&previous) {
                for waiter in old.drain_waiters() {
                    let _ = waiter.send(BridgeOutcome::Failed(cancelled_error(
                        "replaced_by_new_registration",
                    )));
                }
            }
        }
        state
            .by_client
            .insert(lease.client_id.clone(), session_id.clone());
        state.sessions.insert(
            session_id.clone(),
            BridgeSession {
                token_hash,
                client_id: lease.client_id.clone(),
                lease_id: lease.lease_id.clone(),
                capabilities: declaration.capabilities.clone(),
                expires_at,
                work_tx: Some(work_tx),
                work_rx: Some(work_rx),
                in_flight: HashMap::new(),
            },
        );
        log::debug!(
            "[ClientBridge] registered bridge session for client of kind {}",
            lease.client_kind
        );
        Ok(RegisteredSession {
            session_id,
            session_token,
            expires_at: expires_at_text,
            declaration_fingerprint: fingerprint,
        })
    }

    /// Resolves the client id of a session whose token matches.
    pub fn client_id_for_session(&self, session_id: &str, token: &str) -> Option<String> {
        let state = self.lock();
        let session = state.sessions.get(session_id)?;
        constant_time_eq(&session.token_hash, token).then(|| session.client_id.clone())
    }

    /// The lease id a session is bound to, if the session still exists.
    pub fn lease_id_for_session(&self, session_id: &str) -> Option<String> {
        self.lock()
            .sessions
            .get(session_id)
            .map(|session| session.lease_id.clone())
    }

    /// Rejects a session whose token or client id does not match.
    fn authorize(
        state: &BridgeState,
        session_id: &str,
        token: &str,
        client_id: &str,
    ) -> Result<(), BridgeError> {
        let session = state
            .sessions
            .get(session_id)
            .ok_or_else(|| BridgeError::NotFound("no live bridge session".to_string()))?;
        if !constant_time_eq(&session.token_hash, token) {
            return Err(BridgeError::Forbidden(
                "invalid bridge session credential".to_string(),
            ));
        }
        if session.client_id != client_id {
            return Err(BridgeError::Forbidden(
                "bridge session does not belong to this client".to_string(),
            ));
        }
        Ok(())
    }

    /// Takes the session's work-pull receiver; it can be taken only once.
    pub fn attach_reader(
        &self,
        session_id: &str,
        token: &str,
        client_id: &str,
    ) -> Result<mpsc::Receiver<ClientBridgeWorkEnvelope>, BridgeError> {
        let mut state = self.lock();
        Self::authorize(&state, session_id, token, client_id)?;
        let session = state
            .sessions
            .get_mut(session_id)
            .expect("authorized session exists");
        session
            .work_rx
            .take()
            .ok_or_else(|| BridgeError::Conflict("a reader is already attached".to_string()))
    }

    /// Queues one typed invocation and returns the receiver its result arrives on.
    ///
    /// The request id is generated here; the caller must not choose it.
    pub fn enqueue(
        &self,
        session_id: &str,
        capability: &str,
        schema_version: &str,
        arguments: Value,
        deadline: Duration,
    ) -> Result<oneshot::Receiver<BridgeOutcome>, BridgeError> {
        let request_id = generate_token();
        let invocation = ClientCapabilityInvocation {
            request_id: request_id.clone(),
            capability: capability.to_string(),
            schema_version: schema_version.to_string(),
            arguments,
            deadline: rfc3339_after(deadline),
        };

        let mut state = self.lock();
        let session = state
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| BridgeError::Unavailable("no live bridge session".to_string()))?;
        if !session.accepts(capability, schema_version) {
            return Err(BridgeError::Unavailable(format!(
                "the live bridge does not declare `{capability}`"
            )));
        }
        if session.in_flight.len() >= self.limits.max_in_flight {
            return Err(BridgeError::Busy(
                "the bridge has too many in-flight invocations".to_string(),
            ));
        }
        let Some(work_tx) = session.work_tx.as_ref() else {
            return Err(BridgeError::Unavailable(
                "the bridge is not pulling work".to_string(),
            ));
        };
        let (result_tx, result_rx) = oneshot::channel();
        session.in_flight.insert(request_id.clone(), result_tx);
        let envelope = ClientBridgeWorkEnvelope {
            schema_version: BRIDGE_SCHEMA_VERSION.to_string(),
            invocation,
        };
        if let Err(error) = work_tx.try_send(envelope) {
            if let Some(session) = state.sessions.get_mut(session_id) {
                session.in_flight.remove(&request_id);
            }
            return Err(match error {
                mpsc::error::TrySendError::Full(_) => {
                    BridgeError::Busy("the bridge work queue is full".to_string())
                }
                mpsc::error::TrySendError::Closed(_) => {
                    BridgeError::Unavailable("the bridge is not pulling work".to_string())
                }
            });
        }
        Ok(result_rx)
    }

    /// Completes one in-flight request, proving session and request ownership.
    pub fn complete(
        &self,
        session_id: &str,
        token: &str,
        client_id: &str,
        result: ClientCapabilityResult,
    ) -> Result<(), BridgeError> {
        let mut state = self.lock();
        Self::authorize(&state, session_id, token, client_id)?;
        let session = state
            .sessions
            .get_mut(session_id)
            .expect("authorized session exists");
        let waiter = session
            .in_flight
            .remove(&result.request_id)
            .ok_or_else(|| {
                BridgeError::NotFound(format!(
                    "request `{}` is not in flight for this session",
                    result.request_id
                ))
            })?;
        let _ = waiter.send(BridgeOutcome::Completed(result));
        Ok(())
    }

    /// Cancels one in-flight request, failing its waiter closed.
    ///
    /// Cancelling an unknown or already-completed request is a no-op so a
    /// cancel/result race never turns into an error for the client.
    pub fn cancel(
        &self,
        session_id: &str,
        token: &str,
        client_id: &str,
        request_id: &str,
    ) -> Result<(), BridgeError> {
        let mut state = self.lock();
        Self::authorize(&state, session_id, token, client_id)?;
        if let Some(session) = state.sessions.get_mut(session_id) {
            if let Some(waiter) = session.in_flight.remove(request_id) {
                let _ = waiter.send(BridgeOutcome::Failed(cancelled_error(
                    "cancelled_by_client",
                )));
            }
        }
        Ok(())
    }

    /// Removes a session and fails every in-flight waiter closed.
    pub fn unregister(
        &self,
        session_id: &str,
        token: &str,
        client_id: &str,
    ) -> Result<(), BridgeError> {
        let mut state = self.lock();
        Self::authorize(&state, session_id, token, client_id)?;
        Self::remove_session(&mut state, session_id);
        Ok(())
    }

    /// Removes a session after its reader disconnected.
    ///
    /// At-most-once: an unpulled invocation is never re-dispatched, so it fails
    /// closed here instead of waiting for its deadline.
    pub fn disconnect_reader(&self, session_id: &str) {
        let mut state = self.lock();
        Self::remove_session(&mut state, session_id);
    }

    fn remove_session(state: &mut BridgeState, session_id: &str) {
        if let Some(mut session) = state.sessions.remove(session_id) {
            state.by_client.remove(&session.client_id);
            for waiter in session.drain_waiters() {
                let _ = waiter.send(BridgeOutcome::Failed(cancelled_error(
                    "bridge_disconnected",
                )));
            }
        }
    }

    /// Drops expired sessions; used by the lease sweeper.
    pub fn sweep_expired(&self, now: Instant) {
        let mut state = self.lock();
        let expired: Vec<String> = state
            .sessions
            .iter()
            .filter(|(_, session)| session.expires_at <= now)
            .map(|(id, _)| id.clone())
            .collect();
        for session_id in expired {
            Self::remove_session(&mut state, &session_id);
        }
    }

    /// Drops sessions whose lease is no longer valid.
    ///
    /// `is_valid` resolves the lease for a `(client_id, lease_id)` pair; a
    /// session is removed as soon as its lease is gone or expired.
    pub fn sweep_invalid_leases(&self, is_valid: impl Fn(&str, &str) -> bool) {
        let mut state = self.lock();
        let stale: Vec<String> = state
            .sessions
            .iter()
            .filter(|(_, session)| !is_valid(&session.client_id, &session.lease_id))
            .map(|(id, _)| id.clone())
            .collect();
        for session_id in stale {
            Self::remove_session(&mut state, &session_id);
        }
    }

    /// Number of live sessions (test/lifecycle introspection).
    pub fn session_count(&self) -> usize {
        self.lock().sessions.len()
    }

    /// Resolves a live session that both declares `capability` and proves
    /// possession of that session's opaque credential.
    ///
    /// The ordinary runtime bearer authenticates a control-plane client, but it
    /// must not be sufficient to trigger a Tauri WebView. This second proof is
    /// therefore required by the public capability invocation route.
    pub fn session_for_capability_with_token(
        &self,
        capability: &str,
        token: &str,
    ) -> Result<String, BridgeError> {
        let state = self.lock();
        let Some((session_id, session)) = state
            .sessions
            .iter()
            .find(|(_, session)| session.accepts(capability, BRIDGE_SCHEMA_VERSION))
        else {
            return Err(BridgeError::Unavailable(format!(
                "no live bridge declares `{capability}`"
            )));
        };
        if !constant_time_eq(&session.token_hash, token) {
            return Err(BridgeError::Forbidden(
                "a bridge session credential is required to invoke client capabilities".to_string(),
            ));
        }
        Ok(session_id.clone())
    }

    /// Returns whether a live session declares `capability`.
    pub fn has_capability(&self, capability: &str) -> bool {
        self.lock()
            .sessions
            .values()
            .any(|session| session.accepts(capability, BRIDGE_SCHEMA_VERSION))
    }

    /// The first live session that declares `capability` at the current schema.
    pub fn session_for_capability(&self, capability: &str) -> Option<String> {
        let state = self.lock();
        state
            .sessions
            .iter()
            .find(|(_, session)| session.accepts(capability, BRIDGE_SCHEMA_VERSION))
            .map(|(session_id, _)| session_id.clone())
    }

    /// The request ids currently in flight for a session (introspection).
    pub fn in_flight_request_ids(&self, session_id: &str) -> Vec<String> {
        self.lock()
            .sessions
            .get(session_id)
            .map(|session| session.in_flight.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// The dynamic capability registry: what the live bridge actually declares.
    ///
    /// With no live bridge every capability reports `unavailable`, which keeps
    /// the discovery contract honest instead of advertising a capability no
    /// client can execute.
    pub fn capability_registry(&self) -> Vec<ClientCapabilityView> {
        let state = self.lock();
        BRIDGE_CAPABILITY_ALLOWLIST
            .iter()
            .map(|name| {
                let declared = state
                    .sessions
                    .values()
                    .any(|session| session.accepts(name, BRIDGE_SCHEMA_VERSION));
                ClientCapabilityView {
                    name: (*name).to_string(),
                    kind: "web".to_string(),
                    status: if declared {
                        "available".to_string()
                    } else {
                        "unavailable".to_string()
                    },
                    requires_client_bridge: true,
                    bridge_declared: declared,
                    detail: if declared {
                        format!("a live client bridge declares `{name}`")
                    } else {
                        format!(
                            "`{name}` executes only through a client WebView bridge, and this runtime has no live bridge declaring it"
                        )
                    },
                }
            })
            .collect()
    }
}

impl Default for ClientBridgeRegistry {
    fn default() -> Self {
        Self::with_defaults()
    }
}

fn cancelled_error(reason: &str) -> ClientCapabilityError {
    ClientCapabilityError {
        code: "cancelled".to_string(),
        message: format!("the capability invocation was cancelled: {reason}"),
    }
}

fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// RFC 3339 deadline string for `now + offset`.
fn rfc3339_after(offset: Duration) -> String {
    let deadline = chrono::Utc::now() + chrono::TimeDelta::seconds(offset.as_secs() as i64);
    deadline.to_rfc3339()
}

/// Constant-time comparison for a stored token hash and a presented token.
fn constant_time_eq(expected_hash: &[u8; 32], presented: &str) -> bool {
    let digest: [u8; 32] = Sha256::digest(presented.as_bytes()).into();
    let mut diff = 0u8;
    for (x, y) in expected_hash.iter().zip(digest.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Reads the opaque session secret from the dedicated header only.
pub(crate) fn bridge_session_header(headers: &HeaderMap) -> Result<&str, BridgeError> {
    headers
        .get(BRIDGE_SESSION_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            BridgeError::Forbidden("missing bridge session credential header".to_string())
        })
}

fn runtime_absent_response() -> Response {
    dto::error_response(
        StatusCode::NOT_FOUND,
        "not_found",
        "This control plane does not own a client lease lifecycle".to_string(),
    )
}

/// Re-validates the session's underlying lease against the runtime.
fn lease_still_valid(
    runtime: &dyn RuntimeControlPlane,
    registry: &ClientBridgeRegistry,
    session_id: &str,
    client_id: &str,
) -> bool {
    let Some(lease_id) = registry.lease_id_for_session(session_id) else {
        return false;
    };
    match runtime.validate_lease(client_id, &lease_id) {
        Ok(lease) => lease.client_kind == BRIDGE_CLIENT_KIND,
        Err(_) => false,
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `POST /control/v1/client-bridge/register` — opens a bridge session.
///
/// The runtime resolves the client's lease itself and requires the fixed
/// `tauri` kind, so a client that merely claims the kind in its body cannot
/// open a bridge.
async fn register_bridge(State(state): State<ControlPlaneState>, body: String) -> Response {
    let registration: ClientBridgeRegistration = match serde_json::from_str(&body) {
        Ok(registration) => registration,
        Err(error) => {
            return dto::error_response(
                StatusCode::BAD_REQUEST,
                "invalid_input",
                format!("Invalid bridge registration: {error}"),
            )
        }
    };
    let Some(runtime) = state.runtime.as_ref() else {
        return runtime_absent_response();
    };
    let lease = match runtime.validate_lease(&registration.client_id, &registration.lease_id) {
        Ok(lease) => lease,
        Err(RuntimeLeaseError::NotFound(client_id)) => {
            return dto::error_response(
                StatusCode::NOT_FOUND,
                "not_found",
                format!("No active lease for client `{client_id}`"),
            )
        }
        Err(RuntimeLeaseError::InvalidInput(message)) => {
            return dto::error_response(StatusCode::BAD_REQUEST, "invalid_input", message)
        }
    };
    if lease.client_kind != BRIDGE_CLIENT_KIND {
        return dto::error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            format!(
                "client kind `{}` may not open a WebView bridge",
                lease.client_kind
            ),
        );
    }
    match state.bridge.register(&lease, &registration.declaration) {
        Ok(session) => (
            StatusCode::OK,
            Json(ClientBridgeRegistrationResponse {
                session_id: session.session_id,
                session_token: session.session_token,
                expires_at: session.expires_at,
                protocol_version: BRIDGE_PROTOCOL_VERSION.to_string(),
                declaration_fingerprint: session.declaration_fingerprint,
            }),
        )
            .into_response(),
        Err(error) => error.into_http_response(),
    }
}

/// `GET /control/v1/client-bridge/{session_id}/events` — the typed work stream.
///
/// The secret is read from the header only. When the response is dropped the
/// session is torn down and every pending invocation fails closed, so work is
/// never re-dispatched to a client that stopped reading.
async fn stream_bridge_events(
    State(state): State<ControlPlaneState>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let Some(runtime) = state.runtime.as_ref() else {
        return runtime_absent_response();
    };
    let token = match bridge_session_header(&headers) {
        Ok(token) => token.to_string(),
        Err(error) => return error.into_http_response(),
    };
    let Some(client_id) = state.bridge.client_id_for_session(&session_id, &token) else {
        return bridge_credential_rejected();
    };
    if !lease_still_valid(runtime.as_ref(), &state.bridge, &session_id, &client_id) {
        state.bridge.disconnect_reader(&session_id);
        return lease_rejected();
    }
    let work_rx = match state.bridge.attach_reader(&session_id, &token, &client_id) {
        Ok(receiver) => receiver,
        Err(error) => return error.into_http_response(),
    };

    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(64);
    let registry = state.bridge.clone();
    let guard_id = session_id.clone();
    tokio::spawn(async move {
        pump_bridge_events(work_rx, tx).await;
        registry.disconnect_reader(&guard_id);
    });

    Sse::new(ReceiverStream::new(rx))
        .keep_alive(KeepAlive::default())
        .into_response()
}

async fn pump_bridge_events(
    mut work_rx: mpsc::Receiver<ClientBridgeWorkEnvelope>,
    tx: mpsc::Sender<Result<Event, Infallible>>,
) {
    let mut keepalive = tokio::time::interval(KEEPALIVE_INTERVAL);
    keepalive.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The first tick fires immediately; skip it so no keepalive precedes work.
    keepalive.tick().await;
    loop {
        tokio::select! {
            received = work_rx.recv() => {
                let Some(envelope) = received else {
                    return;
                };
                let Ok(data) = serde_json::to_string(&envelope) else {
                    continue;
                };
                if tx.send(Ok(Event::default().data(data))).await.is_err() {
                    return;
                }
            }
            _ = keepalive.tick() => {
                if tx
                    .send(Ok(Event::default().comment("keepalive")))
                    .await
                    .is_err()
                {
                    return;
                }
            }
        }
    }
}

/// `POST /control/v1/client-bridge/{session_id}/result` — one typed result.
async fn submit_bridge_result(
    State(state): State<ControlPlaneState>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let Some(runtime) = state.runtime.as_ref() else {
        return runtime_absent_response();
    };
    let token = match bridge_session_header(&headers) {
        Ok(token) => token.to_string(),
        Err(error) => return error.into_http_response(),
    };
    let result: ClientCapabilityResult = match serde_json::from_str(&body) {
        Ok(result) => result,
        Err(error) => {
            return dto::error_response(
                StatusCode::BAD_REQUEST,
                "invalid_input",
                format!("Invalid bridge result: {error}"),
            )
        }
    };
    let Some(client_id) = state.bridge.client_id_for_session(&session_id, &token) else {
        return bridge_credential_rejected();
    };
    if !lease_still_valid(runtime.as_ref(), &state.bridge, &session_id, &client_id) {
        state.bridge.disconnect_reader(&session_id);
        return lease_rejected();
    }
    match state
        .bridge
        .complete(&session_id, &token, &client_id, result)
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => error.into_http_response(),
    }
}

/// `POST /control/v1/client-bridge/{session_id}/cancel` — cancel one request.
async fn cancel_bridge_request(
    State(state): State<ControlPlaneState>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let Some(runtime) = state.runtime.as_ref() else {
        return runtime_absent_response();
    };
    let token = match bridge_session_header(&headers) {
        Ok(token) => token.to_string(),
        Err(error) => return error.into_http_response(),
    };
    let request: ClientBridgeCancelRequest = match serde_json::from_str(&body) {
        Ok(request) => request,
        Err(error) => {
            return dto::error_response(
                StatusCode::BAD_REQUEST,
                "invalid_input",
                format!("Invalid bridge cancel request: {error}"),
            )
        }
    };
    let Some(client_id) = state.bridge.client_id_for_session(&session_id, &token) else {
        return bridge_credential_rejected();
    };
    if !lease_still_valid(runtime.as_ref(), &state.bridge, &session_id, &client_id) {
        state.bridge.disconnect_reader(&session_id);
        return lease_rejected();
    }
    match state
        .bridge
        .cancel(&session_id, &token, &client_id, &request.request_id)
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => error.into_http_response(),
    }
}

/// `POST /control/v1/client-bridge/{session_id}/unregister` — close a session.
async fn unregister_bridge(
    State(state): State<ControlPlaneState>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let token = match bridge_session_header(&headers) {
        Ok(token) => token.to_string(),
        Err(error) => return error.into_http_response(),
    };
    if !body.trim().is_empty() {
        if let Err(error) = serde_json::from_str::<ClientBridgeUnregisterRequest>(&body) {
            return dto::error_response(
                StatusCode::BAD_REQUEST,
                "invalid_input",
                format!("Invalid bridge unregister request: {error}"),
            );
        }
    }
    let Some(client_id) = state.bridge.client_id_for_session(&session_id, &token) else {
        // Unregistering an already-gone session is a no-op so shutdown races
        // do not turn into spurious errors.
        return StatusCode::NO_CONTENT.into_response();
    };
    match state.bridge.unregister(&session_id, &token, &client_id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => error.into_http_response(),
    }
}

fn bridge_credential_rejected() -> Response {
    dto::error_response(
        StatusCode::FORBIDDEN,
        "forbidden",
        "invalid bridge session credential".to_string(),
    )
}

fn lease_rejected() -> Response {
    dto::error_response(
        StatusCode::FORBIDDEN,
        "forbidden",
        "the bridge session's client lease is no longer valid".to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chatspeed_contracts::ClientCapabilityStatus;
    use serde_json::json;

    fn tauri_lease() -> ClientLease {
        ClientLease {
            client_id: "tauri-main".to_string(),
            lease_id: "lease-1".to_string(),
            client_kind: BRIDGE_CLIENT_KIND.to_string(),
            expires_at: "1970-01-01T00:00:00Z".to_string(),
        }
    }

    fn declaration() -> ClientBridgeDeclaration {
        ClientBridgeDeclaration {
            protocol_version: BRIDGE_PROTOCOL_VERSION.to_string(),
            schema_version: BRIDGE_SCHEMA_VERSION.to_string(),
            capabilities: vec![
                ClientBridgeCapability {
                    name: "web_fetch".to_string(),
                    schema_version: BRIDGE_SCHEMA_VERSION.to_string(),
                },
                ClientBridgeCapability {
                    name: "web_search".to_string(),
                    schema_version: BRIDGE_SCHEMA_VERSION.to_string(),
                },
            ],
        }
    }

    #[test]
    fn a_declaration_outside_the_allowlist_is_refused() {
        let registry = ClientBridgeRegistry::with_defaults();
        let mut bad = declaration();
        bad.capabilities[0].name = "filesystem_read".to_string();
        assert!(matches!(
            registry.register(&tauri_lease(), &bad),
            Err(BridgeError::Forbidden(_))
        ));

        let mut wrong_schema = declaration();
        wrong_schema.schema_version = "999".to_string();
        assert!(matches!(
            registry.register(&tauri_lease(), &wrong_schema),
            Err(BridgeError::InvalidInput(_))
        ));
    }

    #[test]
    fn the_registry_reflects_only_the_live_bridge_declaration() {
        let registry = ClientBridgeRegistry::with_defaults();
        assert!(registry
            .capability_registry()
            .iter()
            .all(|entry| entry.status == "unavailable" && !entry.bridge_declared));

        let session = registry
            .register(&tauri_lease(), &declaration())
            .expect("register");
        assert!(registry.has_capability("web_fetch"));
        assert!(registry
            .capability_registry()
            .iter()
            .all(|entry| entry.status == "available" && entry.bridge_declared));

        registry
            .unregister(&session.session_id, &session.session_token, "tauri-main")
            .expect("unregister");
        assert!(!registry.has_capability("web_fetch"));
        assert_eq!(registry.session_count(), 0);
    }

    #[tokio::test]
    async fn a_result_reaches_the_waiter_only_with_ownership() {
        let registry = ClientBridgeRegistry::with_defaults();
        let session = registry
            .register(&tauri_lease(), &declaration())
            .expect("register");
        let rx = registry
            .enqueue(
                &session.session_id,
                "web_fetch",
                BRIDGE_SCHEMA_VERSION,
                json!({"url": "https://example.com"}),
                Duration::from_secs(5),
            )
            .expect("enqueue");
        let request_id = registry
            .in_flight_request_ids(&session.session_id)
            .remove(0);

        // A wrong session credential may not complete the request.
        let intruder = registry
            .register(
                &ClientLease {
                    client_id: "tauri-other".to_string(),
                    lease_id: "lease-2".to_string(),
                    client_kind: BRIDGE_CLIENT_KIND.to_string(),
                    expires_at: "1970-01-01T00:00:00Z".to_string(),
                },
                &declaration(),
            )
            .expect("second session");
        assert!(registry
            .complete(
                &session.session_id,
                "wrong-token",
                "tauri-main",
                ClientCapabilityResult {
                    request_id: request_id.clone(),
                    status: ClientCapabilityStatus::Ok,
                    result: Some(json!({"content": "x"})),
                    error: None,
                },
            )
            .is_err());
        assert!(registry
            .complete(
                &intruder.session_id,
                &intruder.session_token,
                "tauri-other",
                ClientCapabilityResult {
                    request_id: request_id.clone(),
                    status: ClientCapabilityStatus::Ok,
                    result: Some(json!({"content": "x"})),
                    error: None,
                },
            )
            .is_err());

        // The owning session completes it and the waiter observes the result.
        registry
            .complete(
                &session.session_id,
                &session.session_token,
                "tauri-main",
                ClientCapabilityResult {
                    request_id: request_id.clone(),
                    status: ClientCapabilityStatus::Ok,
                    result: Some(json!({"content": "x"})),
                    error: None,
                },
            )
            .expect("owner completes");
        match rx.await.expect("waiter outcome") {
            BridgeOutcome::Completed(result) => {
                assert_eq!(result.request_id, request_id);
                assert_eq!(result.status, ClientCapabilityStatus::Ok);
            }
            other => panic!("expected a completed result, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn cancel_fails_the_waiter_and_is_idempotent() {
        let registry = ClientBridgeRegistry::with_defaults();
        let session = registry
            .register(&tauri_lease(), &declaration())
            .expect("register");
        let rx = registry
            .enqueue(
                &session.session_id,
                "web_search",
                BRIDGE_SCHEMA_VERSION,
                json!({"query": "rust"}),
                Duration::from_secs(5),
            )
            .expect("enqueue");
        let request_id = registry
            .in_flight_request_ids(&session.session_id)
            .remove(0);

        registry
            .cancel(
                &session.session_id,
                &session.session_token,
                "tauri-main",
                &request_id,
            )
            .expect("cancel");
        // Cancelling again is a no-op, not an error.
        registry
            .cancel(
                &session.session_id,
                &session.session_token,
                "tauri-main",
                &request_id,
            )
            .expect("idempotent cancel");
        match rx.await.expect("waiter outcome") {
            BridgeOutcome::Failed(error) => assert_eq!(error.code, "cancelled"),
            other => panic!("expected a cancellation, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn unregister_clears_pending_work() {
        let registry = ClientBridgeRegistry::with_defaults();
        let session = registry
            .register(&tauri_lease(), &declaration())
            .expect("register");
        let rx = registry
            .enqueue(
                &session.session_id,
                "web_fetch",
                BRIDGE_SCHEMA_VERSION,
                json!({"url": "https://example.com"}),
                Duration::from_secs(5),
            )
            .expect("enqueue");
        registry
            .unregister(&session.session_id, &session.session_token, "tauri-main")
            .expect("unregister");
        assert_eq!(registry.session_count(), 0);
        match rx.await.expect("waiter outcome") {
            BridgeOutcome::Failed(error) => assert_eq!(error.code, "cancelled"),
            other => panic!("expected a cancellation, got {other:?}"),
        }
    }

    #[test]
    fn the_bounded_queue_and_in_flight_limit_are_enforced() {
        let registry = ClientBridgeRegistry::new(BridgeLimits {
            queue_capacity: 1,
            max_in_flight: 1,
            session_ttl: DEFAULT_SESSION_TTL,
        });
        let session = registry
            .register(&tauri_lease(), &declaration())
            .expect("register");
        // The first fill occupies the only in-flight slot.
        let _first = registry
            .enqueue(
                &session.session_id,
                "web_fetch",
                BRIDGE_SCHEMA_VERSION,
                json!({"url": "https://example.com/1"}),
                Duration::from_secs(5),
            )
            .expect("first enqueue");
        // The second is refused because max_in_flight is reached.
        assert!(matches!(
            registry.enqueue(
                &session.session_id,
                "web_fetch",
                BRIDGE_SCHEMA_VERSION,
                json!({"url": "https://example.com/2"}),
                Duration::from_secs(5),
            ),
            Err(BridgeError::Busy(_))
        ));
    }

    #[test]
    fn an_undeclared_capability_cannot_be_enqueued() {
        let registry = ClientBridgeRegistry::with_defaults();
        let mut only_fetch = declaration();
        only_fetch.capabilities.pop();
        let session = registry
            .register(&tauri_lease(), &only_fetch)
            .expect("register");
        assert!(matches!(
            registry.enqueue(
                &session.session_id,
                "web_search",
                BRIDGE_SCHEMA_VERSION,
                json!({"query": "rust"}),
                Duration::from_secs(5),
            ),
            Err(BridgeError::Unavailable(_))
        ));
    }

    #[tokio::test]
    async fn a_disconnected_reader_clears_pending_work() {
        let registry = ClientBridgeRegistry::with_defaults();
        let session = registry
            .register(&tauri_lease(), &declaration())
            .expect("register");
        let rx = registry
            .enqueue(
                &session.session_id,
                "web_fetch",
                BRIDGE_SCHEMA_VERSION,
                json!({"url": "https://example.com"}),
                Duration::from_secs(5),
            )
            .expect("enqueue");
        registry.disconnect_reader(&session.session_id);
        assert_eq!(registry.session_count(), 0);
        match rx.await.expect("waiter outcome") {
            BridgeOutcome::Failed(error) => assert_eq!(error.code, "cancelled"),
            other => panic!("expected a cancellation, got {other:?}"),
        }
    }

    #[test]
    fn sweeping_drops_sessions_without_a_live_lease() {
        let registry = ClientBridgeRegistry::with_defaults();
        let session = registry
            .register(&tauri_lease(), &declaration())
            .expect("register");
        // A session whose lease no longer resolves is dropped.
        registry.sweep_invalid_leases(|_, _| false);
        assert_eq!(registry.session_count(), 0);
        assert!(registry
            .client_id_for_session(&session.session_id, &session.session_token)
            .is_none());
    }
}
