//! Desktop-side supervisor for the standalone `chatspeed-runtime`.
//!
//! The runtime process is the single owner of the canonical database, the
//! chat/tool state, the workflow engine and the loopback HTTP/SSE control plane.
//! The desktop application talks to it as one more control-plane client: it
//! attaches to an already-published runtime when one is live, starts one only
//! when the runtime directory genuinely has no discovery document, completes the
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

use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use chatspeed_contracts::{
    ClientBridgeCapability, ClientBridgeDeclaration, ClientBridgeWorkEnvelope,
    ClientCapabilityInvocation, ClientCapabilityResult, BRIDGE_PROTOCOL_VERSION,
    BRIDGE_SCHEMA_VERSION,
};
use chatspeed_runtime_client::{
    BridgeSession, ClientError, Heartbeat, LeaseGuard, RuntimeChild, RuntimeClient,
    RuntimeLaunchConfig,
};
use serde::Serialize;
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
    /// The runtime returned a structured error.
    #[error("runtime returned an error: {0}")]
    Server(String),
    /// The caller asked for something the supervisor refuses to do.
    #[error("invalid runtime client configuration: {0}")]
    InvalidConfig(String),
    /// No lease is held, so the control-plane client is not available.
    #[error("runtime client is not connected")]
    NotConnected,
}

