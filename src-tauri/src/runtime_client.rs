//! Desktop-side supervisor for the standalone `chatspeed-runtime`.
//!
//! The runtime process is the single owner of the canonical database, the
//! chat/tool state, the workflow engine and the loopback HTTP/SSE control plane.
//! The desktop application talks to it as one more control-plane client: it
//! attaches to an already-published runtime when one is live, starts one only
//! when the runtime directory genuinely has no discovery document or the
//! published document names a process that has confirmed exited, completes the
//! readiness handshake, and then holds a client lease with a background
//! heartbeat.
//!
//! This module deliberately owns **no** runtime state: there is no `MainStore`,
//! `ChatState`, `WorkflowManager`, `WorkflowApplicationService` or `ToolManager`
//! here. It is a thin, thread-safe supervisor around the reusable
//! `chatspeed-runtime-client`, so later command adapters can delegate to it
//! without reintroducing a second owner.
//!
//! The desktop no longer starts or invokes the legacy client-pull bridge in
//! production. The bridge protocol types remain available for compatibility
//! tests, while the fixed WebView provider uses rmcp over loopback and runtime
//! registration.

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use chatspeed_contracts::{
    ClientBridgeCapability, ClientBridgeDeclaration, ClientBridgeWorkEnvelope,
    ClientCapabilityInvocation, ClientCapabilityResult, ErrorEnvelope, RuntimeShutdownRequest,
    UserErrorPresentation, BRIDGE_PROTOCOL_VERSION, BRIDGE_SCHEMA_VERSION,
    RUNTIME_CODE_RUNTIME_STOPPING,
};
use chatspeed_runtime_client::{
    process_alive, runtime_needs_start, BridgeSession, ClientError, Heartbeat, LeaseGuard,
    RuntimeChild, RuntimeClient, RuntimeLaunchConfig,
    RuntimeShutdownOutcome as ClientShutdownOutcome,
};
use serde::Serialize;
#[cfg(feature = "desktop")]
use tauri::Emitter;
use tokio::sync::{watch, Mutex};

/// Client kind the desktop registers its lease under.
pub const CLIENT_KIND: &str = "tauri";

/// Stable client id for the single desktop application instance.
pub const DESKTOP_CLIENT_ID: &str = "tauri-main";

/// Longest accepted client id; matches the runtime's own limit.
const MAX_CLIENT_ID_LEN: usize = 128;

/// How long an already-published runtime may take to answer the handshake.
const ATTACH_READY_TIMEOUT: Duration = Duration::from_secs(3);

/// How long a freshly started runtime may take to publish discovery and answer.
const SPAWN_READY_TIMEOUT: Duration = Duration::from_secs(10);

/// A runtime that is already quiescing must finish before a new desktop can
/// register its lease. Retry for one bounded shutdown window instead of
/// surfacing a transient `runtime_stopping` error as a permanent auth failure.
const RUNTIME_STOPPING_RETRY_WINDOW: Duration = Duration::from_secs(35);
const RUNTIME_STOPPING_RETRY_INTERVAL: Duration = Duration::from_millis(500);

/// How long to wait for a cancelled bridge reader to stop before unregistering.
const BRIDGE_READER_STOP_TIMEOUT: Duration = Duration::from_secs(2);

/// Backoff between bridge reader reconnect attempts after a transport failure.
const BRIDGE_READER_RETRY: Duration = Duration::from_millis(500);

/// Maximum bridge reader reconnect attempts before giving up.
///
/// The reader reconnects only when the session may still be live; a stream that
/// the runtime deliberately ended (unregister, disconnect, expired lease) stops
/// the reader immediately instead of reconnecting.
const BRIDGE_READER_MAX_RETRIES: usize = 3;

/// Backoff between workflow event stream reconnect attempts after an error.
const WORKFLOW_READER_RETRY: Duration = Duration::from_millis(500);

/// Maximum consecutive workflow stream reconnect attempts before giving up.
///
/// The counter resets whenever an event is forwarded, so a long-lived session
/// may reconnect indefinitely across transient drops while a stream that keeps
/// failing without delivering anything stops instead of looping forever.
const WORKFLOW_READER_MAX_RETRIES: usize = 5;

/// How long to wait for a cancelled workflow reader to stop before releasing.
const WORKFLOW_READER_STOP_TIMEOUT: Duration = Duration::from_secs(2);

/// Backoff between supervisor health checks and recovery attempts.
const RUNTIME_HEALTH_CHECK_INTERVAL: Duration = Duration::from_secs(2);

/// Reachability must not wait for the lease renewal cadence or a full RPC timeout.
const RUNTIME_HEALTH_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// A future that resolves to one typed capability result.
pub type BridgeDispatchFuture =
    Pin<Box<dyn Future<Output = ClientCapabilityResult> + Send + 'static>>;

/// Executes one bridged client capability.
///
/// The dispatcher is the only code that runs a capability on behalf of the
/// runtime. It receives a fully typed [`ClientCapabilityInvocation`] and must
/// return a typed [`ClientCapabilityResult`]; it must not expose a generic RPC,
/// arbitrary Tauri command or direct database/filesystem passthrough.
pub trait BridgeDispatcher: Send + Sync + 'static {
    /// Executes one invocation and returns its terminal result.
    fn dispatch(&self, invocation: ClientCapabilityInvocation) -> BridgeDispatchFuture;
}

/// The fixed, closed declaration the desktop bridge publishes.
///
/// Both web capabilities are declared at the current schema version; the
/// runtime rejects anything outside this allowlist.
pub fn web_bridge_declaration() -> ClientBridgeDeclaration {
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

/// Resolves the typed launch configuration the desktop defaults to.
///
/// The shared runtime client already encodes the path priority
/// (`CHATSPEED_RUNTIME_DIR`, then `${CHATSPEED_HOME}/runtime`, then the
/// build-profile default), and it resolves the runtime directory, database and
/// application-data directory together, so the desktop, `cscli` and the runtime
/// binary can never disagree about where discovery and the database live.
///
/// Fails closed when no platform directory is available: there is no
/// `.`-relative fallback.
pub fn default_launch_config() -> Result<RuntimeLaunchConfig, RuntimeUnavailable> {
    chatspeed_runtime_client::resolve_launch_config().map_err(|error| {
        RuntimeUnavailable::InvalidConfig(format!("cannot resolve runtime paths: {error}"))
    })
}

/// Resolves the runtime directory the runtime binary itself defaults to.
pub fn default_runtime_dir() -> Result<PathBuf, RuntimeUnavailable> {
    default_launch_config().map(|config| config.runtime_dir().to_path_buf())
}

// ---------------------------------------------------------------------------
// Errors and status
// ---------------------------------------------------------------------------

/// Why the desktop cannot use the standalone runtime.
///
/// Every variant maps a [`ClientError`] to a stable, redacted classification.
/// There is no fallback: an unreachable, protocol-incompatible or malformed
/// endpoint is reported as-is so a caller never silently speaks to the wrong
/// service or starts a second runtime over a live one.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeUnavailable {
    /// The discovery document is missing, unreadable or malformed.
    #[error("runtime discovery is unavailable: {0}")]
    Discovery(String),
    /// The runtime reports an incompatible protocol or an unrecognized service.
    #[error("runtime protocol mismatch: {0}")]
    Protocol(String),
    /// The runtime could not be reached over the loopback endpoint.
    #[error("runtime is unreachable: {0}")]
    Unreachable(String),
    /// The runtime rejected this client's authentication.
    #[error("runtime rejected this client: {0}")]
    Rejected(String),
    /// The runtime returned a structured error. The message keeps the
    /// historical `runtime returned an error: <status> <code>: <message>`
    /// shape, while the typed payload preserves the status, the canonical code
    /// and the optional safe presentation for callers that branch on them.
    #[error("runtime returned an error: {0}")]
    Server(RuntimeServerError),
    /// The caller asked for something the supervisor refuses to do.
    #[error("invalid runtime client configuration: {0}")]
    InvalidConfig(String),
    /// No lease is held, so the control-plane client is not available.
    #[error("runtime client is not connected")]
    NotConnected,
}

/// Typed structured error the runtime returned over the control plane.
///
/// `Display` reproduces the historical `Server(String)` payload
/// (`<status> <code>: <message>`) so adapters that format the supervisor error
/// keep the same text, while the fields let callers branch on the canonical
/// code without reconstructing it from the message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeServerError {
    /// HTTP status the runtime answered with.
    pub status: u16,
    /// Canonical error envelope code, e.g. `runtime_stopping`.
    pub code: String,
    /// Redacted wire message.
    pub message: String,
    /// Safe presentation the runtime attached, when present.
    pub presentation: Option<UserErrorPresentation>,
}

impl fmt::Display for RuntimeServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}: {}", self.status, self.code, self.message)
    }
}

impl RuntimeUnavailable {
    /// Classifies a client error and scrubs it before it is stored or logged.
    pub(crate) fn from_client_error(error: ClientError) -> Self {
        match error {
            ClientError::Discovery(message) => Self::Discovery(redact_secrets(&message)),
            ClientError::Protocol(message) => Self::Protocol(redact_secrets(&message)),
            ClientError::Transport(message) => Self::Unreachable(redact_secrets(&message)),
            ClientError::Auth(auth) => Self::Rejected(redact_secrets(&auth.to_string())),
            ClientError::Server(response) => Self::Server(RuntimeServerError {
                status: response.status,
                code: response.code,
                message: redact_secrets(&response.message),
                presentation: response.presentation,
            }),
            ClientError::InvalidRequest(message) => Self::InvalidConfig(message),
            ClientError::Serialization(message) => Self::Protocol(redact_secrets(&message)),
        }
    }

    /// Reports whether the structured server error carries the stable
    /// `runtime_stopping` code, so retry logic never reconstructs it from text.
    pub fn is_runtime_stopping(&self) -> bool {
        matches!(self, Self::Server(response) if response.code == RUNTIME_CODE_RUNTIME_STOPPING)
    }
}

/// Serializes one public runtime rejection for the Tauri IPC boundary.
///
/// Tauri commands still reject with `String`, so the adapter emits the shared
/// envelope as JSON instead of flattening it. `code` and `message` stay the
/// legacy pair old readers consume, `presentation` is optional and already
/// redacted by the runtime, and `status` preserves the rejection's HTTP
/// meaning. Only the runtime's public `Server`/`Auth` rejections use this
/// helper; internal `String` failures must not be wrapped as user-facing JSON.
pub(crate) fn public_rejection_wire(
    status: u16,
    code: &str,
    legacy_message: &str,
    presentation: Option<UserErrorPresentation>,
) -> String {
    let mut envelope = ErrorEnvelope::new(code, legacy_message);
    envelope.error.presentation = presentation;
    let mut wire = serde_json::to_value(envelope).unwrap_or_else(|_| {
        serde_json::json!({
            "error": { "code": code, "message": legacy_message }
        })
    });
    if let Some(object) = wire.as_object_mut() {
        object.insert("status".to_string(), serde_json::json!(status));
    }
    wire.to_string()
}

/// Maps one control-plane client error to the IPC string error.
///
/// Public rejections keep their structured envelope; transport, discovery,
/// protocol and serialization failures stay technical strings. No variant
/// contributes a source chain to the wire.
pub(crate) fn client_error_wire(error: ClientError) -> String {
    match error {
        ClientError::Server(response) => public_rejection_wire(
            response.status,
            &response.code,
            &response.message,
            response.presentation,
        ),
        ClientError::Auth(auth) => {
            public_rejection_wire(auth.status, &auth.code, &auth.message, auth.presentation)
        }
        ClientError::Transport(message) => transport_cause(&message).to_string(),
        other => other.to_string(),
    }
}

fn transport_cause(message: &str) -> &str {
    message.strip_prefix("request failed: ").unwrap_or(message)
}

/// Maps one supervisor failure to the IPC string error.
///
/// A structured runtime rejection keeps the same status/code/presentation
/// envelope as the direct client path, so an availability failure cannot erase
/// the classification. Every other variant keeps its technical description.
pub(crate) fn runtime_error_wire(error: RuntimeUnavailable) -> String {
    match error {
        RuntimeUnavailable::Server(response) => public_rejection_wire(
            response.status,
            &response.code,
            &response.message,
            response.presentation,
        ),
        other => other.to_string(),
    }
}

/// Lifecycle of the desktop's relationship with the runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeConnectionState {
    /// No connection attempt has completed yet.
    Disconnected,
    /// Attached to a runtime that another process started.
    Attached,
    /// Attached to a runtime this process started.
    Spawned,
    /// The lease was released (application shutdown or explicit release).
    Released,
    /// The last connection attempt failed; see `last_error`.
    Unavailable,
}

/// Redacted, serializable snapshot of the supervisor.
///
/// It deliberately omits the bearer token, the opaque lease id and the runtime
/// child's captured stderr. Only diagnostic values that are safe to show (loop
/// address, client id, runtime directory, child pid) are reported.
#[derive(Debug, Clone, Serialize)]
pub struct RuntimeStatus {
    /// Current lifecycle state.
    pub state: RuntimeConnectionState,
    /// Client id this process registered, once connected.
    pub client_id: Option<String>,
    /// Loopback base URL of the connected control plane.
    pub base_url: Option<String>,
    /// Runtime directory the supervisor was pointed at.
    pub runtime_dir: Option<PathBuf>,
    /// Whether this process started the runtime and still holds its handle.
    pub owns_runtime_child: bool,
    /// Pid of the runtime child, when this process started it.
    pub runtime_child_pid: Option<u32>,
    /// Whether the last lease renewal succeeded; `None` before a connection.
    pub heartbeat_ok: Option<bool>,
    /// Whether a client WebView capability bridge session is live.
    pub bridge_active: bool,
    /// RFC 3339 lease expiry reported by the runtime.
    pub lease_expires_at: Option<String>,
    /// Redacted message from the last failed connection attempt, if any.
    pub last_error: Option<String>,
    /// Monotonically increasing connection generation. It changes only after
    /// the lease and all connection state have been installed.
    pub generation: u64,
}

/// Runtime instance identity a coordinated shutdown must target.
///
/// It is read from the live status route so the request can prove it addresses
/// the instance the user confirmed rather than a stale discovery document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeIdentity {
    /// Instance id reported by the live runtime.
    pub instance_id: String,
    /// Pid reported by the live runtime.
    pub pid: u32,
}

/// Observed end of a forced coordinated runtime shutdown.
///
/// Only [`Self::Confirmed`] is a completed stop: a `202 Accepted` is an
/// acceptance, so a gone endpoint or an elapsed budget stays unconfirmed and is
/// never reported as a graceful exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeStopOutcome {
    /// The runtime's own status route reported the `stopped` phase.
    Confirmed,
    /// The control-plane endpoint disappeared before reporting `stopped`.
    EndpointGone,
    /// The wait budget elapsed while the runtime was still up.
    TimedOut,
    /// This desktop holds no connected runtime client.
    NotConnected,
    /// The request or the observation failed with a structured error.
    Failed,
}

impl RuntimeStopOutcome {
    /// Stable code the frontend maps to a localized, non-technical message.
    pub fn code(self) -> &'static str {
        match self {
            Self::Confirmed => "runtime_stop_confirmed",
            Self::EndpointGone => "runtime_endpoint_gone",
            Self::TimedOut => "runtime_stop_timeout",
            Self::NotConnected => "runtime_not_connected",
            Self::Failed => "runtime_stop_failed",
        }
    }
}

/// Result of one forced coordinated shutdown attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStopReport {
    /// What the observation concluded.
    pub outcome: RuntimeStopOutcome,
    /// Instance the request targeted; `None` when no client was available.
    pub instance_id: Option<String>,
    /// Operation id the runtime accepted, when one was reported.
    pub operation_id: Option<String>,
    /// Redacted failure detail, for logs only.
    pub detail: Option<String>,
}

// ---------------------------------------------------------------------------
// Supervisor
// ---------------------------------------------------------------------------

struct MonitorState {
    started: AtomicBool,
    stop: watch::Sender<bool>,
}

/// Thread-safe supervisor around the desktop's relationship with the runtime.
///
/// It owns the [`RuntimeClient`], the [`LeaseGuard`] and its [`Heartbeat`], and
/// (only when this process started the runtime) the [`RuntimeChild`] handle. It
/// never owns runtime state. Tauri manages one `Arc<RuntimeSupervisor>`, and
/// every accessor is safe to call from any command.
pub struct RuntimeSupervisor {
    inner: Arc<Mutex<SupervisorState>>,
    monitor: Arc<MonitorState>,
}

struct SupervisorState {
    connection: Option<Connection>,
    state: RuntimeConnectionState,
    runtime_dir: Option<PathBuf>,
    last_error: Option<String>,
    launch_config: Option<RuntimeLaunchConfig>,
    client_id: Option<String>,
    connecting: bool,
    stopping: bool,
    generation: u64,
    pending_child: Option<RuntimeChild>,
    pending_pid: Option<u32>,
}

/// Everything the supervisor owns once a lease is held.
struct Connection {
    client: RuntimeClient,
    lease: LeaseGuard,
    heartbeat: Option<Heartbeat>,
    child: Option<RuntimeChild>,
    client_id: String,
    bridge: Option<BridgeHandle>,
    /// Live workflow SSE forwarding tasks, at most one per session id.
    workflow_streams: HashMap<String, WorkflowStreamHandle>,
    /// The desktop loopback Web MCP provider, when one is registered.
    #[cfg(feature = "desktop")]
    web_provider: Option<crate::runtime_web_mcp_provider::WebMcpProviderHandle>,
}

/// A live client WebView capability bridge session and its reader.
struct BridgeHandle {
    session: BridgeSession,
    cancel: watch::Sender<bool>,
    reader: tokio::task::JoinHandle<()>,
}

/// A live workflow SSE forwarding task for one session.
///
/// The task reads the runtime's versioned workflow stream and re-emits each
/// envelope's raw payload as the existing `workflow://event/{session}` Tauri
/// event. Cancelling it drops the reader so the session is no longer forwarded.
struct WorkflowStreamHandle {
    cancel: watch::Sender<bool>,
    reader: tokio::task::JoinHandle<()>,
}

impl SupervisorState {
    /// Builds the redacted status snapshot. Never reads a secret.
    fn snapshot(&self) -> RuntimeStatus {
        let connection = self.connection.as_ref();
        RuntimeStatus {
            state: self.state,
            client_id: connection.map(|c| c.client_id.clone()),
            base_url: connection.map(|c| c.client.base_url().to_string()),
            runtime_dir: self.runtime_dir.clone(),
            owns_runtime_child: connection.is_some_and(|c| c.child.is_some())
                || self.pending_child.is_some(),
            runtime_child_pid: connection
                .and_then(|c| c.child.as_ref())
                .map(RuntimeChild::id)
                .or(self.pending_pid),
            heartbeat_ok: connection
                .and_then(|c| c.heartbeat.as_ref())
                .map(|heartbeat| heartbeat.status().is_ok()),
            bridge_active: connection.is_some_and(|c| c.bridge.is_some()),
            lease_expires_at: connection.map(|c| c.lease.expires_at().to_string()),
            last_error: self.last_error.clone(),
            generation: self.generation,
        }
    }
}