impl RuntimeUnavailable {
    /// Classifies a client error and scrubs it before it is stored or logged.
    pub(crate) fn from_client_error(error: ClientError) -> Self {
        match error {
            ClientError::Discovery(message) => Self::Discovery(redact_secrets(&message)),
            ClientError::Protocol(message) => Self::Protocol(redact_secrets(&message)),
            ClientError::Transport(message) => Self::Unreachable(redact_secrets(&message)),
            ClientError::Auth(message) => Self::Rejected(redact_secrets(&message)),
            ClientError::Server {
                status,
                code,
                message,
            } => Self::Server(format!("{status} {code}: {}", redact_secrets(&message))),
            ClientError::InvalidRequest(message) => Self::InvalidConfig(message),
            ClientError::Serialization(message) => Self::Protocol(redact_secrets(&message)),
        }
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
}

// ---------------------------------------------------------------------------
// Supervisor
// ---------------------------------------------------------------------------

/// Thread-safe supervisor around the desktop's relationship with the runtime.
///
/// It owns the [`RuntimeClient`], the [`LeaseGuard`] and its [`Heartbeat`], and
/// (only when this process started the runtime) the [`RuntimeChild`] handle. It
/// never owns runtime state. Tauri manages one `Arc<RuntimeSupervisor>`, and
/// every accessor is safe to call from any command.
pub struct RuntimeSupervisor {
    inner: Arc<Mutex<SupervisorState>>,
}

struct SupervisorState {
    connection: Option<Connection>,
    state: RuntimeConnectionState,
    runtime_dir: Option<PathBuf>,
    last_error: Option<String>,
}

/// Everything the supervisor owns once a lease is held.
struct Connection {
    client: RuntimeClient,
    lease: LeaseGuard,
    heartbeat: Option<Heartbeat>,
    child: Option<RuntimeChild>,
    client_id: String,
    bridge: Option<BridgeHandle>,
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

impl SupervisorState {
    /// Builds the redacted status snapshot. Never reads a secret.
    fn snapshot(&self) -> RuntimeStatus {
        let connection = self.connection.as_ref();
        RuntimeStatus {
            state: self.state,
            client_id: connection.map(|c| c.client_id.clone()),
            base_url: connection.map(|c| c.client.base_url().to_string()),
            runtime_dir: self.runtime_dir.clone(),
            owns_runtime_child: connection.is_some_and(|c| c.child.is_some()),
            runtime_child_pid: connection
                .and_then(|c| c.child.as_ref())
                .map(RuntimeChild::id),
            heartbeat_ok: connection
                .and_then(|c| c.heartbeat.as_ref())
                .map(|heartbeat| heartbeat.status().is_ok()),
            bridge_active: connection.is_some_and(|c| c.bridge.is_some()),
            lease_expires_at: connection.map(|c| c.lease.expires_at().to_string()),
            last_error: self.last_error.clone(),
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
            })),
        }
    }

    /// Attaches to a live runtime or starts one, then holds a client lease.
    ///
    /// The launch configuration is the runtime's identity: a published discovery
    /// document in its runtime directory is attached to only after the readiness
    /// handshake proves it is the expected standalone runtime, and a runtime is
    /// started only when that document is genuinely absent. A started runtime
    /// receives every path explicitly, so an inherited environment or a
    /// cross-profile binary can never redirect it.
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

        let mut state = self.inner.lock().await;
        if state.connection.is_some() {
            return Err(RuntimeUnavailable::InvalidConfig(
                "runtime supervisor is already connected".to_string(),
            ));
        }
        state.runtime_dir = Some(config.runtime_dir().to_path_buf());

        let discovery_file = config.discovery_file();
        let attempt = if chatspeed_runtime_client::discovery_absent(&discovery_file) {
            start_runtime(config, &discovery_file).await
        } else {
            attach_runtime(&discovery_file).await
        };

        let (client, child, connection_state) = match attempt {
            Ok(connected) => connected,
            Err(error) => {
                state.state = RuntimeConnectionState::Unavailable;
                state.last_error = Some(error.to_string());
                return Err(error);
            }
        };

        let lease = match LeaseGuard::register(&client, client_id, CLIENT_KIND).await {
            Ok(lease) => lease,
            Err(error) => {
                let error = RuntimeUnavailable::from_client_error(error);
                state.state = RuntimeConnectionState::Unavailable;
                state.last_error = Some(error.to_string());
                return Err(error);
            }
        };

        // The heartbeat keeps the lease alive for as long as the desktop runs.
        let heartbeat = lease.heartbeat();
        state.connection = Some(Connection {
            heartbeat: Some(heartbeat),
            lease,
            child,
            client,
            client_id: client_id.to_string(),
            bridge: None,
            #[cfg(feature = "desktop")]
            web_provider: None,
        });
        state.state = connection_state;
        state.last_error = None;
        Ok(())
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

    /// Redacted snapshot for a status command or a log line.
    pub async fn status(&self) -> RuntimeStatus {
        self.inner.lock().await.snapshot()
    }

    /// Opens the client WebView capability bridge and starts its reader.
    ///
    /// The session is registered with the supervisor's own client id and the
    /// opaque lease id, so the runtime resolves a live `tauri` lease itself. The
    /// reader pulls typed work envelopes and hands each to `dispatcher`.
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
        if state.state == RuntimeConnectionState::Released {
            return Err(RuntimeUnavailable::NotConnected);
        }
        let Some(connection) = state.connection.as_mut() else {
            return Err(RuntimeUnavailable::NotConnected);
        };
        if connection.web_provider.is_some() {
            return Err(RuntimeUnavailable::InvalidConfig(
                "the desktop Web MCP provider is already connected".to_string(),
            ));
        }
        connection.web_provider = Some(handle);
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
        let mut state = self.inner.lock().await;
        release_connection(&mut state).await
    }

    /// Releases the lease when the application exits.
    ///
    /// Identical to [`RuntimeSupervisor::release`] but logs instead of returning,
    /// so the Tauri exit handler can call it without handling a result. It never
    /// kills a runtime this process did not start.
    pub async fn shutdown(&self) {
        let mut state = self.inner.lock().await;
        if let Err(error) = release_connection(&mut state).await {
            log::warn!("[RuntimeSupervisor] releasing the runtime lease failed: {error}");
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
        // `Drop` cannot await. When the last handle is dropped and the async
        // runtime is still alive, hand the lease back on it; otherwise the lease
        // simply expires on its TTL and the runtime's idle grace ends it. A
        // healthy runtime is never killed here.
        let Some(state) = Arc::get_mut(&mut self.inner).map(Mutex::get_mut) else {
            return;
        };
        if state.state == RuntimeConnectionState::Released {
            return;
        }
        let Some(mut connection) = state.connection.take() else {
            return;
        };
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        handle.spawn(async move {
            if let Some(heartbeat) = connection.heartbeat.take() {
                heartbeat.stop().await;
            }
            // Cancel the reader and unregister the bridge before releasing the
            // lease, mirroring `release_connection`.
            if let Some(bridge) = connection.bridge.take() {
                bridge.cancel.send_replace(true);
                bridge.reader.abort();
                let _ = bridge.session.unregister().await;
            }
            #[cfg(feature = "desktop")]
            if let Some(provider) = connection.web_provider.take() {
                provider.shutdown().await;
            }
            let _ = connection.lease.release().await;
            // `connection` drops here: its `RuntimeChild` only reaps an exited
            // runtime and never kills a healthy one.
        });
    }
}