impl RuntimeSupervisor {
    /// Creates an inert supervisor.
    ///
    /// Nothing is started and no socket is opened: this exists so Tauri `setup`
    /// can put the supervisor into managed state before any I/O, keeping the
    /// first paint free of blocking work. Call
    /// [`RuntimeSupervisor::connect_or_start`] next.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(SupervisorState {
                connection: None,
                state: RuntimeConnectionState::Disconnected,
                runtime_dir: None,
                last_error: None,
                launch_config: None,
                client_id: None,
                connecting: false,
                stopping: false,
                generation: 0,
                pending_child: None,
                pending_pid: None,
            })),
            monitor: Arc::new(MonitorState {
                started: AtomicBool::new(false),
                stop: watch::channel(false).0,
            }),
        }
    }

    /// Attaches to a live runtime or starts one, then holds a client lease.
    ///
    /// The launch configuration is the runtime's identity: a published discovery
    /// document in its runtime directory is attached to only after the readiness
    /// handshake proves it is the expected standalone runtime, and a runtime is
    /// started only when that document is genuinely absent or names a process
    /// that has confirmed exited. A started runtime receives every path
    /// explicitly, so an inherited environment or a cross-profile binary can
    /// never redirect it.
    ///
    /// An unreachable, protocol-incompatible or malformed endpoint is reported as
    /// [`RuntimeUnavailable`] and never silently worked around.
    ///
    /// Expected to run once, from a background task; other accessors wait until
    /// the attempt settles.
    pub async fn connect_or_start(
        &self,
        config: &RuntimeLaunchConfig,
        client_id: &str,
    ) -> Result<(), RuntimeUnavailable> {
        validate_client_id(client_id)?;
        self.start_health_monitor();
        let deadline = tokio::time::Instant::now() + RUNTIME_STOPPING_RETRY_WINDOW;
        let mut logged_stopping = false;
        loop {
            match self.connect_or_start_once(config, client_id).await {
                Err(error) if error.is_runtime_stopping() => {
                    let now = tokio::time::Instant::now();
                    if now >= deadline {
                        return Err(error);
                    }
                    if !logged_stopping {
                        log::warn!(
                            "[RuntimeSupervisor] runtime is stopping; waiting for it to exit before retrying lease registration"
                        );
                        logged_stopping = true;
                    }
                    tokio::time::sleep(RUNTIME_STOPPING_RETRY_INTERVAL.min(deadline - now)).await;
                }
                result => return result,
            }
        }
    }

    async fn connect_or_start_once(
        &self,
        config: &RuntimeLaunchConfig,
        client_id: &str,
    ) -> Result<(), RuntimeUnavailable> {
        connect_once(self.inner.clone(), config, client_id).await
    }

    fn start_health_monitor(&self) {
        if self
            .monitor
            .started
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        let inner = Arc::downgrade(&self.inner);
        let monitor = self.monitor.clone();
        tokio::spawn(async move { run_health_monitor(inner, monitor).await });
    }

    /// Returns a clone of the connected control-plane client.
    ///
    /// This is the only RPC surface this unit exposes: callers use the client's
    /// typed `get`/`post`/`post_json`/`stream` methods against the documented
    /// `/control/v1` routes. A generic method/path passthrough is deliberately
    /// not provided.
    pub async fn client(&self) -> Result<RuntimeClient, RuntimeUnavailable> {
        let state = self.inner.lock().await;
        match &state.connection {
            Some(connection) if state.state != RuntimeConnectionState::Released => {
                Ok(connection.client.clone())
            }
            _ => Err(RuntimeUnavailable::NotConnected),
        }
    }

    /// Returns the connected client plus the exact lease identity used by typed
    /// runtime-owned interactive terminal routes.
    ///
    /// The lease id is exposed only to the in-process adapter so it can be sent
    /// in the dedicated terminal proof header; it is never serialized into a
    /// generic request, URL or log message.
    pub async fn terminal_connection(
        &self,
    ) -> Result<(RuntimeClient, String, String), RuntimeUnavailable> {
        let state = self.inner.lock().await;
        let Some(connection) = state.connection.as_ref() else {
            return Err(RuntimeUnavailable::NotConnected);
        };
        if state.state == RuntimeConnectionState::Released {
            return Err(RuntimeUnavailable::NotConnected);
        }
        Ok((
            connection.client.clone(),
            connection.client_id.clone(),
            connection.lease.lease_id().to_string(),
        ))
    }

    /// Whether desktop exit has fenced off further recovery.
    pub async fn is_stopping(&self) -> bool {
        self.inner.lock().await.stopping
    }

    /// Redacted snapshot for a status command or a log line.
    pub async fn status(&self) -> RuntimeStatus {
        self.inner.lock().await.snapshot()
    }

    /// Returns whether the exact runtime process has exited.
    ///
    /// A runtime this desktop spawned is checked with `try_wait`, which also
    /// reaps a just-exited child. An attached runtime uses the conservative
    /// platform liveness probe because this process does not own its child
    /// handle.
    pub async fn runtime_process_exited(&self, expected_pid: u32) -> bool {
        let mut state = self.inner.lock().await;
        if let Some(connection) = state.connection.as_mut() {
            if let Some(child) = connection.child.as_mut() {
                if child.id() == expected_pid {
                    return matches!(child.try_wait(), Ok(Some(_)));
                }
            }
        }
        !process_alive(expected_pid)
    }

    /// Waits for the exact runtime process to exit after its endpoint disappears.
    ///
    /// The runtime closes its listener before the serve task finishes draining
    /// and releasing its process. A single PID probe in that interval races the
    /// normal shutdown path and falsely reports an unknown state, so keep
    /// observing the target until the caller's shutdown budget is exhausted.
    async fn wait_for_runtime_process_exit(&self, expected_pid: u32, deadline: Instant) -> bool {
        loop {
            if self.runtime_process_exited(expected_pid).await {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            tokio::time::sleep((deadline - now).min(Duration::from_millis(100))).await;
        }
    }

    /// Opens the client WebView capability bridge and starts its reader.
    ///
    /// The session is registered with the supervisor's own client id and the
    /// opaque lease id, so the runtime resolves a live `tauri` lease itself.
    /// The reader pulls typed work envelopes and hands each to `dispatcher`.
    /// Registering twice without an intervening release is refused.
    pub async fn start_bridge(
        &self,
        declaration: ClientBridgeDeclaration,
        dispatcher: Arc<dyn BridgeDispatcher>,
    ) -> Result<(), RuntimeUnavailable> {
        let mut state = self.inner.lock().await;
        if state.state == RuntimeConnectionState::Released {
            return Err(RuntimeUnavailable::NotConnected);
        }
        let Some(connection) = state.connection.as_mut() else {
            return Err(RuntimeUnavailable::NotConnected);
        };
        if connection.bridge.is_some() {
            return Err(RuntimeUnavailable::InvalidConfig(
                "the runtime bridge is already connected".to_string(),
            ));
        }
        let lease_id = connection.lease.lease_id().to_string();
        let client_id = connection.client_id.clone();
        let response = connection
            .client
            .register_bridge(&declaration, &client_id, &lease_id)
            .await
            .map_err(RuntimeUnavailable::from_client_error)?;
        let session = BridgeSession::new(
            connection.client.clone(),
            response.session_id,
            response.session_token,
        );
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let reader = tokio::spawn(run_bridge_reader(session.clone(), dispatcher, cancel_rx));
        connection.bridge = Some(BridgeHandle {
            session,
            cancel: cancel_tx,
            reader,
        });
        log::info!("[RuntimeSupervisor] client WebView bridge connected");
        Ok(())
    }

    /// Ensures one workflow session's live SSE stream is forwarded as Tauri events.
    ///
    /// The SSE subscription is established before this returns, so a caller that
    /// starts the workflow immediately afterwards cannot publish an event into a
    /// stream no one is reading yet. At most one forwarding task exists per
    /// session: a repeated call for a live session is a no-op, and a task that
    /// already finished (terminal state or a closed session) is replaced so a
    /// restarted session can subscribe again.
    #[cfg(feature = "desktop")]
    pub async fn ensure_workflow_event_stream(
        &self,
        app: tauri::AppHandle,
        session_id: &str,
    ) -> Result<(), RuntimeUnavailable> {
        let (client, session_id, generation) = {
            let mut state = self.inner.lock().await;
            let generation = state.generation;
            if state.state == RuntimeConnectionState::Released {
                return Err(RuntimeUnavailable::NotConnected);
            }
            let Some(connection) = state.connection.as_mut() else {
                return Err(RuntimeUnavailable::NotConnected);
            };
            connection
                .workflow_streams
                .retain(|_, handle| !handle.reader.is_finished());
            if connection.workflow_streams.contains_key(session_id) {
                return Ok(());
            }
            (
                connection.client.clone(),
                session_id.to_string(),
                generation,
            )
        };

        // Open the SSE response before the workflow starts: the subscription is
        // live the moment this future resolves.
        let stream = client
            .stream_workflow(&session_id, None)
            .await
            .map_err(RuntimeUnavailable::from_client_error)?;

        let mut state = self.inner.lock().await;
        if state.stopping
            || state.generation != generation
            || state.state == RuntimeConnectionState::Released
        {
            return Err(RuntimeUnavailable::NotConnected);
        }
        let Some(connection) = state.connection.as_mut() else {
            return Err(RuntimeUnavailable::NotConnected);
        };
        if connection.workflow_streams.contains_key(&session_id) {
            // Another caller established the stream first; drop this duplicate.
            return Ok(());
        }
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let reader = tokio::spawn(run_workflow_event_reader(
            app,
            client,
            session_id.clone(),
            stream,
            cancel_rx,
        ));
        connection.workflow_streams.insert(
            session_id.clone(),
            WorkflowStreamHandle {
                cancel: cancel_tx,
                reader,
            },
        );
        log::info!(
            "[RuntimeSupervisor] workflow event stream established for session {session_id}"
        );
        Ok(())
    }

    /// Stores the desktop loopback Web MCP provider so it is stopped before the
    /// lease is released.
    ///
    /// Registering twice without an intervening release is refused, so a second
    /// provider server is never silently left running.
    #[cfg(feature = "desktop")]
    pub async fn set_web_provider(
        &self,
        handle: crate::runtime_web_mcp_provider::WebMcpProviderHandle,
    ) -> Result<(), RuntimeUnavailable> {
        let mut state = self.inner.lock().await;
        let accepted = !state.stopping
            && state.state != RuntimeConnectionState::Released
            && state.connection.as_ref().is_some_and(|connection| {
                connection.lease.lease_id() == handle.lease_id()
                    && connection.web_provider.is_none()
            });
        if !accepted {
            drop(state);
            handle.shutdown().await;
            return Err(RuntimeUnavailable::NotConnected);
        }
        if let Some(connection) = state.connection.as_mut() {
            connection.web_provider = Some(handle);
        }
        Ok(())
    }

    /// Invokes one runtime-owned client WebView capability through this
    /// supervisor's authenticated bridge session. The ordinary bearer client
    /// is intentionally insufficient; the session credential is attached by
    /// the stored `BridgeSession`.
    pub async fn invoke_client_capability(
        &self,
        capability: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, RuntimeUnavailable> {
        let session = {
            let state = self.inner.lock().await;
            state
                .connection
                .as_ref()
                .and_then(|connection| connection.bridge.as_ref())
                .map(|bridge| bridge.session.clone())
                .ok_or(RuntimeUnavailable::NotConnected)?
        };
        session
            .invoke_client_capability(capability, body)
            .await
            .map_err(RuntimeUnavailable::from_client_error)
    }

    /// Releases the lease and stops the heartbeat.
    ///
    /// A healthy runtime owned by another client is never stopped: with no lease
    /// left, the runtime's own idle grace decides whether it exits. Releasing
    /// twice is a no-op.
    pub async fn release(&self) -> Result<(), RuntimeUnavailable> {
        self.monitor.stop.send_replace(true);
        let connection = {
            let mut state = self.inner.lock().await;
            state.stopping = true;
            state.state = RuntimeConnectionState::Released;
            state.connection.take()
        };
        match connection {
            Some(connection) => cleanup_connection(connection).await,
            None => Ok(()),
        }
    }

    /// Releases the lease when the application exits.
    ///
    /// Identical to [`RuntimeSupervisor::release`] but logs instead of returning,
    /// so the Tauri exit handler can call it without handling a result. It never
    /// kills a runtime this process did not start.
    pub async fn shutdown(&self) {
        if let Err(error) = self.release().await {
            let detail = match &error {
                RuntimeUnavailable::Unreachable(message) => transport_cause(message).to_string(),
                other => other.to_string(),
            };
            log::warn!("[RuntimeSupervisor] releasing the runtime lease failed: {detail}");
        }
    }

    /// Returns the connected client plus the exact lease identity a coordinated
    /// runtime shutdown must present.
    ///
    /// The bearer token proves the client and the opaque lease id is the second
    /// proof the shutdown route requires. Like the terminal proof, neither value
    /// is ever serialized into a generic request, URL or log line.
    pub async fn shutdown_connection(
        &self,
    ) -> Result<(RuntimeClient, String, String), RuntimeUnavailable> {
        let state = self.inner.lock().await;
        let Some(connection) = state.connection.as_ref() else {
            return Err(RuntimeUnavailable::NotConnected);
        };
        if state.state == RuntimeConnectionState::Released {
            return Err(RuntimeUnavailable::NotConnected);
        }
        Ok((
            connection.client.clone(),
            connection.client_id.clone(),
            connection.lease.lease_id().to_string(),
        ))
    }

    /// Reads the live runtime instance identity a coordinated shutdown targets.
    ///
    /// The status route is the authority, so a stale discovery document can never
    /// redirect the request to a different instance.
    pub async fn runtime_identity(&self) -> Result<RuntimeIdentity, RuntimeUnavailable> {
        let client = self.client().await?;
        let status = client
            .runtime_status()
            .await
            .map_err(RuntimeUnavailable::from_client_error)?;
        Ok(RuntimeIdentity {
            instance_id: status.instance_id,
            pid: status.pid,
        })
    }

    /// Prevents health monitoring and recovery while a coordinated runtime stop
    /// is being observed. The existing lease remains available to the caller so
    /// a failed stop can still be reported or retried, but no successor runtime
    /// may be started in the meantime.

    pub async fn begin_stopping(&self) {
        self.monitor.stop.send_replace(true);
        self.inner.lock().await.stopping = true;
    }

    /// waits, within `budget`, for the observed end.
    ///
    /// The request always carries an explicit `confirm`, an explicit `force` and
    /// a stable `Idempotency-Key`, so a retry for the same prompt is idempotent
    /// and never starts a second operation. This never starts, loads or spawns a
    /// runtime: it addresses only the client this desktop already holds, and it
    /// never kills a successor instance.
    ///
    /// A `202 Accepted` is an acceptance, never a completion: only the status
    /// route reporting `stopped` or a confirmed exit of the exact target PID
    /// completes a forced stop. A gone endpoint while the PID is still alive is
    /// kept as a failure so a transient listener failure cannot stop the desktop
    /// from exiting against an unverified runtime.
    pub async fn request_forced_runtime_shutdown(
        &self,
        expected_instance_id: &str,
        expected_pid: u32,
        idempotency_key: &str,
        budget: Duration,
    ) -> RuntimeStopReport {
        self.begin_stopping().await;
        let (client, client_id, lease_id) = match self.shutdown_connection().await {
            Ok(connection) => connection,
            Err(error) => {
                return RuntimeStopReport {
                    outcome: RuntimeStopOutcome::NotConnected,
                    instance_id: None,
                    operation_id: None,
                    detail: Some(error.to_string()),
                }
            }
        };

        let request = RuntimeShutdownRequest {
            expected_instance_id: expected_instance_id.to_string(),
            confirm: true,
            force: true,
        };
        let accepted = match client
            .request_runtime_shutdown(&request, &client_id, &lease_id, idempotency_key)
            .await
        {
            Ok(accepted) => accepted,
            Err(error) => {
                return RuntimeStopReport {
                    outcome: RuntimeStopOutcome::Failed,
                    instance_id: Some(expected_instance_id.to_string()),
                    operation_id: None,
                    detail: Some(RuntimeUnavailable::from_client_error(error).to_string()),
                }
            }
        };

        let instance_id = Some(accepted.instance_id.clone());
        let operation_id = Some(accepted.operation_id.clone());
        let deadline = Instant::now() + budget;
        match client.wait_runtime_stopped(budget).await {
            Ok(ClientShutdownOutcome::Stopped) => RuntimeStopReport {
                outcome: RuntimeStopOutcome::Confirmed,
                instance_id,
                operation_id,
                detail: None,
            },
            Ok(ClientShutdownOutcome::EndpointGone) => {
                let outcome = if self
                    .wait_for_runtime_process_exit(expected_pid, deadline)
                    .await
                {
                    RuntimeStopOutcome::Confirmed
                } else {
                    RuntimeStopOutcome::EndpointGone
                };
                RuntimeStopReport {
                    outcome,
                    instance_id,
                    operation_id,
                    detail: None,
                }
            }
            Ok(ClientShutdownOutcome::TimedOut) => RuntimeStopReport {
                outcome: RuntimeStopOutcome::TimedOut,
                instance_id,
                operation_id,
                detail: None,
            },
            Err(error) => RuntimeStopReport {
                outcome: RuntimeStopOutcome::Failed,
                instance_id,
                operation_id,
                detail: Some(RuntimeUnavailable::from_client_error(error).to_string()),
            },
        }
    }
}

impl Default for RuntimeSupervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for RuntimeSupervisor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.inner.try_lock() {
            Ok(state) => f
                .debug_struct("RuntimeSupervisor")
                .field("state", &state.state)
                .field(
                    "client_id",
                    &state.connection.as_ref().map(|c| c.client_id.as_str()),
                )
                .field("runtime_dir", &state.runtime_dir)
                .field("last_error", &state.last_error)
                .finish(),
            Err(_) => f
                .debug_struct("RuntimeSupervisor")
                .field("state", &"<locked>")
                .finish(),
        }
    }
}

impl Drop for RuntimeSupervisor {
    fn drop(&mut self) {
        self.monitor.stop.send_replace(true);
        let inner = self.inner.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let connection = {
                    let mut state = inner.lock().await;
                    state.stopping = true;
                    state.connection.take()
                };
                if let Some(connection) = connection {
                    let _ = cleanup_connection(connection).await;
                }
            });
        }
    }
}

/// Stops connection-owned readers/providers, then releases its lease.
async fn cleanup_connection(mut connection: Connection) -> Result<(), RuntimeUnavailable> {
    // Stop and join the heartbeat first: a renewal must never be in flight while
    // the lease is released.
    if let Some(heartbeat) = connection.heartbeat.take() {
        heartbeat.stop().await;
    }
    if let Some(bridge) = connection.bridge.take() {
        bridge.cancel.send_replace(true);
        bridge.reader.abort();
        let _ = tokio::time::timeout(BRIDGE_READER_STOP_TIMEOUT, bridge.reader).await;
        if let Err(error) = bridge.session.unregister().await {
            log::debug!("[RuntimeSupervisor] unregistering the bridge failed: {error}");
        }
    }
    let workflow_readers: Vec<WorkflowStreamHandle> = connection
        .workflow_streams
        .drain()
        .map(|(_, handle)| handle)
        .collect();
    for handle in workflow_readers {
        handle.cancel.send_replace(true);
        handle.reader.abort();
        let _ = tokio::time::timeout(WORKFLOW_READER_STOP_TIMEOUT, handle.reader).await;
    }
    #[cfg(feature = "desktop")]
    if let Some(provider) = connection.web_provider.take() {
        provider.shutdown().await;
    }
    connection
        .lease
        .release()
        .await
        .map_err(RuntimeUnavailable::from_client_error)
}

/// Watches control-plane reachability, the lease heartbeat and any desktop-owned
/// child, then reconnects through the same discovery/lock/lease handshake.
async fn run_health_monitor(inner: Weak<Mutex<SupervisorState>>, monitor: Arc<MonitorState>) {
    let mut stop_rx = monitor.stop.subscribe();
    loop {
        tokio::select! {
            _ = tokio::time::sleep(RUNTIME_HEALTH_CHECK_INTERVAL) => {}
            changed = stop_rx.changed() => {
                if changed.is_err() || *stop_rx.borrow() { return; }
                continue;
            }
        }
        let Some(inner) = inner.upgrade() else { return };

        // Never hold the supervisor lock over I/O: status and desktop exit must
        // stay responsive even when the old runtime accepts but stops answering.
        let probe = {
            let state = inner.lock().await;
            if state.stopping {
                return;
            }
            state
                .connection
                .as_ref()
                .map(|connection| (state.generation, connection.client.clone()))
        };
        let probe_failure = if let Some((generation, client)) = probe {
            let result = tokio::select! {
                _ = stop_rx.changed() => return,
                result = tokio::time::timeout(
                    RUNTIME_HEALTH_PROBE_TIMEOUT,
                    client.get(chatspeed_runtime_client::META_PATH),
                ) => result,
            };
            match result {
                Ok(Ok(_)) => None,
                Ok(Err(error)) => Some((
                    generation,
                    format!(
                        "control-plane health probe failed: {}",
                        redact_secrets(&error.to_string())
                    ),
                )),
                Err(_) => Some((
                    generation,
                    "control-plane health probe timed out".to_string(),
                )),
            }
        } else {
            None
        };

        let failed = {
            let mut state = inner.lock().await;
            if state.stopping {
                return;
            }
            let generation = state.generation;
            let reason = state.connection.as_mut().and_then(|connection| {
                if connection
                    .heartbeat
                    .as_ref()
                    .is_some_and(|heartbeat| heartbeat.status().is_err())
                {
                    Some("runtime lease heartbeat failed".to_string())
                } else if connection
                    .child
                    .as_mut()
                    .is_some_and(|child| matches!(child.try_wait(), Ok(Some(_))))
                {
                    Some("spawned runtime child exited".to_string())
                } else {
                    probe_failure
                        .as_ref()
                        .filter(|(probe_generation, _)| *probe_generation == generation)
                        .map(|(_, reason)| reason.clone())
                }
            });
            if let Some(reason) = reason {
                let mut connection = state.connection.take();
                if let Some(connection) = connection.as_mut() {
                    if let Some(child) = connection.child.take() {
                        state.pending_pid = Some(child.id());
                        state.pending_child = Some(child);
                    }
                }
                state.state = RuntimeConnectionState::Unavailable;
                state.last_error = Some(reason.clone());
                Some((connection, reason))
            } else if state.connection.is_none()
                && !state.connecting
                && state.launch_config.is_some()
            {
                Some((None, "runtime unavailable".to_string()))
            } else {
                None
            }
        };

        let Some((connection, reason)) = failed else {
            continue;
        };
        log::warn!("[RuntimeSupervisor] {reason}; starting controlled recovery");
        if let Some(connection) = connection {
            if let Err(error) = cleanup_connection(connection).await {
                log::debug!("[RuntimeSupervisor] cleaning failed runtime connection: {error}");
            }
        }

        let retry = {
            let state = inner.lock().await;
            if state.stopping || state.connection.is_some() || state.connecting {
                None
            } else {
                state.launch_config.clone().zip(state.client_id.clone())
            }
        };
        if let Some((config, client_id)) = retry {
            if let Err(error) = connect_once(inner.clone(), &config, &client_id).await {
                log::warn!("[RuntimeSupervisor] runtime recovery failed: {error}");
            }
        }
        drop(inner);
    }
}

async fn connect_once(
    inner: Arc<Mutex<SupervisorState>>,
    config: &RuntimeLaunchConfig,
    client_id: &str,
) -> Result<(), RuntimeUnavailable> {
    let pending = {
        let mut state = inner.lock().await;
        if state.connection.is_some() || state.connecting {
            return Err(RuntimeUnavailable::InvalidConfig(
                "runtime supervisor is already connected or connecting".to_string(),
            ));
        }
        if state.stopping {
            return Err(RuntimeUnavailable::NotConnected);
        }
        state.runtime_dir = Some(config.runtime_dir().to_path_buf());
        state.launch_config = Some(config.clone());
        state.client_id = Some(client_id.to_string());
        state.connecting = true;
        state.pending_pid = state.pending_child.as_ref().map(RuntimeChild::id);
        state.pending_child.take()
    };
    let discovery_file = config.discovery_file();
    let attempt = start_runtime(config, &discovery_file, pending).await;
    let (client, child, connection_state, pending) = match attempt {
        Ok(connected) => connected,
        Err((error, pending)) => {
            let mut state = inner.lock().await;
            state.connecting = false;
            state.pending_pid = pending.as_ref().map(RuntimeChild::id);
            state.pending_child = pending;
            state.state = RuntimeConnectionState::Unavailable;
            state.last_error = Some(error.to_string());
            return Err(error);
        }
    };
    let lease = match LeaseGuard::register(&client, client_id, CLIENT_KIND).await {
        Ok(lease) => lease,
        Err(error) => {
            let error = RuntimeUnavailable::from_client_error(error);
            let mut state = inner.lock().await;
            state.connecting = false;
            state.pending_pid = child
                .as_ref()
                .map(RuntimeChild::id)
                .or_else(|| pending.as_ref().map(RuntimeChild::id));
            state.pending_child = child.or(pending);
            state.state = RuntimeConnectionState::Unavailable;
            state.last_error = Some(error.to_string());
            return Err(error);
        }
    };
    let connection = Connection {
        heartbeat: Some(lease.heartbeat()),
        lease,
        child,
        client,
        client_id: client_id.to_string(),
        bridge: None,
        workflow_streams: HashMap::new(),
        #[cfg(feature = "desktop")]
        web_provider: None,
    };
    let mut state = inner.lock().await;
    state.connecting = false;
    state.pending_child = pending;
    state.pending_pid = state.pending_child.as_ref().map(RuntimeChild::id);
    if state.stopping {
        drop(state);
        let _ = cleanup_connection(connection).await;
        return Err(RuntimeUnavailable::NotConnected);
    }
    state.connection = Some(connection);
    state.state = connection_state;
    state.generation = state.generation.saturating_add(1);
    state.last_error = None;
    Ok(())
}