/// Stops the heartbeat and releases the lease held by `state`, if any.
///
/// Order matters: the bridge reader and any in-flight invocation are cancelled
/// first, then the bridge session is unregistered, and only then is the client
/// lease released. Releasing the lease while the bridge still holds work would
/// leave the runtime dispatching into a session the desktop has abandoned.
async fn release_connection(state: &mut SupervisorState) -> Result<(), RuntimeUnavailable> {
    let Some(connection) = state.connection.as_mut() else {
        return Ok(());
    };
    // Stop and join the heartbeat first: a renewal must never be in flight while
    // the lease is released.
    if let Some(heartbeat) = connection.heartbeat.take() {
        heartbeat.stop().await;
    }
    if let Some(bridge) = connection.bridge.take() {
        bridge.cancel.send_replace(true);
        bridge.reader.abort();
        // Bound the join so a stuck reader cannot block shutdown; the abort
        // above already guarantees it observes cancellation.
        let _ = tokio::time::timeout(BRIDGE_READER_STOP_TIMEOUT, bridge.reader).await;
        if let Err(error) = bridge.session.unregister().await {
            log::debug!("[RuntimeSupervisor] unregistering the bridge failed: {error}");
        }
    }
    // Stop the desktop Web MCP provider before the lease is released, so the
    // runtime never keeps dialing a provider whose lease is already gone.
    #[cfg(feature = "desktop")]
    if let Some(provider) = connection.web_provider.take() {
        provider.shutdown().await;
    }
    connection
        .lease
        .release()
        .await
        .map_err(RuntimeUnavailable::from_client_error)?;
    state.state = RuntimeConnectionState::Released;
    Ok(())
}

/// Starts the runtime child with a fully resolved launch configuration and waits
/// for its readiness handshake.
async fn start_runtime(
    config: &RuntimeLaunchConfig,
    discovery_file: &Path,
) -> Result<(RuntimeClient, Option<RuntimeChild>, RuntimeConnectionState), RuntimeUnavailable> {
    let mut child = chatspeed_runtime_client::spawn_runtime_with_config(config)
        .map_err(RuntimeUnavailable::from_client_error)?;
    let document = match chatspeed_runtime_client::wait_for_discovery(
        Some(discovery_file),
        SPAWN_READY_TIMEOUT,
    )
    .await
    {
        Ok(document) => document,
        Err(error) => {
            // Surface a failed child's stderr so a startup failure is
            // diagnosable instead of a bare timeout.
            if let Some(detail) = child.diagnostics() {
                log::warn!(
                    "[RuntimeSupervisor] runtime child failed to become ready: {}",
                    redact_secrets(&detail)
                );
            }
            return Err(RuntimeUnavailable::from_client_error(error));
        }
    };
    // Readiness is proven; the child is long-lived from here.
    child.silence_stderr();
    let client = RuntimeClient::new(&document).map_err(RuntimeUnavailable::from_client_error)?;
    Ok((client, Some(child), RuntimeConnectionState::Spawned))
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
            "Bearer test-token rejected".to_string(),
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
}