/// Preserve dependency targets and levels before the desktop file filter sees them.
fn forward_runtime_log_line(line: &str) {
    let line = redact_secrets(line);
    if let Some(record) = runtime_log_record(&line) {
        if record.level > log::max_level() {
            return;
        }
        log::logger().log(
            &log::Record::builder()
                .args(format_args!("{}", record.message))
                .target(record.target)
                .level(record.level)
                .file(record.file.or(Some(file!())))
                .line(record.line.or(Some(line!())))
                .build(),
        );
    } else {
        // Unstructured child stderr remains available for native crash diagnosis,
        // but cannot bypass file filtering by masquerading as project info.
        eprintln!("[RuntimeChild] {}", line);
    }
}

#[derive(Debug, PartialEq)]
struct RuntimeLogRecord<'a> {
    level: log::Level,
    target: &'a str,
    message: &'a str,
    file: Option<&'a str>,
    line: Option<u32>,
}

fn runtime_log_record(line: &str) -> Option<RuntimeLogRecord<'_>> {
    if line.starts_with('{') {
        if let Ok(report) = serde_json::from_str::<serde_json::Value>(line) {
            if report.get("event").and_then(serde_json::Value::as_str) == Some("error_diagnostic") {
                return Some(RuntimeLogRecord {
                    level: log::Level::Error,
                    target: "error_diagnostic",
                    message: line,
                    file: None,
                    line: None,
                });
            }
        }
    }
    let (level, record) = line.split_once("] ")?;
    let level = level.strip_prefix('[').and_then(parse_runtime_log_level)?;
    let (header, message) = record.split_once(": ")?;
    // Older runtimes omit source metadata; keep accepting their original header.
    let (target, source) = match header.rsplit_once(" [") {
        Some((target, source)) => (target, Some(source.strip_suffix(']')?)),
        None => (header, None),
    };
    let (file, line) = match source {
        Some(source) => {
            let (file, line) = source.rsplit_once(':')?;
            (Some(file), Some(line.parse().ok()?))
        }
        None => (None, None),
    };
    Some(RuntimeLogRecord {
        level,
        target,
        message,
        file,
        line,
    })
}

fn parse_runtime_log_level(level: &str) -> Option<log::Level> {
    match level {
        "ERROR" => Some(log::Level::Error),
        "WARN" => Some(log::Level::Warn),
        "INFO" => Some(log::Level::Info),
        "DEBUG" => Some(log::Level::Debug),
        "TRACE" => Some(log::Level::Trace),
        _ => None,
    }
}

/// for its readiness handshake.
///
/// Starting can lose a race with another client: a spawn that is refused because
/// discovery already exists, or a child that exits while a different instance
/// wins the runtime-dir lock, is not an error. In both cases this attaches to the
/// winner and reports no owned child, so ownership is never misreported.
async fn start_runtime(
    config: &RuntimeLaunchConfig,
    discovery_file: &Path,
    pending: Option<RuntimeChild>,
) -> Result<
    (
        RuntimeClient,
        Option<RuntimeChild>,
        RuntimeConnectionState,
        Option<RuntimeChild>,
    ),
    (RuntimeUnavailable, Option<RuntimeChild>),
> {
    start_runtime_with_budget(config, discovery_file, pending, SPAWN_READY_TIMEOUT).await
}

async fn start_runtime_with_budget(
    config: &RuntimeLaunchConfig,
    discovery_file: &Path,
    mut pending: Option<RuntimeChild>,
    readiness_budget: Duration,
) -> Result<
    (
        RuntimeClient,
        Option<RuntimeChild>,
        RuntimeConnectionState,
        Option<RuntimeChild>,
    ),
    (RuntimeUnavailable, Option<RuntimeChild>),
> {
    if let Some(child) = pending.as_mut() {
        if matches!(child.try_wait(), Ok(Some(_))) {
            pending = None;
        }
    }
    let mut child = if pending.is_some() {
        match chatspeed_runtime_client::wait_for_discovery(Some(discovery_file), readiness_budget)
            .await
        {
            Ok(document) => {
                let owned = document.pid
                    == pending.as_ref().map(RuntimeChild::id).unwrap_or_default()
                    && matches!(pending.as_mut().and_then(|c| c.try_wait().ok()), Some(None));
                let client = match RuntimeClient::new(&document) {
                    Ok(client) => client,
                    Err(error) => {
                        return Err((RuntimeUnavailable::from_client_error(error), pending))
                    }
                };
                if owned {
                    return Ok((client, pending, RuntimeConnectionState::Spawned, None));
                }
                return Ok((client, None, RuntimeConnectionState::Attached, pending));
            }
            Err(error) => return Err((RuntimeUnavailable::from_client_error(error), pending)),
        }
    } else {
        match runtime_needs_start(config) {
            Ok(false) => {
                return attach_runtime(discovery_file)
                    .await
                    .map(|(client, child, state)| (client, child, state, None))
                    .map_err(|error| (error, None))
            }
            Err(error) => return Err((RuntimeUnavailable::from_client_error(error), None)),
            Ok(true) => {}
        }
        match chatspeed_runtime_client::spawn_runtime_with_config(config) {
            Ok(mut child) => {
                child.forward_stderr_to(forward_runtime_log_line);
                log::info!(
                    "[RuntimeSupervisor] spawned runtime child {}; readiness budget {:?}",
                    child.id(),
                    SPAWN_READY_TIMEOUT
                );
                child
            }
            Err(error) => {
                if !chatspeed_runtime_client::discovery_absent(discovery_file) {
                    return attach_runtime(discovery_file)
                        .await
                        .map(|(client, child, state)| (client, child, state, None))
                        .map_err(|e| (e, None));
                }
                return Err((RuntimeUnavailable::from_client_error(error), None));
            }
        }
    };
    let document = match chatspeed_runtime_client::wait_for_discovery(
        Some(discovery_file),
        readiness_budget,
    )
    .await
    {
        Ok(document) => document,
        Err(error) => {
            log::warn!("[RuntimeSupervisor] readiness failed; child_pid={}, exit_status={:?}, discovery_present={}: {}",
                child.id(), child.try_wait(), discovery_file.exists(), error);
            return Err((RuntimeUnavailable::from_client_error(error), Some(child)));
        }
    };
    let owned = document.pid == child.id() && matches!(child.try_wait(), Ok(None));
    let client = match RuntimeClient::new(&document) {
        Ok(client) => client,
        Err(error) => return Err((RuntimeUnavailable::from_client_error(error), Some(child))),
    };
    if owned {
        child.forward_stderr();
        Ok((client, Some(child), RuntimeConnectionState::Spawned, None))
    } else {
        Ok((client, None, RuntimeConnectionState::Attached, Some(child)))
    }
}

/// Attaches to an already-published runtime after a readiness handshake.
async fn attach_runtime(
    discovery_file: &Path,
) -> Result<(RuntimeClient, Option<RuntimeChild>, RuntimeConnectionState), RuntimeUnavailable> {
    let document =
        chatspeed_runtime_client::wait_for_discovery(Some(discovery_file), ATTACH_READY_TIMEOUT)
            .await
            .map_err(RuntimeUnavailable::from_client_error)?;
    let client = RuntimeClient::new(&document).map_err(RuntimeUnavailable::from_client_error)?;
    Ok((client, None, RuntimeConnectionState::Attached))
}

/// Pulls typed work envelopes and dispatches each to the client dispatcher.
///
/// A stream that ends (`Ok(None)`) means the runtime removed the session, so the
/// reader stops instead of reconnecting: delivery is at-most-once. A transport
/// failure before the stream is established retries a bounded number of times.
async fn run_bridge_reader(
    session: BridgeSession,
    dispatcher: Arc<dyn BridgeDispatcher>,
    mut cancel: watch::Receiver<bool>,
) {
    let mut attempts = 0usize;
    loop {
        if *cancel.borrow() {
            return;
        }
        let mut stream = match session.events().await {
            Ok(stream) => stream,
            Err(error) => {
                attempts += 1;
                if attempts >= BRIDGE_READER_MAX_RETRIES {
                    log::warn!(
                        "[RuntimeSupervisor] bridge reader stopped after {attempts} failed connection attempts"
                    );
                    return;
                }
                log::debug!("[RuntimeSupervisor] bridge reader reconnecting: {error}");
                tokio::select! {
                    _ = tokio::time::sleep(BRIDGE_READER_RETRY) => {}
                    _ = cancel.changed() => return,
                }
                continue;
            }
        };
        loop {
            tokio::select! {
                _ = cancel.changed() => return,
                event = stream.next_event() => match event {
                    Ok(Some(envelope)) => {
                        let dispatcher = dispatcher.clone();
                        let session = session.clone();
                        let cancel = cancel.clone();
                        tokio::spawn(async move {
                            dispatch_bridge_invocation(dispatcher, session, envelope, cancel).await;
                        });
                    }
                    // The session was removed server-side; stop rather than
                    // re-dispatch into a session that no longer exists.
                    Ok(None) => return,
                    Err(error) => {
                        log::debug!("[RuntimeSupervisor] bridge stream error: {error}");
                        return;
                    }
                }
            }
        }
    }
}

/// Executes one invocation and returns its result unless it was cancelled.
///
/// Cancellation (application shutdown or lease release) drops the dispatch
/// future, tells the runtime the request is cancelled, and never submits a
/// result for it.
async fn dispatch_bridge_invocation(
    dispatcher: Arc<dyn BridgeDispatcher>,
    session: BridgeSession,
    envelope: ClientBridgeWorkEnvelope,
    mut cancel: watch::Receiver<bool>,
) {
    let request_id = envelope.invocation.request_id.clone();
    let outcome = tokio::select! {
        _ = cancel.changed() => None,
        result = dispatcher.dispatch(envelope.invocation) => Some(result),
    };
    let Some(result) = outcome else {
        let _ = session.cancel(&request_id).await;
        return;
    };
    if *cancel.borrow() {
        let _ = session.cancel(&request_id).await;
        return;
    }
    if let Err(error) = session.submit_result(&result).await {
        log::debug!("[RuntimeSupervisor] submitting a bridge result failed: {error}");
    }
}

/// Forwards one workflow session's runtime SSE payloads as Tauri events.
///
/// Each envelope's raw `payload` is emitted to `workflow://event/{session_id}`
/// exactly as the frontend expects (it reads `payload.type`), with no
/// `StreamEnvelope` wrapper. A terminal state or a server-closed stream ends the
/// reader; a transport error reconnects from the last `Last-Event-ID` cursor so
/// the runtime replays the gap. A reset or an unparseable frame drops the cursor
/// so the reconnect re-subscribes from scratch instead of replaying a stale
/// window. Errors are logged only and never change the runtime's state.
#[cfg(feature = "desktop")]
async fn run_workflow_event_reader(
    app: tauri::AppHandle,
    client: RuntimeClient,
    session_id: String,
    initial: chatspeed_runtime_client::WorkflowEventStream,
    mut cancel: watch::Receiver<bool>,
) {
    let event_name = workflow_event_name(&session_id);
    let mut stream: Option<chatspeed_runtime_client::WorkflowEventStream> = Some(initial);
    let mut cursor: Option<String> = None;
    let mut failures = 0usize;

    loop {
        if *cancel.borrow() {
            return;
        }
        if stream.is_none() {
            match client.stream_workflow(&session_id, cursor.as_deref()).await {
                Ok(reopened) => stream = Some(reopened),
                Err(error) => {
                    failures += 1;
                    if failures > WORKFLOW_READER_MAX_RETRIES {
                        log::warn!(
                            "[RuntimeSupervisor] workflow reader for session {session_id} stopped after {failures} failed reconnect attempts"
                        );
                        return;
                    }
                    log::debug!(
                        "[RuntimeSupervisor] reconnecting the workflow stream for session {session_id} failed: {error}"
                    );
                    if wait_or_cancel(&mut cancel, WORKFLOW_READER_RETRY).await {
                        return;
                    }
                    continue;
                }
            }
        }
        let Some(mut current) = stream.take() else {
            continue;
        };

        // The inner loop only exits through `break` after a stream error; every
        // other outcome (terminal state, closed stream, cancellation) returns.
        loop {
            tokio::select! {
                _ = cancel.changed() => return,
                event = current.next_event() => match event {
                    Ok(Some(envelope)) => {
                        failures = 0;
                        cursor = Some(chatspeed_runtime_client::stream_envelope_cursor(&envelope));
                        let terminal = is_terminal_workflow_payload(&envelope.payload);
                        let terminal_state = envelope
                            .payload
                            .get("state")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("unknown")
                            .to_string();
                        let wait_reason = envelope
                            .payload
                            .get("wait_reason")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("none")
                            .to_string();
                        if let Err(error) = app.emit(&event_name, envelope.payload) {
                            log::debug!(
                                "[RuntimeSupervisor] emitting a workflow event failed for session {session_id}: {error}"
                            );
                        }
                        if terminal {
                            match terminal_state.as_str() {
                                "error" | "failed" => log::warn!(
                                    "[RuntimeSupervisor] workflow stream for session {session_id} reached terminal state={terminal_state}, wait_reason={wait_reason}"
                                ),
                                _ => log::info!(
                                    "[RuntimeSupervisor] workflow stream for session {session_id} reached terminal state={terminal_state}, wait_reason={wait_reason}"
                                ),
                            }
                            return;
                        }
                    }
                    // The runtime closed the session stream; there is nothing
                    // left to resume, so the reader stops instead of reconnecting.
                    Ok(None) => {
                        log::debug!(
                            "[RuntimeSupervisor] workflow event stream for session {session_id} ended"
                        );
                        return;
                    }
                    Err(error) => {
                        if matches!(error, ClientError::Serialization(_)) {
                            let reason = error.to_string();
                            if let Err(emit_error) = app.emit(
                                &event_name,
                                serde_json::json!({
                                    "type": "reset_required",
                                    "reason": reason,
                                }),
                            ) {
                                log::debug!(
                                    "[RuntimeSupervisor] emitting workflow reset event failed for session {session_id}: {emit_error}"
                                );
                            }
                            // A reset or malformed frame makes the cursor
                            // unusable; re-subscribe from scratch.
                            cursor = None;
                        } else {
                            log::debug!(
                                "[RuntimeSupervisor] workflow stream error for session {session_id}: {error}"
                            );
                        }
                        break;
                    }
                }
            }
        }

        failures += 1;
        if failures > WORKFLOW_READER_MAX_RETRIES {
            log::warn!(
                "[RuntimeSupervisor] workflow reader for session {session_id} stopped after {failures} failed reconnect attempts"
            );
            return;
        }
        if wait_or_cancel(&mut cancel, WORKFLOW_READER_RETRY).await {
            return;
        }
    }
}

/// Waits for `delay`, returning `true` when the reader was cancelled first.
#[cfg(feature = "desktop")]
async fn wait_or_cancel(cancel: &mut watch::Receiver<bool>, delay: Duration) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(delay) => false,
        _ = cancel.changed() => true,
    }
}

/// Returns the canonical Tauri event name for a workflow session's live stream.
///
/// Mirrors the frontend listener contract (`workflow://event/{session_id}`); the
/// batching `tauri::gateway` adapter is not compiled in the desktop build, so
/// this adapter is the single place that names the event.
#[cfg(feature = "desktop")]
fn workflow_event_name(session_id: &str) -> String {
    format!("workflow://event/{session_id}")
}

/// Returns whether a forwarded workflow payload reports a terminal state.
///
/// Only `state` payloads carry a lifecycle status. The reader stops on the four
/// terminal statuses so a finished session is not re-subscribed forever.
#[cfg(feature = "desktop")]
fn is_terminal_workflow_payload(payload: &serde_json::Value) -> bool {
    if payload.get("type").and_then(serde_json::Value::as_str) != Some("state") {
        return false;
    }
    matches!(
        payload.get("state").and_then(serde_json::Value::as_str),
        Some("completed") | Some("failed") | Some("error") | Some("cancelled")
    )
}

/// Validates the client id before it is sent to the runtime.
///
/// The runtime only trims and length-checks an id, so the desktop rejects the
/// values that would be ambiguous in a route or a log line: empty, overlong,
/// whitespace, control characters and the path/query delimiters `/?#\`.
fn validate_client_id(client_id: &str) -> Result<(), RuntimeUnavailable> {
    if client_id.is_empty() {
        return Err(RuntimeUnavailable::InvalidConfig(
            "client id must not be empty".to_string(),
        ));
    }
    if client_id.len() > MAX_CLIENT_ID_LEN {
        return Err(RuntimeUnavailable::InvalidConfig(format!(
            "client id must be at most {MAX_CLIENT_ID_LEN} characters"
        )));
    }
    if client_id
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '/' | '?' | '#' | '\\'))
    {
        return Err(RuntimeUnavailable::InvalidConfig(
            "client id must not contain whitespace, control characters or `/?#\\`".to_string(),
        ));
    }
    Ok(())
}

/// Removes credential-shaped substrings from a message before it is stored in a
/// status document or written to a log line.
///
/// The runtime client never echoes its bearer token and already redacts its own
/// `Debug`, but a control-plane error body is the one free-form string that
/// reaches this module, so it is scrubbed as defense in depth.
fn redact_secrets(message: &str) -> String {
    // `access_token=` contains `token=`, so two markers cover both spellings.
    redact_after(&redact_after(message, "Bearer "), "token=")
}

/// Replaces every `<marker><value>` occurrence with `<marker><redacted>`.
fn redact_after(haystack: &str, marker: &str) -> String {
    let mut redacted = String::with_capacity(haystack.len());
    let mut rest = haystack;
    while let Some(index) = rest.find(marker) {
        redacted.push_str(&rest[..index]);
        redacted.push_str(marker);
        redacted.push_str("<redacted>");
        rest = skip_secret(&rest[index + marker.len()..]);
    }
    redacted.push_str(rest);
    redacted
}

/// Consumes a credential value up to the next delimiter so it is never copied.
fn skip_secret(value: &str) -> &str {
    let end = value
        .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | ';' | '&' | '}'))
        .unwrap_or(value.len());
    &value[end..]
}

#[cfg(test)]
mod tests {
    use super::*;
    use chatspeed_contracts::{ControlPlaneDiscovery, PROTOCOL_VERSION};
    use chatspeed_runtime_client::ClientResponseError;

    #[test]
    fn child_log_records_preserve_targets_levels_and_file_filtering() {
        use log::Log;

        let directory = tempfile::tempdir().expect("temporary log directory");
        let path = directory.path().join("runtime-child.log");
        let (_, logger) = fern::Dispatch::new()
            .level(log::LevelFilter::Debug)
            .filter(|record| {
                record.target().contains("chatspeed")
                    || (record.level() < log::LevelFilter::Info
                        && !matches!(
                            record.target(),
                            "ccproxy_client_logger" | "ccproxy_upstream_logger"
                        ))
            })
            .format(crate::logger::file_log_formatter)
            .chain(fern::log_file(&path).expect("temporary file sink"))
            .into_log();
        for line in [
            "[INFO] rmcp::service: THIRD_PARTY_INFO",
            "[DEBUG] rmcp::service: THIRD_PARTY_DEBUG",
            "[TRACE] tracing::span: THIRD_PARTY_TRACE",
            "[WARN] rmcp::service: THIRD_PARTY_WARN",
            "[ERROR] rmcp::service: THIRD_PARTY_ERROR",
            "[INFO] chatspeed_runtime_backend::owner [crates/backend/src/workflow/react/engine.rs:123]: PROJECT_INFO: still intact",
            "[DEBUG] chatspeed_runtime::startup: PROJECT_DEBUG",
            r#"{"event":"error_diagnostic","error":"STARTUP_FAILURE"}"#,
        ] {
            let record = runtime_log_record(line).expect("structured child log");
            logger.log(
                &log::Record::builder()
                    .args(format_args!("{}", record.message))
                    .target(record.target)
                    .level(record.level)
                    .file(record.file)
                    .line(record.line)
                    .build(),
            );
        }
        logger.flush();
        let output = std::fs::read_to_string(path).expect("read file sink");
        for noise in ["THIRD_PARTY_INFO", "THIRD_PARTY_DEBUG", "THIRD_PARTY_TRACE"] {
            assert!(
                !output.contains(noise),
                "external noise reached file: {noise}"
            );
        }
        for retained in [
            "THIRD_PARTY_WARN",
            "THIRD_PARTY_ERROR",
            "PROJECT_INFO: still intact",
            "PROJECT_DEBUG",
            "STARTUP_FAILURE",
        ] {
            assert!(output.contains(retained), "missing log: {retained}");
        }
        assert_eq!(output.lines().count(), 5);
        assert!(output.contains(
            "[I] crates/backend/src/workflow/react/engine.rs:123 PROJECT_INFO: still intact"
        ));
        assert!(!output.contains("[RuntimeChild]"));
        assert!(!output.contains("chatspeed_runtime_backend::owner"));
        assert!(!output.contains("[INFO]"));
        assert_eq!(
            runtime_log_record("[WARN] rmcp::service: retry"),
            Some(RuntimeLogRecord {
                level: log::Level::Warn,
                target: "rmcp::service",
                message: "retry",
                file: None,
                line: None,
            })
        );
        assert_eq!(
            runtime_log_record(
                r"[ERROR] chatspeed_runtime::owner [C:\workspace\main.rs:42]: failed: detail"
            ),
            Some(RuntimeLogRecord {
                level: log::Level::Error,
                target: "chatspeed_runtime::owner",
                message: "failed: detail",
                file: Some(r"C:\workspace\main.rs"),
                line: Some(42),
            })
        );
        assert!(runtime_log_record("[INFO] chatspeed_runtime [main.rs:bad]: invalid").is_none());
        assert!(runtime_log_record("[CodeGraph] update available").is_none());
        assert!(runtime_log_record("native stderr without metadata").is_none());
        assert!(runtime_log_record("[INVALID] rmcp::service: unknown level").is_none());
    }

    #[test]
    fn child_debug_logs_stay_out_of_info_files_but_remain_in_debug_console() {
        use log::Log;

        let directory = tempfile::tempdir().expect("temporary log directory");
        let file_path = directory.path().join("app.log");
        let console_path = directory.path().join("console.log");
        let (_, logger) = fern::Dispatch::new()
            .level(log::LevelFilter::Debug)
            .chain(
                fern::Dispatch::new()
                    .level(log::LevelFilter::Info)
                    .format(crate::logger::file_log_formatter)
                    .chain(fern::log_file(&file_path).expect("file sink")),
            )
            .chain(
                fern::Dispatch::new()
                    .level(log::LevelFilter::Debug)
                    .format(crate::logger::console_log_formatter)
                    .chain(fern::log_file(&console_path).expect("captured console sink")),
            )
            .into_log();
        for line in [
            "[DEBUG] chatspeed_runtime_backend::workflow::react::engine [crates/backend/src/workflow/react/engine.rs:12]: STEP_DETAIL",
            "[INFO] chatspeed_runtime_backend::workflow::react::engine [crates/backend/src/workflow/react/engine.rs:34]: TASK_COMPLETED",
            "[WARN] chatspeed_runtime_backend::workflow::react::engine [crates/backend/src/workflow/react/engine.rs:56]: RECOVERY_WARNING",
        ] {
            let redacted = redact_secrets(line);
            let record = runtime_log_record(&redacted).expect("child log");
            logger.log(
                &log::Record::builder()
                    .args(format_args!("{}", record.message))
                    .target(record.target)
                    .level(record.level)
                    .file(record.file)
                    .line(record.line)
                    .build(),
            );
        }
        logger.flush();
        let file = std::fs::read_to_string(file_path).expect("read file sink");
        let console = std::fs::read_to_string(console_path).expect("read console sink");
        assert!(!file.contains("STEP_DETAIL"));
        assert!(console.contains("[D] crates/backend/src/workflow/react/engine.rs:12 STEP_DETAIL"));
        for output in [&file, &console] {
            assert!(output
                .contains("[I] crates/backend/src/workflow/react/engine.rs:34 TASK_COMPLETED"));
            assert!(output
                .contains("[W] crates/backend/src/workflow/react/engine.rs:56 RECOVERY_WARNING"));
            assert!(!output.contains("[RuntimeChild]"));
            assert!(!output.contains("chatspeed_runtime_backend::"));
        }
    }

    fn discovery_with_token(token: &str) -> ControlPlaneDiscovery {
        ControlPlaneDiscovery {
            protocol_version: PROTOCOL_VERSION.to_string(),
            server_instance_id: "instance".to_string(),
            pid: 1,
            host: "127.0.0.1".to_string(),
            port: 1,
            token: token.to_string(),
            started_at: "now".to_string(),
        }
    }

    fn server_error(status: u16, code: &str, message: &str) -> RuntimeUnavailable {
        RuntimeUnavailable::from_client_error(ClientError::Server(ClientResponseError {
            legacy_display: String::new(),
            status,
            code: code.to_string(),
            message: message.to_string(),
            presentation: None,
        }))
    }

    #[test]
    fn is_runtime_stopping_uses_the_typed_code_not_the_message_text() {
        assert!(server_error(503, RUNTIME_CODE_RUNTIME_STOPPING, "draining").is_runtime_stopping());

        // Same message, different code: must not classify as stopping.
        assert!(!server_error(503, "unavailable", "draining").is_runtime_stopping());
        // A code that only embeds the marker as a substring or a display-shaped
        // string must not reconstruct the classification from text.
        assert!(!server_error(503, "not_runtime_stopping", "draining").is_runtime_stopping());
        assert!(!RuntimeUnavailable::Unreachable(
            "runtime returned an error: 503 runtime_stopping: draining".to_string()
        )
        .is_runtime_stopping());
        assert!(!RuntimeUnavailable::NotConnected.is_runtime_stopping());
    }

    #[test]
    fn a_typed_server_error_keeps_the_status_code_and_display() {
        let error = server_error(503, RUNTIME_CODE_RUNTIME_STOPPING, "draining");
        match &error {
            RuntimeUnavailable::Server(response) => {
                assert_eq!(response.status, 503);
                assert_eq!(response.code, RUNTIME_CODE_RUNTIME_STOPPING);
                assert_eq!(response.message, "draining");
                assert!(response.presentation.is_none());
            }
            other => panic!("expected a typed server error, got {other:?}"),
        }
        // The Display keeps the historical flat payload for callers that format.
        assert_eq!(
            error.to_string(),
            "runtime returned an error: 503 runtime_stopping: draining"
        );
    }

    #[test]
    fn a_typed_server_error_keeps_the_optional_presentation() {
        let presentation = UserErrorPresentation {
            schema_version: 1,
            message: "The runtime is shutting down; try again".to_string(),
            reason: Some(chatspeed_contracts::ErrorReason::RuntimeStopping),
            upstream_status: None,
        };
        let error =
            RuntimeUnavailable::from_client_error(ClientError::Server(ClientResponseError {
                legacy_display: String::new(),
                status: 503,
                code: RUNTIME_CODE_RUNTIME_STOPPING.to_string(),
                message: "runtime is stopping".to_string(),
                presentation: Some(presentation.clone()),
            }));
        match &error {
            RuntimeUnavailable::Server(response) => {
                assert_eq!(response.presentation.as_ref(), Some(&presentation));
            }
            other => panic!("expected a typed server error, got {other:?}"),
        }
        // The presentation never leaks into the display string.
        assert!(!error.to_string().contains("try again"));
    }

    #[test]
    fn the_ipc_runtime_error_keeps_the_structured_server_payload() {
        let wire = serde_json::from_str::<serde_json::Value>(&runtime_error_wire(
            RuntimeUnavailable::Server(RuntimeServerError {
                status: 503,
                code: RUNTIME_CODE_RUNTIME_STOPPING.to_string(),
                message: "runtime is stopping".to_string(),
                presentation: Some(UserErrorPresentation {
                    schema_version: 1,
                    message: "The runtime is shutting down; try again".to_string(),
                    reason: Some(chatspeed_contracts::ErrorReason::RuntimeStopping),
                    upstream_status: Some(503),
                }),
            }),
        ))
        .expect("structured rejection is a JSON envelope");

        assert_eq!(wire["error"]["code"], RUNTIME_CODE_RUNTIME_STOPPING);
        assert_eq!(wire["error"]["message"], "runtime is stopping");
        assert_eq!(wire["status"], 503);
        assert_eq!(wire["error"]["presentation"]["reason"], "runtime_stopping");
        assert_eq!(wire["error"]["presentation"]["upstream_status"], 503);
    }

    #[test]
    fn the_ipc_runtime_error_keeps_technical_failures_as_plain_text() {
        let wire = runtime_error_wire(RuntimeUnavailable::NotConnected);
        assert_eq!(wire, "runtime client is not connected");
        assert!(!wire.starts_with('{'));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn endpoint_gone_waits_for_a_runtime_that_exits_after_the_listener() {
        let mut child = std::process::Command::new("sleep")
            .arg("0.15")
            .spawn()
            .expect("spawn delayed-exit process");
        let pid = child.id();
        let reaper = std::thread::spawn(move || child.wait().expect("reap delayed-exit process"));
        let supervisor = RuntimeSupervisor::new();
        let started = Instant::now();

        assert!(
            supervisor
                .wait_for_runtime_process_exit(pid, started + Duration::from_secs(1))
                .await
        );
        assert!(started.elapsed() >= Duration::from_millis(50));
        reaper.join().expect("join delayed-exit reaper");
    }

    #[test]
    fn runtime_client_and_error_debug_do_not_leak_secrets() {
        // The encapsulated client redacts its bearer token.
        let client = RuntimeClient::new(&discovery_with_token("test-token")).expect("client");
        let debug = format!("{client:?}");
        assert!(!debug.contains("test-token"), "{debug}");
        assert!(debug.contains("<redacted>"), "{debug}");

        // A credential-shaped control-plane message is scrubbed before it can be
        // stored in a status document or logged.
        let error = RuntimeUnavailable::from_client_error(ClientError::Auth(
            chatspeed_runtime_client::AuthError {
                status: 401,
                code: "unauthorized".to_string(),
                message: "Bearer test-token rejected".to_string(),
                presentation: None,
            },
        ));
        let rendered = format!("{error:?} {error}");
        assert!(!rendered.contains("test-token"), "{rendered}");
        assert!(rendered.contains("<redacted>"), "{rendered}");
    }

    #[test]
    fn client_id_validation_accepts_normal_ids_and_rejects_unsafe_ones() {
        assert!(validate_client_id("tauri-main").is_ok());
        assert!(validate_client_id("cscli-0f9a1b").is_ok());
        assert!(validate_client_id(&"a".repeat(MAX_CLIENT_ID_LEN)).is_ok());

        for bad in ["", "has space", "a/b", "a?b", "a#b", "a\\b", "line\nbreak"] {
            assert!(
                matches!(
                    validate_client_id(bad),
                    Err(RuntimeUnavailable::InvalidConfig(_))
                ),
                "{bad:?} should be rejected"
            );
        }

        assert!(matches!(
            validate_client_id(&"a".repeat(MAX_CLIENT_ID_LEN + 1)),
            Err(RuntimeUnavailable::InvalidConfig(_))
        ));
    }

    #[tokio::test]
    async fn a_fresh_supervisor_reports_disconnected_without_secrets() {
        let supervisor = RuntimeSupervisor::new();
        let status = supervisor.status().await;
        assert_eq!(status.state, RuntimeConnectionState::Disconnected);
        assert!(status.client_id.is_none());
        assert!(status.base_url.is_none());
        assert!(status.lease_expires_at.is_none());
        assert!(status.last_error.is_none());

        // No lease is held yet, so the control-plane client is unavailable.
        assert!(matches!(
            supervisor.client().await,
            Err(RuntimeUnavailable::NotConnected)
        ));

        // The Debug surface never carries a client or lease secret.
        let debug = format!("{supervisor:?}");
        assert!(!debug.contains("<redacted>"), "{debug}");
    }

    #[tokio::test]
    async fn start_runtime_attaches_when_a_spawn_is_refused_by_an_existing_document() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = chatspeed_runtime_client::sandbox_launch_config(dir.path());
        let discovery_file = config.discovery_file();
        // A protocol-incompatible document makes the spawn refuse *and* lets the
        // attach path fail fast, without a network round trip.
        let document = ControlPlaneDiscovery {
            protocol_version: "2".to_string(),
            server_instance_id: "other".to_string(),
            pid: std::process::id(),
            host: "127.0.0.1".to_string(),
            port: 1,
            token: "token".to_string(),
            started_at: "now".to_string(),
        };
        std::fs::write(
            &discovery_file,
            serde_json::to_vec(&document).expect("encode"),
        )
        .expect("write");

        let result = start_runtime(&config, &discovery_file, None).await;
        // The spawn refusal is not surfaced: the existing document was attached
        // to and its protocol mismatch reported instead.
        assert!(
            matches!(result, Err((RuntimeUnavailable::Protocol(_), _))),
            "{result:?}"
        );
    }

    async fn recovery_test_server() -> (ControlPlaneDiscovery, tokio::task::JoinHandle<()>) {
        use axum::{
            routing::{get, post},
            Json, Router,
        };
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind fake runtime");
        let mut document = discovery_with_token("test-only-token");
        document.pid = std::process::id();
        document.port = listener.local_addr().expect("address").port();
        let meta = serde_json::json!({
            "service": "chatspeed-runtime", "protocol_version": PROTOCOL_VERSION,
            "schema_version": chatspeed_contracts::SCHEMA_VERSION,
            "server_instance_id": document.server_instance_id, "pid": document.pid
        });
        let app = Router::new()
            .route("/control/v1/meta", get(move || { let meta = meta.clone(); async move { Json(meta) } }))
            .route("/control/v1/clients/register", post(|| async { Json(serde_json::json!({
                "client_id": "tauri-main", "lease_id": "test-lease", "expires_at": "2099-01-01T00:00:00Z"
            })) }))
            .route("/control/v1/clients/tauri-main/release", post(|| async { Json(serde_json::json!({})) }));
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve fake runtime");
        });
        (document, task)
    }

    #[tokio::test]
    async fn initial_failure_recovers_after_discovery_is_repaired_without_restarting_desktop() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = chatspeed_runtime_client::sandbox_launch_config(dir.path());
        let (document, server) = recovery_test_server().await;
        let mut incompatible = document.clone();
        incompatible.protocol_version = "2".to_string();
        std::fs::write(
            config.discovery_file(),
            serde_json::to_vec(&incompatible).expect("encode"),
        )
        .expect("write");
        let supervisor = RuntimeSupervisor::new();
        assert!(supervisor
            .connect_or_start(&config, "tauri-main")
            .await
            .is_err());
        assert_eq!(supervisor.status().await.generation, 0);
        std::fs::write(
            config.discovery_file(),
            serde_json::to_vec(&document).expect("encode"),
        )
        .expect("repair discovery");
        tokio::time::timeout(Duration::from_secs(6), async {
            loop {
                if supervisor.status().await.generation == 1 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("monitor reconnects after initial failure");
        assert_eq!(
            supervisor.status().await.state,
            RuntimeConnectionState::Attached
        );
        supervisor.release().await.expect("release lease");
        server.abort();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn slow_pending_child_survives_repeated_readiness_budgets_without_a_second_spawn() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bin = dir.path().join("fake-runtime.sh");
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(&bin, "#!/bin/sh\nprintf 'spawn\\n' >> \"$CHATSPEED_RUNTIME_DIR/spawns\"\nwhile [ ! -e \"$CHATSPEED_RUNTIME_DIR/exit\" ]; do sleep 0.01; done\n").expect("write child");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700))
            .expect("permissions");
        // Only the synchronous spawn reads this override; no await occurs while
        // it is installed, and this module has no other environment-mutating tests.
        let original = std::env::var_os(chatspeed_runtime_client::RUNTIME_BINARY_ENV);
        std::env::set_var(chatspeed_runtime_client::RUNTIME_BINARY_ENV, &bin);
        let config = chatspeed_runtime_client::sandbox_launch_config(dir.path());
        let child =
            chatspeed_runtime_client::spawn_runtime_with_config(&config).expect("spawn fake child");
        if let Some(original) = original {
            std::env::set_var(chatspeed_runtime_client::RUNTIME_BINARY_ENV, original);
        } else {
            std::env::remove_var(chatspeed_runtime_client::RUNTIME_BINARY_ENV);
        }
        let pid = child.id();
        let mut pending = Some(child);
        for _ in 0..2 {
            let result = start_runtime_with_budget(
                &config,
                &config.discovery_file(),
                pending.take(),
                Duration::from_millis(40),
            )
            .await;
            let (error, retained) = result.expect_err("discovery remains absent");
            assert!(matches!(error, RuntimeUnavailable::Discovery(_)));
            pending = retained;
            assert_eq!(pending.as_ref().map(RuntimeChild::id), Some(pid));
        }
        assert_eq!(
            std::fs::read_to_string(dir.path().join("spawns")).expect("spawn counter"),
            "spawn\n"
        );
        std::fs::write(dir.path().join("exit"), b"exit").expect("stop fake child");
        drop(pending);
    }

    #[tokio::test]
    async fn heartbeat_failure_reconnects_to_a_new_generation_and_release_keeps_it_stopped() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = chatspeed_runtime_client::sandbox_launch_config(dir.path());
        let (document, server) = recovery_test_server().await;
        std::fs::write(
            config.discovery_file(),
            serde_json::to_vec(&document).expect("encode"),
        )
        .expect("write");
        let supervisor = RuntimeSupervisor::new();
        supervisor
            .connect_or_start(&config, "tauri-main")
            .await
            .expect("connect");
        let old_heartbeat = {
            let mut state = supervisor.inner.lock().await;
            let connection = state.connection.as_mut().expect("connection");
            connection.heartbeat.replace(
                connection
                    .lease
                    .heartbeat_with_interval(Duration::from_millis(20)),
            )
        };
        if let Some(heartbeat) = old_heartbeat {
            heartbeat.stop().await;
        }
        server.abort();
        let (successor, next_server) = recovery_test_server().await;
        std::fs::write(
            config.discovery_file(),
            serde_json::to_vec(&successor).expect("encode"),
        )
        .expect("write successor");
        tokio::time::timeout(Duration::from_secs(7), async {
            while supervisor.status().await.generation < 2 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("failed heartbeat triggers a new lease");
        assert!(
            process_alive(std::process::id()),
            "attached runtime must never be killed"
        );
        supervisor.release().await.expect("release");
        tokio::time::sleep(RUNTIME_HEALTH_CHECK_INTERVAL + Duration::from_millis(50)).await;
        let status = supervisor.status().await;
        assert_eq!(status.generation, 2);
        assert_eq!(status.state, RuntimeConnectionState::Released);
        next_server.abort();
    }

    #[tokio::test]
    async fn health_probe_recovers_a_dead_endpoint_before_the_next_lease_heartbeat() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = chatspeed_runtime_client::sandbox_launch_config(dir.path());
        let (document, server) = recovery_test_server().await;
        std::fs::write(
            config.discovery_file(),
            serde_json::to_vec(&document).expect("encode"),
        )
        .expect("write");
        let supervisor = RuntimeSupervisor::new();
        supervisor
            .connect_or_start(&config, "tauri-main")
            .await
            .expect("connect");
        let old_heartbeat = {
            let mut state = supervisor.inner.lock().await;
            let connection = state.connection.as_mut().expect("connection");
            connection.heartbeat.replace(
                connection
                    .lease
                    .heartbeat_with_interval(Duration::from_secs(60)),
            )
        };
        if let Some(heartbeat) = old_heartbeat {
            heartbeat.stop().await;
        }
        server.abort();
        let _ = server.await;
        let (successor, next_server) = recovery_test_server().await;
        std::fs::write(
            config.discovery_file(),
            serde_json::to_vec(&successor).expect("encode"),
        )
        .expect("write successor");
        tokio::time::timeout(Duration::from_secs(7), async {
            while supervisor.status().await.generation < 2 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("health probe recovers without waiting for lease renewal");
        let status = supervisor.status().await;
        assert_eq!(
            status.base_url,
            Some(format!("http://127.0.0.1:{}", successor.port))
        );
        assert_eq!(status.heartbeat_ok, Some(true));
        assert!(
            process_alive(std::process::id()),
            "attached runtime must never be killed"
        );
        supervisor.release().await.expect("release");
        next_server.abort();
    }

    #[tokio::test]
    async fn hanging_health_probe_does_not_block_status_or_desktop_exit() {
        use axum::{routing::get, Router};
        let dir = tempfile::tempdir().expect("tempdir");
        let config = chatspeed_runtime_client::sandbox_launch_config(dir.path());
        let (document, server) = recovery_test_server().await;
        std::fs::write(
            config.discovery_file(),
            serde_json::to_vec(&document).expect("encode"),
        )
        .expect("write");
        let supervisor = RuntimeSupervisor::new();
        supervisor
            .connect_or_start(&config, "tauri-main")
            .await
            .expect("connect");
        server.abort();
        let _ = server.await;
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", document.port))
            .await
            .expect("rebind endpoint");
        let entered = Arc::new(tokio::sync::Notify::new());
        let signal = entered.clone();
        let app = Router::new().route(
            chatspeed_runtime_client::META_PATH,
            get(move || {
                let signal = signal.clone();
                async move {
                    signal.notify_one();
                    std::future::pending::<String>().await
                }
            }),
        );
        let hung_server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve hung endpoint");
        });
        tokio::time::timeout(Duration::from_secs(4), entered.notified())
            .await
            .expect("probe started");
        tokio::time::timeout(Duration::from_millis(100), supervisor.status())
            .await
            .expect("status remains responsive");
        tokio::time::timeout(Duration::from_millis(100), supervisor.begin_stopping())
            .await
            .expect("exit fences recovery without waiting for I/O");
        tokio::time::sleep(RUNTIME_HEALTH_PROBE_TIMEOUT + Duration::from_millis(50)).await;
        assert_eq!(
            supervisor.status().await.generation,
            1,
            "cancelled probes cannot reconnect"
        );
        let _ = supervisor.release().await;
        hung_server.abort();
    }

    #[tokio::test]
    async fn health_probe_timeout_marks_a_hung_endpoint_unavailable() {
        use axum::{routing::get, Router};
        let dir = tempfile::tempdir().expect("tempdir");
        let config = chatspeed_runtime_client::sandbox_launch_config(dir.path());
        let (document, server) = recovery_test_server().await;
        std::fs::write(
            config.discovery_file(),
            serde_json::to_vec(&document).expect("encode"),
        )
        .expect("write");
        let supervisor = RuntimeSupervisor::new();
        supervisor
            .connect_or_start(&config, "tauri-main")
            .await
            .expect("connect");
        server.abort();
        let _ = server.await;
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", document.port))
            .await
            .expect("rebind endpoint");
        let app = Router::new().route(
            chatspeed_runtime_client::META_PATH,
            get(|| async { std::future::pending::<String>().await }),
        );
        let hung_server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve hung endpoint");
        });
        tokio::time::timeout(Duration::from_secs(7), async {
            while supervisor.status().await.state != RuntimeConnectionState::Unavailable {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("probe timeout invalidates stale ready state");
        let status = supervisor.status().await;
        assert_eq!(
            status.last_error.as_deref(),
            Some("control-plane health probe timed out")
        );
        assert_eq!(status.generation, 1);
        assert_eq!(
            status.base_url, None,
            "failed endpoint is no longer available to commands"
        );
        supervisor.begin_stopping().await;
        hung_server.abort();
        let _ = supervisor.release().await;
    }

    #[tokio::test]
    async fn release_and_forced_stop_fence_future_recovery() {
        let supervisor = RuntimeSupervisor::new();
        supervisor.begin_stopping().await;
        assert!(supervisor.is_stopping().await);
        let config = chatspeed_runtime_client::sandbox_launch_config(
            tempfile::tempdir().expect("tempdir").path(),
        );
        assert!(matches!(
            supervisor.connect_or_start(&config, "tauri-main").await,
            Err(RuntimeUnavailable::NotConnected)
        ));
        supervisor.release().await.expect("release is idempotent");
        assert_eq!(
            supervisor.status().await.state,
            RuntimeConnectionState::Released
        );
    }
    #[cfg(feature = "desktop")]
    #[test]
    fn workflow_event_name_matches_the_frontend_listener_contract() {
        assert_eq!(
            workflow_event_name("session-1"),
            "workflow://event/session-1"
        );
    }

    #[cfg(feature = "desktop")]
    #[test]
    fn only_the_four_terminal_states_stop_the_workflow_reader() {
        for terminal in ["completed", "failed", "error", "cancelled"] {
            let payload = serde_json::json!({ "type": "state", "state": terminal });
            assert!(is_terminal_workflow_payload(&payload), "{terminal}");
        }
        for active in ["running", "awaiting_user", "awaiting_approval"] {
            let payload = serde_json::json!({ "type": "state", "state": active });
            assert!(!is_terminal_workflow_payload(&payload), "{active}");
        }
        // Only `state` payloads carry a lifecycle status.
        assert!(!is_terminal_workflow_payload(
            &serde_json::json!({ "type": "chunk", "content": "hi" })
        ));
        assert!(!is_terminal_workflow_payload(
            &serde_json::json!({ "type": "task_completed" })
        ));
    }
}
