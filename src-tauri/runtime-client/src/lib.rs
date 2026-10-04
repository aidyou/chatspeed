//! Reusable, Tauri-free control-plane runtime client.
//!
//! `chatspeed` (Tauri), `cscli` and future clients all speak the same loopback
//! HTTP/JSON + SSE `/control/v1` protocol. Keeping the discovery lookup, bearer
//! transport, stable error mapping and lease lifecycle here means clients reuse
//! one implementation instead of drifting apart.
//!
//! Scope: the client reads discovery, optionally starts an explicitly configured
//! runtime child, and talks HTTP. It never opens the database, owns an executor,
//! or implements runtime state. The production dependency graph deliberately
//! excludes the desktop crate, `tauri`, `wry`, `gtk` and any Tauri plugin.

use chatspeed_contracts::{
    ChatStartRequest, ChatStartResponse, ChatStopRequest, ChatStopResponse, ChatStreamEnvelope,
    ClientBridgeCancelRequest, ClientBridgeDeclaration, ClientBridgeRegistration,
    ClientBridgeRegistrationResponse, ClientBridgeUnregisterRequest, ClientBridgeWorkEnvelope,
    ClientCapabilityResult, ClientLease, ClientLeaseRequest, ClientLeaseResponse,
    ControlPlaneDiscovery, ErrorEnvelope, ListModelsRequest, MetaResponse, ModelDetailsDto,
    ModelsDevPresetProviderDto, ModelsDevProviderModelsRequest, ResolveModelProfileRequest,
    StreamEnvelope, TerminalCloseRequest, TerminalCreateRequest, TerminalListSessionsRequest,
    TerminalListShellsRequest, TerminalResizeRequest, TerminalSessionMetadataDto, TerminalShellDto,
    TerminalStreamEnvelope, TerminalWriteRequest, WebMcpProviderRegistration,
    WebMcpProviderRegistrationResponse, WebMcpProviderStatus, DISCOVERY_FILE_NAME, PROTOCOL_MAJOR,
    SCHEMA_VERSION, WEB_MCP_CLIENT_HEADER, WEB_MCP_INSTANCE_HEADER, WEB_MCP_LEASE_HEADER,
    WEB_MCP_PROOF_HEADER, WEB_MCP_REGISTER_PATH, WEB_MCP_STATUS_PATH, WEB_MCP_UNREGISTER_PATH,
};
use reqwest::redirect::Policy;
use reqwest::{header, Method, Response, StatusCode};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use std::fmt;
use std::io::Read as _;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::Duration;

pub mod paths;

pub use paths::{
    build_profile, dev_app_data_dir, production_app_data_dir, resolve_launch_config,
    sandbox_launch_config, BuildProfile, LaunchConfigError, RuntimeLaunchConfig, DB_FILE_NAME,
    LEGACY_HOME_ENV, RUNTIME_APP_DATA_DIR_ENV, RUNTIME_DB_ENV, RUNTIME_DIR_ENV,
};

/// Serializes tests that mutate the process environment so path resolution and
/// respawn tests never observe another test's overrides. Shared by every test
/// module in this crate.
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Protocol major version this client understands.
pub const SUPPORTED_PROTOCOL_MAJOR: u32 = PROTOCOL_MAJOR;

/// A runtime child process started by a client.
///
/// The child is never killed from `Drop`; lifecycle remains owned by the lease
/// and the runtime's own graceful idle shutdown. Dropping the handle does reap a
/// child that has already exited, so a crashed runtime cannot linger as a
/// zombie while the client is still running.
pub struct RuntimeChild {
    child: Child,
    exit: Option<ExitStatus>,
}

impl RuntimeChild {
    /// Returns the child process id for diagnostics without exposing secrets.
    pub fn id(&self) -> u32 {
        self.child.id()
    }

    /// Reaps the child if it has already exited.
    ///
    /// The exit status is remembered, so `diagnostics` can still read a captured
    /// stderr after a caller has already polled for exit.
    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>, ClientError> {
        if let Some(status) = self.exit {
            return Ok(Some(status));
        }
        let status = self.child.try_wait().map_err(|error| {
            ClientError::Transport(format!("cannot poll runtime child: {error}"))
        })?;
        if let Some(status) = status {
            self.exit = Some(status);
        }
        Ok(status)
    }

    /// Best-effort diagnostics captured from the child's stderr.
    ///
    /// Only reads once the child has exited: reading a live child's stderr pipe
    /// would block. Returns `None` when nothing useful was captured, so a caller
    /// can append the details conditionally.
    pub fn diagnostics(&mut self) -> Option<String> {
        if !matches!(self.try_wait(), Ok(Some(_))) {
            return None;
        }
        let mut stderr = self.child.stderr.take()?;
        let mut buffer = String::new();
        stderr.read_to_string(&mut buffer).ok()?;
        let trimmed = buffer.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    }

    /// Detaches a reader that discards the child's remaining stderr.
    ///
    /// Called after the ready handshake: diagnostics only matter before
    /// readiness, and leaving the pipe unread could eventually block a
    /// long-lived runtime on a full pipe.
    pub fn silence_stderr(&mut self) {
        let Some(mut stderr) = self.child.stderr.take() else {
            return;
        };
        std::thread::spawn(move || {
            let mut sink = std::io::sink();
            let _ = std::io::copy(&mut stderr, &mut sink);
        });
    }
}

impl fmt::Debug for RuntimeChild {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeChild")
            .field("pid", &self.child.id())
            .finish()
    }
}

impl Drop for RuntimeChild {
    fn drop(&mut self) {
        // Reap an exited child so it cannot stay a zombie; never kill a healthy
        // runtime, whose lifecycle belongs to its leases.
        let _ = self.child.try_wait();
    }
}

/// Starts the runtime binary with `runtime_dir` as its owned runtime directory.
///
/// This is the explicit-sandbox entry point: the database and the
/// application-data directory are derived inside `runtime_dir`. Callers that
/// have resolved a full [`RuntimeLaunchConfig`] (the desktop and `cscli`
/// defaults) use [`spawn_runtime_with_config`] instead, so a cross-profile spawn
/// passes every path explicitly instead of relying on inherited environment.
///
/// The binary is resolved explicitly: `CHATSPEED_RUNTIME_BIN` wins, otherwise
/// the `chatspeed-runtime` binary next to this executable is used. It refuses to
/// start when a discovery document already exists in `runtime_dir`, so a client
/// never races a live (or freshly published) runtime and starts a second one.
pub fn spawn_runtime(runtime_dir: &Path) -> Result<RuntimeChild, ClientError> {
    spawn_runtime_with_config(&paths::sandbox_launch_config(runtime_dir))
}

/// Starts the runtime binary with a fully resolved launch configuration.
///
/// All three paths are passed explicitly (`CHATSPEED_RUNTIME_DIR`,
/// `CHATSPEED_RUNTIME_DB`, `CHATSPEED_RUNTIME_APP_DATA_DIR`) and the legacy
/// `CHATSPEED_HOME` is removed, so an inherited environment can never redirect
/// the child to a different runtime directory, database or profile. A release
/// runtime binary started by a debug client therefore still serves the
/// development paths the client resolved.
pub fn spawn_runtime_with_config(
    config: &RuntimeLaunchConfig,
) -> Result<RuntimeChild, ClientError> {
    let discovery = discovery_file_in(config.runtime_dir());
    if discovery.exists() {
        return Err(ClientError::Discovery(format!(
            "refusing to start a second runtime: a discovery document already exists at {}",
            discovery.display()
        )));
    }
    let binary = resolve_runtime_binary().ok_or_else(|| {
        ClientError::Discovery(format!(
            "runtime discovery is unavailable and no runtime binary was found; set {RUNTIME_BINARY_ENV} or place `{}` next to this executable",
            runtime_binary_name()
        ))
    })?;
    #[cfg(test)]
    let stderr = Stdio::piped();
    #[cfg(not(test))]
    let stderr = Stdio::null();
    let child = Command::new(&binary)
        .env(RUNTIME_DIR_ENV, config.runtime_dir())
        .env(RUNTIME_DB_ENV, config.db_path())
        .env(RUNTIME_APP_DATA_DIR_ENV, config.app_data_dir())
        .env_remove(LEGACY_HOME_ENV)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr)
        .spawn()
        .map_err(|error| {
            ClientError::Transport(format!(
                "cannot start runtime {}: {error}",
                binary.display()
            ))
        })?;
    Ok(RuntimeChild { child, exit: None })
}

/// Resolves the runtime binary from the environment or the sibling default.
fn resolve_runtime_binary() -> Option<PathBuf> {
    if let Some(explicit) = non_empty_env(RUNTIME_BINARY_ENV) {
        return Some(PathBuf::from(explicit));
    }
    let executable = std::env::current_exe().ok()?;
    let sibling = executable.parent()?.join(runtime_binary_name());
    sibling.exists().then_some(sibling)
}

fn runtime_binary_name() -> String {
    if cfg!(windows) {
        format!("{RUNTIME_BINARY_NAME}.exe")
    } else {
        RUNTIME_BINARY_NAME.to_string()
    }
}

/// Waits until the runtime publishes a valid discovery document *and* answers
/// the readiness handshake, then returns the document.
///
/// Reading the file alone is not readiness: a stale document from a previous
/// instance can also be present. Missing discovery and transport failures are
/// retried until `timeout`; a protocol or identity mismatch returns immediately
/// because waiting cannot fix it.
pub async fn wait_for_discovery(
    explicit: Option<&Path>,
    timeout: Duration,
) -> Result<ControlPlaneDiscovery, ClientError> {
    let deadline = tokio::time::Instant::now() + timeout;
    let path = try_discovery_path(explicit)?;
    loop {
        let ready = match load_discovery(Some(&path)) {
            Ok(document) => RuntimeClient::connect(&document).await.map(|_| document),
            Err(error) => Err(error),
        };
        match ready {
            Ok(document) => return Ok(document),
            Err(error) => {
                let retryable = matches!(error, ClientError::Transport(_))
                    || (matches!(error, ClientError::Discovery(_)) && discovery_absent(&path));
                if !retryable || tokio::time::Instant::now() >= deadline {
                    return Err(error);
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Environment variable naming the runtime binary used for explicit spawn.
pub const RUNTIME_BINARY_ENV: &str = "CHATSPEED_RUNTIME_BIN";

/// Default file name of the runtime binary started next to this executable.
const RUNTIME_BINARY_NAME: &str = "chatspeed-runtime";

/// Service name the standalone runtime reports on `GET /control/v1/meta`.
///
/// The desktop application's in-process workflow control plane reports a
/// different service name, so verifying it makes a client fail clearly instead
/// of silently speaking to the wrong endpoint.
pub const RUNTIME_SERVICE_NAME: &str = "chatspeed-runtime";

/// Environment key that overrides the derived heartbeat interval, in ms.
const HEARTBEAT_MS_ENV: &str = "CHATSPEED_RUNTIME_HEARTBEAT_MS";
/// Environment key the runtime uses for its lease TTL, in ms.
const LEASE_TTL_ENV: &str = "CHATSPEED_RUNTIME_LEASE_TTL_MS";
/// Smallest accepted heartbeat interval; keeps a very short lease renewing.
const MIN_HEARTBEAT_INTERVAL: Duration = Duration::from_millis(20);
/// Lease TTL assumed when the environment configures none.
const DEFAULT_ASSUMED_LEASE_TTL: Duration = Duration::from_secs(60);

/// `GET` route that reports runtime metadata.
pub const META_PATH: &str = "/control/v1/meta";
/// `POST` route that registers a client lease.
pub const CLIENT_REGISTER_PATH: &str = "/control/v1/clients/register";
/// `POST` route that lists the models a provider exposes.
pub const MODELS_LIST_PATH: &str = "/control/v1/models/list";

/// `POST` route prefix for the allowlisted runtime data commands.
///
/// The command set is the runtime's explicit allowlist; this client only ever
/// appends a fixed, known command name and never exposes a generic RPC.
pub const DATA_COMMANDS_PATH: &str = "/control/v1/data-commands";

/// Builds the allowlisted data-command path for one fixed command name.
pub fn data_command_path(command: &str) -> String {
    format!("{DATA_COMMANDS_PATH}/{command}")
}

/// Header that carries the opaque client-bridge session secret.
///
/// The secret is only ever sent here: never a URL, query string or body.
pub const BRIDGE_SESSION_HEADER: &str = "X-Bridge-Session";

/// `POST` route that invokes one declared client WebView capability through
/// the currently registered bridge session.
pub fn client_capability_invoke_path(capability: &str) -> String {
    format!(
        "/control/v1/client-capabilities/{}/invoke",
        encode_path_segment(capability)
    )
}

/// `POST` route that opens a client WebView capability bridge.
pub const CLIENT_BRIDGE_REGISTER_PATH: &str = "/control/v1/client-bridge/register";

/// `GET` route that pulls one bridge session's typed work envelopes.
pub fn bridge_events_path(session_id: &str) -> String {
    format!(
        "/control/v1/client-bridge/{}/events",
        encode_path_segment(session_id)
    )
}

/// `POST` route that returns one bridge invocation result.
pub fn bridge_result_path(session_id: &str) -> String {
    format!(
        "/control/v1/client-bridge/{}/result",
        encode_path_segment(session_id)
    )
}

/// `POST` route that cancels one bridge invocation.
pub fn bridge_cancel_path(session_id: &str) -> String {
    format!(
        "/control/v1/client-bridge/{}/cancel",
        encode_path_segment(session_id)
    )
}

/// `POST` route that closes one bridge session.
pub fn bridge_unregister_path(session_id: &str) -> String {
    format!(
        "/control/v1/client-bridge/{}/unregister",
        encode_path_segment(session_id)
    )
}

/// Header carrying the owning client id for a runtime terminal call.
///
/// The lease proof travels here rather than in a URL query or body, so it is
/// never captured by request logging.
pub const TERMINAL_CLIENT_HEADER: &str = "X-Terminal-Client";
/// Header carrying the owning lease id for a runtime terminal call.
pub const TERMINAL_LEASE_HEADER: &str = "X-Terminal-Lease";

/// `GET` route that lists the shells the runtime offers for a terminal.
pub const TERMINAL_SHELLS_PATH: &str = "/control/v1/terminal/shells";
/// `GET` route that lists the caller's own terminal sessions.
pub const TERMINAL_SESSIONS_PATH: &str = "/control/v1/terminal/sessions";
/// `POST` route that opens a terminal session.
pub const TERMINAL_CREATE_PATH: &str = "/control/v1/terminal/create";

/// `POST` route that forwards input to one terminal session.
pub fn terminal_write_path(session_id: &str) -> String {
    format!(
        "/control/v1/terminal/{}/write",
        encode_path_segment(session_id)
    )
}

/// `POST` route that resizes one terminal session.
pub fn terminal_resize_path(session_id: &str) -> String {
    format!(
        "/control/v1/terminal/{}/resize",
        encode_path_segment(session_id)
    )
}

/// `POST` route that retires one terminal session.
pub fn terminal_close_path(session_id: &str) -> String {
    format!(
        "/control/v1/terminal/{}/close",
        encode_path_segment(session_id)
    )
}

/// `GET` route that streams one terminal session's typed SSE envelopes.
pub fn terminal_stream_path(session_id: &str) -> String {
    format!(
        "/control/v1/terminal/{}/stream",
        encode_path_segment(session_id)
    )
}

/// `GET` route that streams one workflow session's versioned SSE envelopes.
pub fn workflow_stream_path(session_id: &str) -> String {
    format!(
        "/control/v1/workflows/{}/stream",
        encode_path_segment(session_id)
    )
}

/// Formats the opaque `Last-Event-ID` cursor of one workflow stream envelope.
///
/// The cursor (`${server_instance_id}:${sequence}`) is an instance-local replay
/// handle; a reconnect presents it so the server replays the gap instead of
/// starting a fresh subscription.
pub fn stream_envelope_cursor(envelope: &StreamEnvelope) -> String {
    format!("{}:{}", envelope.server_instance_id, envelope.sequence)
}

/// Default request timeout; a loopback control plane answers immediately.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Query parameter names that must never carry the bearer token. The runtime
/// rejects these as well; rejecting them here keeps a misconfigured caller from
/// putting a credential into a URL that ends up in logs or error messages.
const FORBIDDEN_TOKEN_QUERY_KEYS: [&str; 3] = ["token", "access_token", "auth"];

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Stable failure classification for control-plane clients.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// The discovery document is missing, unreadable or malformed.
    #[error("discovery error: {0}")]
    Discovery(String),
    /// The runtime reports an incompatible protocol version.
    #[error("protocol mismatch: {0}")]
    Protocol(String),
    /// The request could not be delivered (connection, timeout, body read).
    #[error("transport error: {0}")]
    Transport(String),
    /// The runtime rejected authentication or authorization.
    #[error("authentication failed: {0}")]
    Auth(String),
    /// The runtime returned a structured error for a non-2xx response.
    #[error("server error {status}: {message} ({code})")]
    Server {
        status: u16,
        code: String,
        message: String,
    },
    /// The caller asked for something this client refuses to send.
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    /// A response body did not match the expected wire shape.
    #[error("serialization error: {0}")]
    Serialization(String),
}

/// Maps a non-2xx response to a stable [`ClientError`], reusing the shared
/// [`ErrorEnvelope`] whenever the body carries one. Raw bodies are used only as
/// a fallback message and never include the token, which stays in the header.
fn error_from_status(status: StatusCode, body: &str) -> ClientError {
    let (code, message) = match serde_json::from_str::<ErrorEnvelope>(body) {
        Ok(envelope) => (envelope.error.code, envelope.error.message),
        Err(_) => (
            "unknown_error".to_string(),
            if body.trim().is_empty() {
                status
                    .canonical_reason()
                    .unwrap_or("response had no body")
                    .to_string()
            } else {
                body.to_string()
            },
        ),
    };

    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        ClientError::Auth(format!("{message} ({code})"))
    } else {
        ClientError::Server {
            status: status.as_u16(),
            code,
            message,
        }
    }
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

/// Resolves the discovery file path, failing closed instead of falling back.
///
/// An explicit path always wins. Otherwise the shared launch resolver decides
/// the runtime directory: an explicit `CHATSPEED_RUNTIME_DIR` sandbox,
/// `${CHATSPEED_HOME}/runtime`, or the build-profile default (the production
/// platform directory, or the repository's `dev_data`), so a client and the
/// runtime binary never disagree about where discovery lives. When no platform
/// directory is available and no override is configured this returns a
/// [`ClientError::Discovery`] rather than a `.`-relative guess.
pub fn try_discovery_path(explicit: Option<&Path>) -> Result<PathBuf, ClientError> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
    }
    let config =
        resolve_launch_config().map_err(|error| ClientError::Discovery(error.to_string()))?;
    Ok(config.discovery_file())
}

/// Discovery document path inside a known runtime directory.
pub fn discovery_file_in(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join(DISCOVERY_FILE_NAME)
}

/// Whether the discovery document is genuinely absent.
///
/// This is `true` only for a not-found result. An unreadable or malformed
/// document that still exists must not be treated as absent, otherwise a client
/// would start a second runtime instead of reporting the real problem.
pub fn discovery_absent(path: &Path) -> bool {
    matches!(
        std::fs::metadata(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound
    )
}

/// Reads, parses and protocol-checks the discovery document.
///
/// Errors name the file that failed but never its contents, so a malformed or
/// incompatible document cannot leak the bearer token.
pub fn load_discovery(explicit: Option<&Path>) -> Result<ControlPlaneDiscovery, ClientError> {
    let path = try_discovery_path(explicit)?;
    let body = std::fs::read(&path).map_err(|error| {
        ClientError::Discovery(format!(
            "cannot read discovery file {}: {error} (is ChatSpeed running?)",
            path.display()
        ))
    })?;
    let document: ControlPlaneDiscovery = serde_json::from_slice(&body).map_err(|error| {
        ClientError::Discovery(format!(
            "invalid discovery file {}: {error}",
            path.display()
        ))
    })?;
    validate_protocol_major(&document)?;
    Ok(document)
}

/// Extracts the major component of a protocol version such as `1` or `1.4.0`.
pub fn protocol_major(version: &str) -> u32 {
    version
        .split('.')
        .next()
        .and_then(|part| part.parse::<u32>().ok())
        .unwrap_or(0)
}

/// Rejects a discovery document whose protocol major differs from
/// [`SUPPORTED_PROTOCOL_MAJOR`].
pub fn validate_protocol_major(document: &ControlPlaneDiscovery) -> Result<(), ClientError> {
    let major = protocol_major(&document.protocol_version);
    if major != SUPPORTED_PROTOCOL_MAJOR {
        return Err(ClientError::Protocol(format!(
            "control plane reports protocol version {} but this client supports major version {}",
            document.protocol_version, SUPPORTED_PROTOCOL_MAJOR
        )));
    }
    let address = document.host.parse::<IpAddr>().map_err(|error| {
        ClientError::Discovery(format!("discovery host is not an IP address: {error}"))
    })?;
    if !address.is_loopback() {
        return Err(ClientError::Discovery(
            "control plane discovery must use a loopback host".to_string(),
        ));
    }
    Ok(())
}

/// Rejects a `/meta` document that does not describe the runtime named by the
/// discovery document.
fn validate_meta(
    discovery: &ControlPlaneDiscovery,
    meta: &MetaResponse,
) -> Result<(), ClientError> {
    if meta.service != RUNTIME_SERVICE_NAME {
        return Err(ClientError::Protocol(format!(
            "control plane reports service `{}` but `{RUNTIME_SERVICE_NAME}` is required; the desktop in-process control plane is not a supported runtime endpoint",
            meta.service
        )));
    }
    let major = protocol_major(&meta.protocol_version);
    if major != SUPPORTED_PROTOCOL_MAJOR {
        return Err(ClientError::Protocol(format!(
            "control plane reports protocol version {} but this client supports major version {}",
            meta.protocol_version, SUPPORTED_PROTOCOL_MAJOR
        )));
    }
    if meta.schema_version != SCHEMA_VERSION {
        return Err(ClientError::Protocol(format!(
            "control plane reports schema version {} but this client supports {}",
            meta.schema_version, SCHEMA_VERSION
        )));
    }
    if meta.server_instance_id != discovery.server_instance_id {
        return Err(ClientError::Protocol(format!(
            "stale discovery document: it names instance `{}` but the control plane reports `{}`",
            discovery.server_instance_id, meta.server_instance_id
        )));
    }
    if meta.pid != discovery.pid {
        return Err(ClientError::Protocol(format!(
            "stale discovery document: it names pid {} but the control plane reports {}",
            discovery.pid, meta.pid
        )));
    }
    Ok(())
}

fn non_empty_env(key: &str) -> Option<std::ffi::OsString> {
    match std::env::var_os(key) {
        Some(value) if !value.is_empty() => Some(value),
        _ => None,
    }
}

/// Reads a positive millisecond duration from the environment.
fn read_env_millis(key: &str) -> Option<Duration> {
    let millis = std::env::var(key).ok()?.trim().parse::<u64>().ok()?;
    (millis > 0).then(|| Duration::from_millis(millis))
}

// ---------------------------------------------------------------------------
// HTTP client
// ---------------------------------------------------------------------------

/// HTTP client bound to one loopback control-plane instance.
///
/// Cheap to clone: the underlying `reqwest::Client` is reference counted, so a
/// [`LeaseGuard`] can own its own handle.
#[derive(Clone)]
pub struct RuntimeClient {
    base_url: String,
    token: String,
    http: reqwest::Client,
    timeout: Duration,
}

impl fmt::Debug for RuntimeClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeClient")
            .field("base_url", &self.base_url)
            .field("token", &"<redacted>")
            .finish()
    }
}

impl RuntimeClient {
    /// Builds a client for a discovery document, validating it first.
    pub fn new(discovery: &ControlPlaneDiscovery) -> Result<Self, ClientError> {
        Self::with_timeout(discovery, DEFAULT_TIMEOUT)
    }

    /// Builds a client with an explicit per-request timeout.
    ///
    /// The discovery document is validated here too, so a caller cannot build a
    /// client for a non-loopback host or an incompatible protocol major by
    /// bypassing [`load_discovery`]. Redirects are never followed and proxies
    /// are never consulted: both would let the bearer token leave the loopback
    /// endpoint. There is no total request timeout on the client itself, so a
    /// long SSE stream is not cut off; normal requests apply `timeout` each.
    pub fn with_timeout(
        discovery: &ControlPlaneDiscovery,
        timeout: Duration,
    ) -> Result<Self, ClientError> {
        validate_protocol_major(discovery)?;
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(Policy::none())
            .connect_timeout(timeout)
            .build()
            .map_err(|error| {
                ClientError::Transport(format!("cannot build HTTP client: {error}"))
            })?;
        Ok(Self {
            base_url: format!("http://{}:{}", discovery.host, discovery.port),
            token: discovery.token.clone(),
            http,
            timeout,
        })
    }

    /// Builds a client and completes the readiness handshake against `/meta`.
    pub async fn connect(discovery: &ControlPlaneDiscovery) -> Result<Self, ClientError> {
        let client = Self::new(discovery)?;
        client.handshake(discovery).await?;
        Ok(client)
    }

    /// Verifies that the control plane behind the discovery document is the live
    /// standalone runtime that document claims.
    ///
    /// A stale document (different instance or pid), an incompatible protocol or
    /// schema, or a different service (for example the desktop in-process
    /// control plane) all fail here, before a lease is ever registered.
    pub async fn handshake(&self, discovery: &ControlPlaneDiscovery) -> Result<(), ClientError> {
        let value = self.get(META_PATH).await?;
        let meta: MetaResponse = serde_json::from_value(value).map_err(|error| {
            ClientError::Protocol(format!(
                "control plane /meta is not a recognized runtime document: {error}"
            ))
        })?;
        validate_meta(discovery, &meta)
    }

    /// Base URL (`http://host:port`) this client talks to.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Performs a `GET` and decodes the JSON response.
    pub async fn get(&self, path: &str) -> Result<Value, ClientError> {
        let response = self
            .build(Method::GET, path)?
            .send()
            .await
            .map_err(transport_error)?;
        decode(response).await
    }

    /// Performs a `POST` with a JSON body and decodes the JSON response.
    pub async fn post(&self, path: &str, body: &Value) -> Result<Value, ClientError> {
        self.send_json(Method::POST, path, body).await
    }

    /// Performs an idempotent `POST` with a stable key and one transport retry.
    ///
    /// The key is sent as a header, never as part of the URL or request body.
    /// Reusing it on the single retry lets the server deduplicate mutations.
    pub async fn post_with_idempotency(
        &self,
        path: &str,
        body: &Value,
        idempotency_key: &str,
    ) -> Result<Value, ClientError> {
        if idempotency_key.trim().is_empty() {
            return Err(ClientError::InvalidRequest(
                "idempotency key must not be empty".to_string(),
            ));
        }
        let mut attempt = 0;
        loop {
            let response = self
                .build(Method::POST, path)?
                .header("Idempotency-Key", idempotency_key)
                .json(body)
                .send()
                .await;
            match response {
                Ok(response) => return decode(response).await,
                Err(error) if attempt == 0 => {
                    attempt += 1;
                    let _ = error;
                }
                Err(error) => return Err(transport_error(error)),
            }
        }
    }

    /// Performs a `POST` with a typed body and decodes a typed response.
    pub async fn post_json<T, R>(&self, path: &str, body: &T) -> Result<R, ClientError>
    where
        T: Serialize + ?Sized,
        R: DeserializeOwned,
    {
        let value = self.send_json(Method::POST, path, body).await?;
        serde_json::from_value(value).map_err(|error| {
            ClientError::Serialization(format!("unexpected response body: {error}"))
        })
    }

    /// Opens an SSE stream; the caller consumes raw bytes incrementally.
    ///
    /// `last_event_id` resumes from a cursor when present. Non-2xx responses are
    /// decoded as structured errors instead of being returned as a stream. The
    /// request carries no total timeout, so a stream longer than the normal
    /// request timeout is not cut off.
    pub async fn stream(
        &self,
        path: &str,
        last_event_id: Option<&str>,
    ) -> Result<Response, ClientError> {
        let mut request = self.build_without_timeout(Method::GET, path)?;
        if let Some(cursor) = last_event_id {
            request = request.header("Last-Event-ID", cursor);
        }
        let response = request.send().await.map_err(transport_error)?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(error_from_status(status, &body));
        }
        Ok(response)
    }

    /// Lists the models a provider exposes through the runtime owner.
    pub async fn list_models(
        &self,
        request: &ListModelsRequest,
    ) -> Result<Vec<ModelDetailsDto>, ClientError> {
        self.post_json(MODELS_LIST_PATH, request).await
    }

    /// Returns the generated Models.dev provider presets the runtime owns.
    ///
    /// The presets are served from the same runtime snapshot the provider-model
    /// read resolves against, so the desktop never caches a second catalog.
    pub async fn models_dev_providers(
        &self,
    ) -> Result<Vec<ModelsDevPresetProviderDto>, ClientError> {
        self.post_json(
            &data_command_path("get_models_dev_providers"),
            &serde_json::json!({}),
        )
        .await
    }

    /// Returns the embedded catalog models of one Models.dev provider.
    pub async fn models_dev_provider_models(
        &self,
        request: &ModelsDevProviderModelsRequest,
    ) -> Result<Vec<ModelDetailsDto>, ClientError> {
        self.post_json(
            &data_command_path("get_models_dev_provider_models"),
            request,
        )
        .await
    }

    /// Resolves a model profile against the runtime-owned catalog snapshot.
    ///
    /// The response keeps the canonical camelCase profile shape; the desktop
    /// decodes it back into its own `ResolvedModelProfile` domain type.
    pub async fn resolve_model_profile(
        &self,
        request: &ResolveModelProfileRequest,
    ) -> Result<Value, ClientError> {
        self.post_json(&data_command_path("resolve_model_profile"), request)
            .await
    }

    /// Opens a client WebView capability bridge session.
    ///
    /// The bearer token proves the client; the runtime resolves the client's
    /// lease itself and requires the fixed `tauri` kind. The returned session
    /// token is the second credential every later bridge call must present.
    pub async fn register_bridge(
        &self,
        declaration: &ClientBridgeDeclaration,
        client_id: &str,
        lease_id: &str,
    ) -> Result<ClientBridgeRegistrationResponse, ClientError> {
        let registration = ClientBridgeRegistration {
            client_id: client_id.to_string(),
            lease_id: lease_id.to_string(),
            declaration: declaration.clone(),
        };
        self.post_json(CLIENT_BRIDGE_REGISTER_PATH, &registration)
            .await
    }

    /// Opens a bridge session from a prepared registration body.
    pub async fn register_bridge_request(
        &self,
        registration: &ClientBridgeRegistration,
    ) -> Result<ClientBridgeRegistrationResponse, ClientError> {
        self.post_json(CLIENT_BRIDGE_REGISTER_PATH, registration)
            .await
    }

    /// Registers the desktop-hosted loopback Web MCP provider with the runtime.
    ///
    /// The JSON body carries **only** the loopback port, so the runtime derives
    /// the endpoint itself. The provider proof token, the client id, the lease id
    /// and the desktop instance id travel in dedicated headers, never in the
    /// URL, query string or body.
    pub async fn register_web_mcp_provider(
        &self,
        registration: &WebMcpProviderRegistration,
        client_id: &str,
        lease_id: &str,
        instance_id: &str,
        proof: &str,
    ) -> Result<WebMcpProviderRegistrationResponse, ClientError> {
        let response = self
            .build(Method::POST, WEB_MCP_REGISTER_PATH)?
            .header(WEB_MCP_PROOF_HEADER, proof)
            .header(WEB_MCP_CLIENT_HEADER, client_id)
            .header(WEB_MCP_LEASE_HEADER, lease_id)
            .header(WEB_MCP_INSTANCE_HEADER, instance_id)
            .json(registration)
            .send()
            .await
            .map_err(transport_error)?;
        let value = decode(response).await?;
        serde_json::from_value(value).map_err(|error| {
            ClientError::Serialization(format!("unexpected response body: {error}"))
        })
    }

    /// Unregisters this client's loopback Web MCP provider.
    ///
    /// Repeating it for an already-dropped provider is harmless: the runtime
    /// treats a missing slot for the same proof as a no-op.
    pub async fn unregister_web_mcp_provider(
        &self,
        client_id: &str,
        lease_id: &str,
        instance_id: &str,
        proof: &str,
    ) -> Result<(), ClientError> {
        let response = self
            .build(Method::POST, WEB_MCP_UNREGISTER_PATH)?
            .header(WEB_MCP_PROOF_HEADER, proof)
            .header(WEB_MCP_CLIENT_HEADER, client_id)
            .header(WEB_MCP_LEASE_HEADER, lease_id)
            .header(WEB_MCP_INSTANCE_HEADER, instance_id)
            .json(&serde_json::json!({}))
            .send()
            .await
            .map_err(transport_error)?;
        decode(response).await?;
        Ok(())
    }

    /// Reads the current provider slot; `None` when no provider is installed.
    pub async fn web_mcp_provider_status(
        &self,
    ) -> Result<Option<WebMcpProviderStatus>, ClientError> {
        let response = self
            .build(Method::GET, WEB_MCP_STATUS_PATH)?
            .send()
            .await
            .map_err(transport_error)?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let value = decode(response).await?;
        if value.is_null() {
            return Ok(None);
        }
        serde_json::from_value(value)
            .map(Some)
            .map_err(|error| {
                ClientError::Serialization(format!("unexpected response body: {error}"))
            })
    }

    /// Starts one chat turn on the runtime owner.
    pub async fn start_chat(
        &self,
        chat_id: &str,
        request: &ChatStartRequest,
    ) -> Result<ChatStartResponse, ClientError> {
        self.post_json(&chat_start_path(chat_id), request).await
    }

    /// Stops the runtime-owned chat for `chat_id`.
    pub async fn stop_chat(
        &self,
        chat_id: &str,
        request: &ChatStopRequest,
    ) -> Result<ChatStopResponse, ClientError> {
        self.post_json(&chat_stop_path(chat_id), request).await
    }

    /// Subscribes to the typed SSE stream of one chat.
    ///
    /// The request carries no total timeout, so a long turn is never cut off by
    /// the normal request timeout.
    pub async fn stream_chat(&self, chat_id: &str) -> Result<ChatEventStream, ClientError> {
        let response = self.stream(&chat_events_path(chat_id), None).await?;
        Ok(ChatEventStream {
            response,
            buffer: Vec::new(),
            finished: false,
        })
    }

    /// Subscribes to the versioned SSE stream of one workflow session.
    ///
    /// `last_event_id` resumes from an opaque cursor after a reconnect. The
    /// request carries no total timeout, so a long-lived session is never cut
    /// off by the normal request timeout.
    pub async fn stream_workflow(
        &self,
        session_id: &str,
        last_event_id: Option<&str>,
    ) -> Result<WorkflowEventStream, ClientError> {
        let response = self
            .stream(&workflow_stream_path(session_id), last_event_id)
            .await?;
        Ok(WorkflowEventStream {
            response,
            buffer: Vec::new(),
            finished: false,
        })
    }

    /// Lists the shells the runtime offers for an interactive terminal.
    pub async fn terminal_list_shells(
        &self,
        client_id: &str,
        lease_id: &str,
    ) -> Result<Vec<TerminalShellDto>, ClientError> {
        let request = TerminalListShellsRequest::default();
        self.send_terminal(
            Method::GET,
            TERMINAL_SHELLS_PATH,
            client_id,
            lease_id,
            &request,
        )
        .await
    }

    /// Lists the terminal sessions owned by this client's live lease.
    pub async fn terminal_list_sessions(
        &self,
        client_id: &str,
        lease_id: &str,
    ) -> Result<Vec<TerminalSessionMetadataDto>, ClientError> {
        let request = TerminalListSessionsRequest::default();
        self.send_terminal(
            Method::GET,
            TERMINAL_SESSIONS_PATH,
            client_id,
            lease_id,
            &request,
        )
        .await
    }

    /// Opens a terminal session bound to this client's live lease.
    pub async fn terminal_create(
        &self,
        client_id: &str,
        lease_id: &str,
        request: &TerminalCreateRequest,
    ) -> Result<TerminalSessionMetadataDto, ClientError> {
        self.send_terminal(
            Method::POST,
            TERMINAL_CREATE_PATH,
            client_id,
            lease_id,
            request,
        )
        .await
    }

    /// Forwards raw input to a terminal session owned by this client.
    pub async fn terminal_write(
        &self,
        client_id: &str,
        lease_id: &str,
        session_id: &str,
        request: &TerminalWriteRequest,
    ) -> Result<(), ClientError> {
        self.send_terminal(
            Method::POST,
            &terminal_write_path(session_id),
            client_id,
            lease_id,
            request,
        )
        .await
    }

    /// Resizes a terminal session owned by this client.
    pub async fn terminal_resize(
        &self,
        client_id: &str,
        lease_id: &str,
        session_id: &str,
        request: &TerminalResizeRequest,
    ) -> Result<(), ClientError> {
        self.send_terminal(
            Method::POST,
            &terminal_resize_path(session_id),
            client_id,
            lease_id,
            request,
        )
        .await
    }

    /// Retires a terminal session owned by this client; repeating it is harmless.
    pub async fn terminal_close(
        &self,
        client_id: &str,
        lease_id: &str,
        session_id: &str,
    ) -> Result<(), ClientError> {
        let request = TerminalCloseRequest::default();
        self.send_terminal(
            Method::POST,
            &terminal_close_path(session_id),
            client_id,
            lease_id,
            &request,
        )
        .await
    }

    /// Subscribes to a terminal session's typed SSE stream.
    ///
    /// The request carries no total timeout, so a long-lived terminal is never
    /// cut off by the normal request timeout.
    pub async fn terminal_stream(
        &self,
        client_id: &str,
        lease_id: &str,
        session_id: &str,
    ) -> Result<TerminalEventStream, ClientError> {
        let response = self
            .build_without_timeout(Method::GET, &terminal_stream_path(session_id))?
            .header(TERMINAL_CLIENT_HEADER, client_id)
            .header(TERMINAL_LEASE_HEADER, lease_id)
            .send()
            .await
            .map_err(transport_error)?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(error_from_status(status, &body));
        }
        Ok(TerminalEventStream {
            response,
            buffer: Vec::new(),
            finished: false,
        })
    }

    /// Sends a typed terminal call that proves the caller's live lease.
    ///
    /// The lease proof travels in dedicated headers, never in the URL or body,
    /// so a credential can never be captured by request logging.
    async fn send_terminal<T, R>(
        &self,
        method: Method,
        path: &str,
        client_id: &str,
        lease_id: &str,
        body: &T,
    ) -> Result<R, ClientError>
    where
        T: Serialize + ?Sized,
        R: DeserializeOwned,
    {
        let response = self
            .build(method, path)?
            .header(TERMINAL_CLIENT_HEADER, client_id)
            .header(TERMINAL_LEASE_HEADER, lease_id)
            .json(body)
            .send()
            .await
            .map_err(transport_error)?;
        let value = decode(response).await?;
        serde_json::from_value(value).map_err(|error| {
            ClientError::Serialization(format!("unexpected response body: {error}"))
        })
    }

    async fn send_json<T>(&self, method: Method, path: &str, body: &T) -> Result<Value, ClientError>
    where
        T: Serialize + ?Sized,
    {
        let response = self
            .build(method, path)?
            .json(body)
            .send()
            .await
            .map_err(transport_error)?;
        decode(response).await
    }

    async fn send_without_body(&self, method: Method, path: &str) -> Result<Value, ClientError> {
        let response = self
            .build(method, path)?
            .send()
            .await
            .map_err(transport_error)?;
        decode(response).await
    }

    async fn post_empty_json<R: DeserializeOwned>(&self, path: &str) -> Result<R, ClientError> {
        let value = self.send_without_body(Method::POST, path).await?;
        serde_json::from_value(value).map_err(|error| {
            ClientError::Serialization(format!("unexpected response body: {error}"))
        })
    }

    /// Builds a request carrying the bearer token in the header only, with the
    /// configured per-request timeout.
    fn build(&self, method: Method, path: &str) -> Result<reqwest::RequestBuilder, ClientError> {
        Ok(self
            .build_without_timeout(method, path)?
            .timeout(self.timeout))
    }

    /// Builds a request with no total timeout, for streaming responses.
    fn build_without_timeout(
        &self,
        method: Method,
        path: &str,
    ) -> Result<reqwest::RequestBuilder, ClientError> {
        let url = self.resolve_url(path)?;
        Ok(self
            .http
            .request(method, url)
            .header(header::AUTHORIZATION, self.auth_header()))
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.token)
    }

    fn resolve_url(&self, path: &str) -> Result<String, ClientError> {
        if !path.starts_with('/') {
            return Err(ClientError::InvalidRequest(format!(
                "control-plane path must be absolute: {path}"
            )));
        }
        if let Some(name) = forbidden_query_key(path) {
            return Err(ClientError::InvalidRequest(format!(
                "credentials must use the Authorization header, not the URL query parameter `{name}`"
            )));
        }
        Ok(format!("{}{}", self.base_url, path))
    }
}

fn transport_error(error: reqwest::Error) -> ClientError {
    ClientError::Transport(format!("request failed: {error}"))
}

/// Incremental reader over one chat's typed SSE stream.
///
/// The stream is consumed frame by frame from the raw response body, so a long
/// turn is never buffered whole and never cut off by a request timeout. SSE
/// comment keepalives and `id`-only frames are skipped.
pub struct ChatEventStream {
    response: Response,
    buffer: Vec<u8>,
    finished: bool,
}

impl fmt::Debug for ChatEventStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChatEventStream")
            .field("buffered_bytes", &self.buffer.len())
            .field("finished", &self.finished)
            .finish()
    }
}

impl ChatEventStream {
    /// Returns the next chat envelope, or `None` once the stream has ended.
    pub async fn next_event(&mut self) -> Result<Option<ChatStreamEnvelope>, ClientError> {
        loop {
            if let Some(envelope) = self.take_event()? {
                return Ok(Some(envelope));
            }
            if self.finished {
                return Ok(None);
            }
            match self.response.chunk().await {
                Ok(Some(chunk)) => self.buffer.extend_from_slice(&chunk),
                Ok(None) => self.finished = true,
                Err(error) => return Err(transport_error(error)),
            }
        }
    }

    /// Pulls one complete `data:` frame out of the buffer.
    fn take_event(&mut self) -> Result<Option<ChatStreamEnvelope>, ClientError> {
        let Some(data) = next_frame_data(&mut self.buffer) else {
            return Ok(None);
        };
        let envelope = serde_json::from_str::<ChatStreamEnvelope>(&data).map_err(|error| {
            ClientError::Serialization(format!("invalid chat stream envelope: {error}"))
        })?;
        Ok(Some(envelope))
    }
}

/// An opened client WebView capability bridge session.
///
/// Cheap to clone. The opaque session token is the second credential every
/// bridge request must present; `Debug` never prints it.
#[derive(Clone)]
pub struct BridgeSession {
    client: RuntimeClient,
    session_id: String,
    session_token: String,
}

impl fmt::Debug for BridgeSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BridgeSession")
            .field("session_id", &self.session_id)
            .field("session_token", &"<redacted>")
            .finish()
    }
}

impl BridgeSession {
    /// Builds a session handle from a registration response.
    pub fn new(
        client: RuntimeClient,
        session_id: impl Into<String>,
        session_token: impl Into<String>,
    ) -> Self {
        Self {
            client,
            session_id: session_id.into(),
            session_token: session_token.into(),
        }
    }

    /// Opaque, non-secret session id.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// The control-plane client this session talks through.
    pub fn client(&self) -> &RuntimeClient {
        &self.client
    }

    /// Opens the typed SSE work stream.
    ///
    /// The request carries no total timeout, so a long-lived pull is never cut
    /// off by the normal request timeout.
    pub async fn events(&self) -> Result<BridgeEventStream, ClientError> {
        let response = self
            .client
            .build_without_timeout(Method::GET, &bridge_events_path(&self.session_id))?
            .header(BRIDGE_SESSION_HEADER, &self.session_token)
            .send()
            .await
            .map_err(transport_error)?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(error_from_status(status, &body));
        }
        Ok(BridgeEventStream {
            response,
            buffer: Vec::new(),
            finished: false,
        })
    }

    /// Invokes one declared client WebView capability using the bridge session
    /// credential, rather than the bearer alone.
    pub async fn invoke_client_capability<T: Serialize + ?Sized>(
        &self,
        capability: &str,
        body: &T,
    ) -> Result<Value, ClientError> {
        let response = self
            .client
            .build(Method::POST, &client_capability_invoke_path(capability))?
            .header(BRIDGE_SESSION_HEADER, &self.session_token)
            .json(body)
            .send()
            .await
            .map_err(transport_error)?;
        decode(response).await
    }

    /// Returns one typed invocation result.
    pub async fn submit_result(&self, result: &ClientCapabilityResult) -> Result<(), ClientError> {
        self.post_with_session(&bridge_result_path(&self.session_id), result)
            .await
    }

    /// Cancels one in-flight invocation owned by this session.
    pub async fn cancel(&self, request_id: &str) -> Result<(), ClientError> {
        let body = ClientBridgeCancelRequest {
            request_id: request_id.to_string(),
        };
        self.post_with_session(&bridge_cancel_path(&self.session_id), &body)
            .await
    }

    /// Closes the session. Unregistering an already-gone session succeeds.
    pub async fn unregister(&self) -> Result<(), ClientError> {
        let body = ClientBridgeUnregisterRequest { reason: None };
        self.post_with_session(&bridge_unregister_path(&self.session_id), &body)
            .await
    }

    /// `POST`s a body with the session credential in the dedicated header.
    async fn post_with_session<T: Serialize + ?Sized>(
        &self,
        path: &str,
        body: &T,
    ) -> Result<(), ClientError> {
        let response = self
            .client
            .build(Method::POST, path)?
            .header(BRIDGE_SESSION_HEADER, &self.session_token)
            .json(body)
            .send()
            .await
            .map_err(transport_error)?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        let text = response.text().await.unwrap_or_default();
        Err(error_from_status(status, &text))
    }
}

/// Incremental reader over one bridge session's typed SSE work stream.
///
/// Frames are consumed one at a time from the raw response body, so a
/// long-lived pull is never buffered whole and never cut off by a timeout. SSE
/// comment keepalives and `id`-only frames are skipped.
pub struct BridgeEventStream {
    response: Response,
    buffer: Vec<u8>,
    finished: bool,
}

impl fmt::Debug for BridgeEventStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BridgeEventStream")
            .field("buffered_bytes", &self.buffer.len())
            .field("finished", &self.finished)
            .finish()
    }
}

impl BridgeEventStream {
    /// Returns the next work envelope, or `None` once the stream has ended.
    pub async fn next_event(&mut self) -> Result<Option<ClientBridgeWorkEnvelope>, ClientError> {
        loop {
            if let Some(envelope) = self.take_event()? {
                return Ok(Some(envelope));
            }
            if self.finished {
                return Ok(None);
            }
            match self.response.chunk().await {
                Ok(Some(chunk)) => self.buffer.extend_from_slice(&chunk),
                Ok(None) => self.finished = true,
                Err(error) => return Err(transport_error(error)),
            }
        }
    }

    /// Pulls one complete `data:` frame out of the buffer.
    fn take_event(&mut self) -> Result<Option<ClientBridgeWorkEnvelope>, ClientError> {
        let Some(data) = next_frame_data(&mut self.buffer) else {
            return Ok(None);
        };
        let envelope = serde_json::from_str::<ClientBridgeWorkEnvelope>(&data).map_err(|error| {
            ClientError::Serialization(format!("invalid bridge stream envelope: {error}"))
        })?;
        Ok(Some(envelope))
    }
}

/// Incremental reader over one terminal session's typed SSE stream.
///
/// Frames are consumed one at a time from the raw response body, so a
/// long-lived terminal is never buffered whole and never cut off by a timeout.
/// SSE comment keepalives and `id`-only frames are skipped. A `Reset` or
/// `Unavailable` event terminates the stream: the runtime keeps no transcript,
/// so the caller must reset its view instead of expecting a replay.
pub struct TerminalEventStream {
    response: Response,
    buffer: Vec<u8>,
    finished: bool,
}

impl fmt::Debug for TerminalEventStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TerminalEventStream")
            .field("buffered_bytes", &self.buffer.len())
            .field("finished", &self.finished)
            .finish()
    }
}

impl TerminalEventStream {
    /// Returns the next terminal envelope, or `None` once the stream has ended.
    pub async fn next_event(&mut self) -> Result<Option<TerminalStreamEnvelope>, ClientError> {
        loop {
            if let Some(envelope) = self.take_event()? {
                return Ok(Some(envelope));
            }
            if self.finished {
                return Ok(None);
            }
            match self.response.chunk().await {
                Ok(Some(chunk)) => self.buffer.extend_from_slice(&chunk),
                Ok(None) => self.finished = true,
                Err(error) => return Err(transport_error(error)),
            }
        }
    }

    /// Pulls one complete `data:` frame out of the buffer.
    fn take_event(&mut self) -> Result<Option<TerminalStreamEnvelope>, ClientError> {
        let Some(data) = next_frame_data(&mut self.buffer) else {
            return Ok(None);
        };
        let envelope = serde_json::from_str::<TerminalStreamEnvelope>(&data).map_err(|error| {
            ClientError::Serialization(format!("invalid terminal stream envelope: {error}"))
        })?;
        Ok(Some(envelope))
    }
}

/// Incremental reader over one workflow session's versioned SSE stream.
///
/// Every frame is a [`StreamEnvelope`] whose `payload` is the raw gateway
/// payload the desktop forwards unchanged to its webview, so the envelope is a
/// pure transport wrapper and is never exposed to the frontend. Frames are
/// consumed one at a time from the raw response body, so a long-lived session
/// is never buffered whole and never cut off by a timeout. SSE comment
/// keepalives and `id`-only frames are skipped.
pub struct WorkflowEventStream {
    response: Response,
    buffer: Vec<u8>,
    finished: bool,
}

impl fmt::Debug for WorkflowEventStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkflowEventStream")
            .field("buffered_bytes", &self.buffer.len())
            .field("finished", &self.finished)
            .finish()
    }
}

impl WorkflowEventStream {
    /// Returns the next stream envelope, or `None` once the stream has ended.
    pub async fn next_event(&mut self) -> Result<Option<StreamEnvelope>, ClientError> {
        loop {
            if let Some(envelope) = self.take_event()? {
                return Ok(Some(envelope));
            }
            if self.finished {
                return Ok(None);
            }
            match self.response.chunk().await {
                Ok(Some(chunk)) => self.buffer.extend_from_slice(&chunk),
                Ok(None) => self.finished = true,
                Err(error) => return Err(transport_error(error)),
            }
        }
    }

    /// Pulls one complete `data:` frame out of the buffer.
    fn take_event(&mut self) -> Result<Option<StreamEnvelope>, ClientError> {
        let Some(data) = next_frame_data(&mut self.buffer) else {
            return Ok(None);
        };
        let envelope = serde_json::from_str::<StreamEnvelope>(&data).map_err(|error| {
            ClientError::Serialization(format!("invalid workflow stream envelope: {error}"))
        })?;
        Ok(Some(envelope))
    }
}

/// Finds the end of the first complete SSE frame (`\n\n` or `\r\n\r\n`).
fn frame_end(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(2)
        .position(|window| window == b"\n\n")
        .map(|position| position + 2)
        .or_else(|| {
            buffer
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .map(|position| position + 4)
        })
}

/// Pulls the `data:` payload of the next complete SSE frame out of `buffer`.
///
/// The one frame parser every typed stream reader shares: it returns `None`
/// while no complete frame is buffered yet, and skips frames that carry no
/// `data:` field (comment keepalives and `id`-only frames).
fn next_frame_data(buffer: &mut Vec<u8>) -> Option<String> {
    loop {
        let end = frame_end(buffer)?;
        let frame = String::from_utf8_lossy(&buffer[..end]).to_string();
        buffer.drain(..end);

        let mut data = String::new();
        for line in frame.lines() {
            let line = line.trim_end_matches('\r');
            if let Some(value) = line.strip_prefix("data:") {
                let value = value.strip_prefix(' ').unwrap_or(value);
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(value);
            }
        }
        if data.is_empty() {
            // A keepalive comment or an `id`-only frame carries no payload.
            continue;
        }
        return Some(data);
    }
}

/// Returns the first credential-shaped query parameter in `path`, if any.
fn forbidden_query_key(path: &str) -> Option<String> {
    let query = path.split_once('?')?.1;
    query.split('&').find_map(|pair| {
        let name = pair.split('=').next().unwrap_or("");
        let lower = name.to_ascii_lowercase();
        FORBIDDEN_TOKEN_QUERY_KEYS
            .contains(&lower.as_str())
            .then_some(lower)
    })
}

async fn decode(response: Response) -> Result<Value, ClientError> {
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| ClientError::Transport(format!("cannot read response body: {error}")))?;
    if !status.is_success() {
        return Err(error_from_status(status, &body));
    }
    if body.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&body)
        .map_err(|error| ClientError::Serialization(format!("invalid JSON response: {error}")))
}

// ---------------------------------------------------------------------------
// Leases
// ---------------------------------------------------------------------------

/// `POST` route that renews an existing lease.
pub fn renew_path(client_id: &str) -> String {
    format!(
        "/control/v1/clients/{}/renew",
        encode_path_segment(client_id)
    )
}

/// `POST` route that releases a lease.
pub fn release_path(client_id: &str) -> String {
    format!(
        "/control/v1/clients/{}/release",
        encode_path_segment(client_id)
    )
}

/// `POST` route that starts one chat turn.
pub fn chat_start_path(chat_id: &str) -> String {
    format!("/control/v1/chats/{}/start", encode_path_segment(chat_id))
}

/// `POST` route that stops one chat turn.
pub fn chat_stop_path(chat_id: &str) -> String {
    format!("/control/v1/chats/{}/stop", encode_path_segment(chat_id))
}

/// `GET` route that streams one chat's typed SSE envelopes.
pub fn chat_events_path(chat_id: &str) -> String {
    format!("/control/v1/chats/{}/events", encode_path_segment(chat_id))
}

/// Percent-encodes one path segment so a client id cannot escape its route slot.
fn encode_path_segment(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char)
            }
            _ => {
                encoded.push('%');
                encoded.push_str(&format!("{byte:02X}"));
            }
        }
    }
    encoded
}

/// A running lease heartbeat.
///
/// Owns the renewal task. [`Heartbeat::stop`] signals it and waits for the task
/// to exit, so a caller can release its lease without racing an in-flight
/// renewal.
pub struct Heartbeat {
    status: tokio::sync::watch::Receiver<Result<(), String>>,
    stop: tokio::sync::watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

impl fmt::Debug for Heartbeat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Heartbeat")
            .field("finished", &self.task.is_finished())
            .finish()
    }
}

impl Heartbeat {
    /// Latest renewal result; `Ok` until a renewal fails.
    pub fn status(&self) -> Result<(), String> {
        self.status.borrow().clone()
    }

    /// Resolves with the failure message the first time a renewal fails.
    ///
    /// Never resolves while renewals keep succeeding, or after the task stops,
    /// so a caller can `select!` it against command work.
    pub async fn failure(&mut self) -> Option<String> {
        loop {
            if let Err(message) = self.status.borrow().clone() {
                return Some(message);
            }
            if self.status.changed().await.is_err() {
                return None;
            }
        }
    }

    /// Stops the heartbeat and waits until the renewal task has exited.
    pub async fn stop(self) {
        let _ = self.stop.send(true);
        let _ = self.task.await;
    }
}

/// A held client lease, registered with and released against the runtime.
///
/// The guard owns its own [`RuntimeClient`] handle. Registering happens through
/// [`LeaseGuard::register`]; call [`LeaseGuard::renew`] before the TTL expires
/// and [`LeaseGuard::release`] when the work is done.
#[must_use = "the runtime stays leased until the TTL expires; call release()"]
pub struct LeaseGuard {
    client: RuntimeClient,
    client_id: String,
    client_kind: String,
    lease: ClientLeaseResponse,
    released: bool,
}

impl fmt::Debug for LeaseGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LeaseGuard")
            .field("client_id", &self.client_id)
            .field("client_kind", &self.client_kind)
            .field("lease_id", &"<redacted>")
            .field("expires_at", &self.lease.expires_at)
            .field("released", &self.released)
            .finish()
    }
}

impl LeaseGuard {
    /// Registers a lease for `client_id`/`client_kind` and returns its guard.
    pub async fn register(
        client: &RuntimeClient,
        client_id: &str,
        client_kind: &str,
    ) -> Result<Self, ClientError> {
        let request = ClientLeaseRequest {
            client_id: client_id.to_string(),
            client_kind: client_kind.to_string(),
        };
        let response: ClientLeaseResponse =
            client.post_json(CLIENT_REGISTER_PATH, &request).await?;
        Ok(Self::from_response(client.clone(), client_kind, response))
    }

    fn from_response(
        client: RuntimeClient,
        client_kind: &str,
        response: ClientLeaseResponse,
    ) -> Self {
        Self {
            client,
            client_id: response.client_id.clone(),
            client_kind: client_kind.to_string(),
            lease: response,
            released: false,
        }
    }

    /// Client id this lease is registered under.
    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    /// Client kind this lease was registered with.
    pub fn client_kind(&self) -> &str {
        &self.client_kind
    }

    /// Opaque lease id; treat it as a capability and do not log it.
    pub fn lease_id(&self) -> &str {
        &self.lease.lease_id
    }

    /// RFC 3339 expiry reported by the runtime.
    pub fn expires_at(&self) -> &str {
        &self.lease.expires_at
    }

    /// Whether [`LeaseGuard::release`] has already run.
    pub fn is_released(&self) -> bool {
        self.released
    }

    /// Projection of this lease into the shared wire DTO.
    pub fn lease(&self) -> ClientLease {
        ClientLease {
            client_id: self.client_id.clone(),
            lease_id: self.lease.lease_id.clone(),
            client_kind: self.client_kind.clone(),
            expires_at: self.lease.expires_at.clone(),
        }
    }

    /// Starts a background heartbeat sized from the lease lifetime.
    pub fn heartbeat(&self) -> Heartbeat {
        self.heartbeat_with_interval(self.renew_interval())
    }

    /// Starts a background heartbeat with an explicit interval.
    pub fn heartbeat_with_interval(&self, interval: Duration) -> Heartbeat {
        let interval = interval.max(MIN_HEARTBEAT_INTERVAL);
        let (result_tx, result_rx) = tokio::sync::watch::channel(Ok(()));
        let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
        let mut lease = self.clone_for_heartbeat();
        let task = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            // The first interval tick is immediate; skip it so a fresh lease is
            // never renewed before it needs to be.
            ticker.tick().await;
            loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        if let Err(error) = lease.renew().await {
                            let _ = result_tx.send_replace(Err(error.to_string()));
                            return;
                        }
                        let _ = result_tx.send_replace(Ok(()));
                    }
                    changed = stop_rx.changed() => {
                        if changed.is_err() || *stop_rx.borrow() {
                            return;
                        }
                    }
                }
            }
        });
        Heartbeat {
            status: result_rx,
            stop: stop_tx,
            task,
        }
    }

    /// Renewal interval derived from the lease lifetime.
    ///
    /// `expires_at` carries only second precision, so an explicit millisecond
    /// TTL (the same one the runtime reads) is preferred when present; otherwise
    /// a third of the assumed TTL keeps the lease ahead of expiry. A wire TTL
    /// field would remove this guesswork, but the contract does not carry one.
    fn renew_interval(&self) -> Duration {
        if let Some(millis) = read_env_millis(HEARTBEAT_MS_ENV) {
            return millis.max(MIN_HEARTBEAT_INTERVAL);
        }
        if let Some(millis) = read_env_millis(LEASE_TTL_ENV) {
            return (millis / 3).max(MIN_HEARTBEAT_INTERVAL);
        }
        (DEFAULT_ASSUMED_LEASE_TTL / 3).max(MIN_HEARTBEAT_INTERVAL)
    }

    fn clone_for_heartbeat(&self) -> Self {
        Self {
            client: self.client.clone(),
            client_id: self.client_id.clone(),
            client_kind: self.client_kind.clone(),
            lease: self.lease.clone(),
            released: self.released,
        }
    }

    /// Extends the lease; the runtime keeps the same lease id.
    pub async fn renew(&mut self) -> Result<&ClientLeaseResponse, ClientError> {
        if self.released {
            return Err(ClientError::InvalidRequest(
                "lease was already released".to_string(),
            ));
        }
        let response: ClientLeaseResponse = self
            .client
            .post_empty_json(&renew_path(&self.client_id))
            .await?;
        self.lease = response;
        Ok(&self.lease)
    }

    /// Releases the lease explicitly. Releasing twice is a no-op, so a caller
    /// may release defensively without risking a `not_found`.
    pub async fn release(&mut self) -> Result<(), ClientError> {
        if self.released {
            return Ok(());
        }
        self.client
            .send_without_body(Method::POST, &release_path(&self.client_id))
            .await?;
        self.released = true;
        Ok(())
    }
}

impl Drop for LeaseGuard {
    // Dropping a guard deliberately performs no I/O: `Drop` cannot await, and a
    // best-effort release would race the runtime's shutdown. Call `release()`
    // explicitly when a prompt release matters; otherwise the lease expires on
    // its TTL and the idle supervisor stops the runtime.
    fn drop(&mut self) {}
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chatspeed_contracts::{
        ChatProtocolDto, ChatResponseDto, ChatStartRequest, ChatStopRequest, ChatStreamEvent,
        ClientBridgeCapability, ClientBridgeDeclaration, ClientBridgeWorkEnvelope,
        ClientCapabilityResult, ClientCapabilityStatus, FinishReasonDto, ListModelsRequest,
        MessageTypeDto, ModelDetailsDto, TerminalOutputEvent, TerminalStreamEvent,
        BRIDGE_PROTOCOL_VERSION, BRIDGE_SCHEMA_VERSION, CONTROL_PLANE_HOST, PROTOCOL_VERSION,
    };
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    fn discovery(protocol_version: &str, token: &str) -> ControlPlaneDiscovery {
        ControlPlaneDiscovery {
            protocol_version: protocol_version.to_string(),
            server_instance_id: "instance-a".to_string(),
            pid: 42,
            host: CONTROL_PLANE_HOST.to_string(),
            port: 41234,
            token: token.to_string(),
            started_at: "unix-1".to_string(),
        }
    }

    fn discovery_on(port: u16, instance: &str, pid: u32) -> ControlPlaneDiscovery {
        ControlPlaneDiscovery {
            protocol_version: PROTOCOL_VERSION.to_string(),
            server_instance_id: instance.to_string(),
            pid,
            host: CONTROL_PLANE_HOST.to_string(),
            port,
            token: "test-token".to_string(),
            started_at: "unix-1".to_string(),
        }
    }

    fn client() -> RuntimeClient {
        RuntimeClient::new(&discovery(PROTOCOL_VERSION, "test-token")).expect("client")
    }

    fn lease_response() -> ClientLeaseResponse {
        ClientLeaseResponse {
            client_id: "tauri-main".to_string(),
            lease_id: "lease-1".to_string(),
            expires_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    fn auth_header_value(request: &reqwest::Request) -> Option<&str> {
        request
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
    }

    // -- loopback fake server ----------------------------------------------

    /// One captured HTTP request .
    #[derive(Clone)]
    struct Recorded {
        method: String,
        path: String,
        authorization: Option<String>,
        bridge_session: Option<String>,
        terminal_client: Option<String>,
        terminal_lease: Option<String>,
        web_proof: Option<String>,
        web_client: Option<String>,
        web_lease: Option<String>,
        web_instance: Option<String>,
        last_event_id: Option<String>,
        body: String,
    }

    /// A scripted loopback HTTP/1.1 server.
    ///
    /// Parses real TCP requests and answers with the raw bytes `handler` returns,
    /// so transport behavior (redirects, proxies, bearer headers) is exercised
    /// against actual sockets without a web framework.
    struct FakeServer {
        addr: std::net::SocketAddr,
        recorded: Arc<Mutex<Vec<Recorded>>>,
        task: tokio::task::JoinHandle<()>,
    }

    impl FakeServer {
        async fn start<F>(handler: F) -> Self
        where
            F: Fn(&Recorded) -> Vec<u8> + Send + Sync + 'static,
        {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
            let addr = listener.local_addr().expect("addr");
            let recorded = Arc::new(Mutex::new(Vec::new()));
            let sink = recorded.clone();
            let handler = Arc::new(handler);
            let task = tokio::spawn(async move {
                loop {
                    let Ok((mut stream, _)) = listener.accept().await else {
                        break;
                    };
                    let Some(request) = read_request(&mut stream).await else {
                        continue;
                    };
                    let response = handler(&request);
                    sink.lock().expect("record").push(request);
                    let _ = stream.write_all(&response).await;
                    let _ = stream.flush().await;
                    let _ = stream.shutdown().await;
                }
            });
            Self {
                addr,
                recorded,
                task,
            }
        }

        fn requests(&self) -> Vec<Recorded> {
            self.recorded.lock().expect("record").clone()
        }
    }

    impl Drop for FakeServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    async fn read_request(stream: &mut TcpStream) -> Option<Recorded> {
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 4096];
        let header_end = loop {
            let read = stream.read(&mut chunk).await.ok()?;
            if read == 0 {
                return None;
            }
            buffer.extend_from_slice(&chunk[..read]);
            if let Some(position) = find_subsequence(&buffer, b"\r\n\r\n") {
                break position + 4;
            }
            if buffer.len() > 1 << 20 {
                return None;
            }
        };

        let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
        let mut lines = head.split("\r\n");
        let request_line = lines.next().unwrap_or("");
        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or("").to_string();
        let path = parts.next().unwrap_or("").to_string();
        let mut authorization = None;
        let mut bridge_session = None;
        let mut terminal_client = None;
        let mut terminal_lease = None;
        let mut web_proof = None;
        let mut web_client = None;
        let mut web_lease = None;
        let mut web_instance = None;
        let mut last_event_id = None;
        let mut content_length = 0usize;
        for line in lines {
            if line.is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                let name = name.trim().to_ascii_lowercase();
                if name == "authorization" {
                    authorization = Some(value.trim().to_string());
                } else if name == "x-bridge-session" {
                    bridge_session = Some(value.trim().to_string());
                } else if name == "x-terminal-client" {
                    terminal_client = Some(value.trim().to_string());
                } else if name == "x-terminal-lease" {
                    terminal_lease = Some(value.trim().to_string());
                } else if name == "x-web-mcp-proof" {
                    web_proof = Some(value.trim().to_string());
                } else if name == "x-web-mcp-client" {
                    web_client = Some(value.trim().to_string());
                } else if name == "x-web-mcp-lease" {
                    web_lease = Some(value.trim().to_string());
                } else if name == "x-web-mcp-instance" {
                    web_instance = Some(value.trim().to_string());
                } else if name == "last-event-id" {
                    last_event_id = Some(value.trim().to_string());
                } else if name == "content-length" {
                    content_length = value.trim().parse().unwrap_or(0);
                }
            }
        }

        let mut body = buffer[header_end..].to_vec();
        while body.len() < content_length {
            let read = stream.read(&mut chunk).await.ok()?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..read]);
        }

        Some(Recorded {
            method,
            path,
            authorization,
            bridge_session,
            terminal_client,
            terminal_lease,
            web_proof,
            web_client,
            web_lease,
            web_instance,
            last_event_id,
            body: String::from_utf8_lossy(&body).to_string(),
        })
    }

    fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack
            .windows(needle.len())
            .position(|window| window == needle)
    }

    fn http_response(status: &str, body: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    fn meta_json(instance: &str, pid: u32, schema: u32, service: &str, protocol: &str) -> String {
        json!({
            "service": service,
            "protocol_version": protocol,
            "schema_version": schema,
            "server_instance_id": instance,
            "pid": pid,
        })
        .to_string()
    }

    async fn meta_server(meta: String) -> FakeServer {
        FakeServer::start(move |_| http_response("200 OK", &meta)).await
    }

    // -- protocol -----------------------------------------------------------

    #[test]
    fn protocol_major_accepts_the_current_version_and_rejects_other_majors() {
        let current = discovery(PROTOCOL_VERSION, "token");
        assert!(validate_protocol_major(&current).is_ok());

        let future = discovery("2", "token");
        let error = validate_protocol_major(&future).expect_err("major 2 must be rejected");
        assert!(matches!(error, ClientError::Protocol(_)));

        assert_eq!(protocol_major("2.1.0"), 2);
        assert_eq!(protocol_major("1"), 1);
        assert_eq!(protocol_major("not-a-version"), 0);
    }

    #[test]
    fn load_discovery_rejects_incompatible_major_without_leaking_the_token() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(DISCOVERY_FILE_NAME);
        let document = discovery("2", "super-secret-token");
        std::fs::write(&path, serde_json::to_vec(&document).expect("encode")).expect("write");

        let error = load_discovery(Some(&path)).expect_err("major 2 must be rejected");
        assert!(matches!(error, ClientError::Protocol(_)));
        assert!(!error.to_string().contains("super-secret-token"));
    }

    #[test]
    fn load_discovery_accepts_the_current_protocol() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(DISCOVERY_FILE_NAME);
        let document = discovery(PROTOCOL_VERSION, "token");
        std::fs::write(&path, serde_json::to_vec(&document).expect("encode")).expect("write");

        let loaded = load_discovery(Some(&path)).expect("valid discovery");
        assert_eq!(loaded.port, 41234);
        assert_eq!(loaded.protocol_version, PROTOCOL_VERSION);
    }

    #[test]
    fn load_discovery_reports_a_missing_file_as_a_discovery_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("missing.json");

        let error = load_discovery(Some(&missing)).expect_err("missing file must fail");
        assert!(matches!(error, ClientError::Discovery(_)));
        assert!(error.to_string().contains(&missing.display().to_string()));
    }

    // -- discovery path -----------------------------------------------------

    #[test]
    fn discovery_path_priority_is_explicit_runtime_dir_home_then_profile_default() {
        // The process environment is shared with other tests, so this test owns
        // every mutation under the crate lock and restores the original values.
        let _guard = crate::ENV_LOCK.lock().expect("env lock");
        let original_runtime_dir = std::env::var_os(RUNTIME_DIR_ENV);
        let original_home = std::env::var_os(LEGACY_HOME_ENV);

        let explicit = PathBuf::from("/explicit/control-plane-v1.json");
        std::env::set_var(RUNTIME_DIR_ENV, "/explicit-runtime");
        std::env::set_var(LEGACY_HOME_ENV, "/explicit-home");

        // An explicit path always wins.
        assert_eq!(
            try_discovery_path(Some(explicit.as_path())).expect("explicit"),
            explicit
        );
        // Then CHATSPEED_RUNTIME_DIR.
        assert_eq!(
            try_discovery_path(None).expect("runtime dir"),
            PathBuf::from("/explicit-runtime").join(DISCOVERY_FILE_NAME)
        );
        // Then CHATSPEED_HOME/runtime.
        std::env::remove_var(RUNTIME_DIR_ENV);
        assert_eq!(
            try_discovery_path(None).expect("home"),
            PathBuf::from("/explicit-home/runtime").join(DISCOVERY_FILE_NAME)
        );
        // Then the build-profile default, never a `.`-relative fallback.
        std::env::remove_var(LEGACY_HOME_ENV);
        let default_root = match build_profile() {
            BuildProfile::Debug => dev_app_data_dir().expect("dev root"),
            BuildProfile::Release => {
                production_app_data_dir(&dirs::data_dir().expect("platform data dir"))
            }
        };
        assert_eq!(
            try_discovery_path(None).expect("profile default"),
            default_root.join("runtime").join(DISCOVERY_FILE_NAME)
        );

        restore_env(RUNTIME_DIR_ENV, original_runtime_dir);
        restore_env(LEGACY_HOME_ENV, original_home);
    }

    fn restore_env(key: &str, value: Option<std::ffi::OsString>) {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }

    // -- transport ----------------------------------------------------------

    #[test]
    fn built_requests_carry_the_bearer_token_in_the_header_only() {
        let client = client();
        let request = client
            .build(Method::GET, META_PATH)
            .expect("build")
            .build()
            .expect("request");

        assert_eq!(
            request.url().as_str(),
            "http://127.0.0.1:41234/control/v1/meta"
        );
        assert_eq!(auth_header_value(&request), Some("Bearer test-token"));
        // The client never sends Origin, and the token never reaches the URL.
        assert!(request.headers().get(header::ORIGIN).is_none());
        assert!(!request.url().as_str().contains("test-token"));
    }

    #[test]
    fn url_tokens_and_relative_paths_are_rejected_before_any_request() {
        let client = client();

        let error = client
            .build(Method::GET, "/control/v1/meta?token=leaked")
            .expect_err("a URL token must be rejected");
        assert!(matches!(error, ClientError::InvalidRequest(_)));

        let error = client
            .build(Method::GET, "control/v1/meta")
            .expect_err("a relative path must be rejected");
        assert!(matches!(error, ClientError::InvalidRequest(_)));
    }

    // -- error mapping ------------------------------------------------------

    #[test]
    fn non_success_responses_reuse_the_shared_error_envelope() {
        let unauthorized = error_from_status(
            StatusCode::UNAUTHORIZED,
            r#"{"error":{"code":"unauthorized","message":"Missing or invalid bearer token"}}"#,
        );
        match unauthorized {
            ClientError::Auth(message) => {
                assert!(message.contains("unauthorized"));
                assert!(message.contains("Missing or invalid bearer token"));
            }
            other => panic!("expected an auth error, got {other:?}"),
        }

        let server = error_from_status(
            StatusCode::INTERNAL_SERVER_ERROR,
            r#"{"error":{"code":"internal_error","message":"boom"}}"#,
        );
        match server {
            ClientError::Server {
                status,
                code,
                message,
            } => {
                assert_eq!(status, 500);
                assert_eq!(code, "internal_error");
                assert_eq!(message, "boom");
            }
            other => panic!("expected a server error, got {other:?}"),
        }

        // A body without an envelope falls back without swallowing the status.
        let unparseable = error_from_status(StatusCode::BAD_GATEWAY, "not json");
        match unparseable {
            ClientError::Server {
                status,
                code,
                message,
            } => {
                assert_eq!(status, 502);
                assert_eq!(code, "unknown_error");
                assert_eq!(message, "not json");
            }
            other => panic!("expected a fallback server error, got {other:?}"),
        }
    }

    // -- leases -------------------------------------------------------------

    #[test]
    fn lease_dtos_match_the_shared_wire_contract() {
        let request = ClientLeaseRequest {
            client_id: "tauri-main".to_string(),
            client_kind: "tauri".to_string(),
        };
        assert_eq!(
            serde_json::to_value(&request).expect("serialize request"),
            json!({"client_id": "tauri-main", "client_kind": "tauri"})
        );

        let response: ClientLeaseResponse = serde_json::from_value(json!({
            "client_id": "tauri-main",
            "lease_id": "lease-1",
            "expires_at": "2026-01-01T00:00:00Z"
        }))
        .expect("deserialize response");

        let guard = LeaseGuard::from_response(client(), "tauri", response);
        assert_eq!(guard.client_id(), "tauri-main");
        assert_eq!(guard.client_kind(), "tauri");
        assert_eq!(guard.lease_id(), "lease-1");
        assert_eq!(guard.expires_at(), "2026-01-01T00:00:00Z");
        assert!(!guard.is_released());
        assert_eq!(
            guard.lease(),
            ClientLease {
                client_id: "tauri-main".to_string(),
                lease_id: "lease-1".to_string(),
                client_kind: "tauri".to_string(),
                expires_at: "2026-01-01T00:00:00Z".to_string(),
            }
        );
    }

    #[test]
    fn release_and_renew_requests_target_the_documented_routes() {
        assert_eq!(
            release_path("tauri-main"),
            "/control/v1/clients/tauri-main/release"
        );
        assert_eq!(
            renew_path("tauri-main"),
            "/control/v1/clients/tauri-main/renew"
        );

        let client = client();
        let request = client
            .build(Method::POST, &release_path("tauri-main"))
            .expect("build")
            .build()
            .expect("request");
        assert_eq!(
            request.url().as_str(),
            "http://127.0.0.1:41234/control/v1/clients/tauri-main/release"
        );
        assert_eq!(auth_header_value(&request), Some("Bearer test-token"));

        // A client id containing a slash cannot escape its path segment.
        assert_eq!(release_path("a/b"), "/control/v1/clients/a%2Fb/release");
        assert!(!release_path("a/b").contains("a/b"));
    }

    #[test]
    fn debug_representations_do_not_expose_secrets() {
        let client = client();
        let debug = format!("{client:?}");
        assert!(!debug.contains("test-token"));
        assert!(debug.contains("<redacted>"));

        let mut response = lease_response();
        response.lease_id = "lease-secret".to_string();
        let guard = LeaseGuard::from_response(client, "tauri", response);
        let debug = format!("{guard:?}");
        assert!(!debug.contains("lease-secret"));
        assert!(debug.contains("<redacted>"));
    }

    // -- handshake ----------------------------------------------------------

    #[test]
    fn new_rejects_non_loopback_hosts_and_incompatible_majors() {
        let mut remote = discovery(PROTOCOL_VERSION, "token");
        remote.host = "10.1.2.3".to_string();
        assert!(matches!(
            RuntimeClient::new(&remote),
            Err(ClientError::Discovery(_))
        ));

        let wrong_major = discovery("2", "token");
        assert!(matches!(
            RuntimeClient::new(&wrong_major),
            Err(ClientError::Protocol(_))
        ));
    }

    #[tokio::test]
    async fn handshake_rejects_a_stale_or_wrong_control_plane() {
        let mismatches = [
            meta_json(
                "other-instance",
                42,
                SCHEMA_VERSION,
                RUNTIME_SERVICE_NAME,
                PROTOCOL_VERSION,
            ),
            meta_json(
                "instance-a",
                43,
                SCHEMA_VERSION,
                RUNTIME_SERVICE_NAME,
                PROTOCOL_VERSION,
            ),
            meta_json(
                "instance-a",
                42,
                SCHEMA_VERSION + 1,
                RUNTIME_SERVICE_NAME,
                PROTOCOL_VERSION,
            ),
            // The desktop in-process control plane by its real service name.
            meta_json(
                "instance-a",
                42,
                SCHEMA_VERSION,
                "chatspeed-workflow-control-plane",
                PROTOCOL_VERSION,
            ),
            meta_json("instance-a", 42, SCHEMA_VERSION, RUNTIME_SERVICE_NAME, "2"),
        ];

        for meta in mismatches {
            let server = meta_server(meta).await;
            let document = discovery_on(server.addr.port(), "instance-a", 42);
            let error = RuntimeClient::connect(&document)
                .await
                .expect_err("a mismatched control plane must be rejected");
            assert!(
                matches!(error, ClientError::Protocol(_)),
                "unexpected {error:?}"
            );
        }

        let server = meta_server(meta_json(
            "instance-a",
            42,
            SCHEMA_VERSION,
            RUNTIME_SERVICE_NAME,
            PROTOCOL_VERSION,
        ))
        .await;
        let document = discovery_on(server.addr.port(), "instance-a", 42);
        let client = RuntimeClient::connect(&document)
            .await
            .expect("a matching control plane completes the handshake");
        assert_eq!(
            client.base_url(),
            format!("http://127.0.0.1:{}", server.addr.port())
        );
    }

    #[test]
    fn streams_have_no_total_timeout_while_normal_requests_do() {
        let client = client();
        let request = client
            .build(Method::GET, META_PATH)
            .expect("build")
            .build()
            .expect("request");
        assert_eq!(request.timeout().copied(), Some(DEFAULT_TIMEOUT));

        let stream = client
            .build_without_timeout(Method::GET, "/control/v1/workflows/s/stream")
            .expect("build")
            .build()
            .expect("request");
        assert!(
            stream.timeout().is_none(),
            "a stream must not carry a total timeout"
        );
    }

    // -- transport ----------------------------------------------------------

    #[tokio::test]
    async fn unauthorized_maps_to_an_auth_error_and_sends_the_bearer() {
        let server = FakeServer::start(|_| {
            http_response(
                "401 Unauthorized",
                r#"{"error":{"code":"unauthorized","message":"Missing or invalid bearer token"}}"#,
            )
        })
        .await;
        let document = discovery_on(server.addr.port(), "instance-a", 42);
        let client = RuntimeClient::new(&document).expect("client");
        let error = client.get(META_PATH).await.expect_err("401 must fail");
        assert!(
            matches!(error, ClientError::Auth(_)),
            "unexpected {error:?}"
        );

        let requests = server.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].authorization.as_deref(),
            Some("Bearer test-token")
        );
    }

    #[tokio::test]
    async fn redirects_are_not_followed_and_never_forward_the_bearer() {
        let target = FakeServer::start(|_| http_response("200 OK", "{}")).await;
        let location = format!("http://127.0.0.1:{}/control/v1/meta", target.addr.port());
        let redirect = FakeServer::start(move |_| {
            format!(
                "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .into_bytes()
        })
        .await;

        let document = discovery_on(redirect.addr.port(), "instance-a", 42);
        let client = RuntimeClient::new(&document).expect("client");
        let error = client
            .get(META_PATH)
            .await
            .expect_err("a redirect must not be followed");
        assert!(
            matches!(error, ClientError::Server { status: 302, .. }),
            "unexpected {error:?}"
        );
        assert!(
            target.requests().is_empty(),
            "the bearer token must never reach the redirect target"
        );
    }

    // -- leases -------------------------------------------------------------

    #[tokio::test]
    async fn lease_register_renew_and_release_use_the_runtime_routes() {
        let server =
            FakeServer::start(
                |request| match (request.method.as_str(), request.path.as_str()) {
                    ("POST", "/control/v1/clients/register") => http_response(
                        "200 OK",
                        &json!({
                            "client_id": "cscli-1",
                            "lease_id": "lease-1",
                            "expires_at": "2026-01-01T00:00:10Z",
                        })
                        .to_string(),
                    ),
                    ("POST", "/control/v1/clients/cscli-1/renew") => http_response(
                        "200 OK",
                        &json!({
                            "client_id": "cscli-1",
                            "lease_id": "lease-1",
                            "expires_at": "2026-01-01T00:00:20Z",
                        })
                        .to_string(),
                    ),
                    ("POST", "/control/v1/clients/cscli-1/release") => {
                        b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_vec()
                    }
                    _ => http_response(
                        "404 Not Found",
                        r#"{"error":{"code":"not_found","message":"no such route"}}"#,
                    ),
                },
            )
            .await;

        let document = discovery_on(server.addr.port(), "instance-a", 42);
        let client = RuntimeClient::new(&document).expect("client");
        let mut guard = LeaseGuard::register(&client, "cscli-1", "cscli")
            .await
            .expect("register lease");
        assert_eq!(guard.lease_id(), "lease-1");
        assert_eq!(guard.expires_at(), "2026-01-01T00:00:10Z");

        guard.renew().await.expect("renew lease");
        assert_eq!(guard.expires_at(), "2026-01-01T00:00:20Z");

        guard.release().await.expect("release lease");
        assert!(guard.is_released());

        // Releasing twice is a local no-op: no further request is sent.
        let before = server.requests().len();
        guard.release().await.expect("second release is a no-op");
        assert_eq!(server.requests().len(), before);
    }

    #[tokio::test]
    async fn a_missing_lease_maps_to_a_structured_server_error() {
        let server = FakeServer::start(|_| {
            http_response(
                "404 Not Found",
                r#"{"error":{"code":"not_found","message":"No active lease for client"}}"#,
            )
        })
        .await;
        let document = discovery_on(server.addr.port(), "instance-a", 42);
        let client = RuntimeClient::new(&document).expect("client");
        let mut guard = LeaseGuard::from_response(
            client,
            "cscli",
            ClientLeaseResponse {
                client_id: "cscli-1".to_string(),
                lease_id: "lease-1".to_string(),
                expires_at: "2026-01-01T00:00:00Z".to_string(),
            },
        );
        let error = guard.renew().await.expect_err("a missing lease must fail");
        match error {
            ClientError::Server { status, code, .. } => {
                assert_eq!(status, 404);
                assert_eq!(code, "not_found");
            }
            other => panic!("expected a server error, got {other:?}"),
        }
    }

    fn lease_at(port: u16) -> LeaseGuard {
        let document = discovery_on(port, "instance-a", 42);
        let client = RuntimeClient::new(&document).expect("client");
        LeaseGuard::from_response(
            client,
            "cscli",
            ClientLeaseResponse {
                client_id: "cscli-1".to_string(),
                lease_id: "lease-1".to_string(),
                expires_at: "2026-01-01T00:00:00Z".to_string(),
            },
        )
    }

    #[tokio::test]
    async fn heartbeat_renews_then_stops_without_racing_release() {
        let renews = Arc::new(AtomicUsize::new(0));
        let counter = renews.clone();
        let server = FakeServer::start(move |request| {
            if request.path.ends_with("/renew") {
                counter.fetch_add(1, Ordering::SeqCst);
            }
            http_response(
                "200 OK",
                &json!({
                    "client_id": "cscli-1",
                    "lease_id": "lease-1",
                    "expires_at": "2026-01-01T00:00:10Z",
                })
                .to_string(),
            )
        })
        .await;

        let lease = lease_at(server.addr.port());
        let heartbeat = lease.heartbeat_with_interval(Duration::from_millis(30));
        tokio::time::timeout(Duration::from_secs(5), async {
            while renews.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the heartbeat must renew at least once");
        assert_eq!(heartbeat.status(), Ok(()));

        heartbeat.stop().await;
        let after_stop = renews.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            renews.load(Ordering::SeqCst),
            after_stop,
            "stop() must join the task so no renewal races the release"
        );
    }

    #[tokio::test]
    async fn heartbeat_failure_is_reported() {
        let server = FakeServer::start(|_| {
            http_response(
                "401 Unauthorized",
                r#"{"error":{"code":"unauthorized","message":"lease expired"}}"#,
            )
        })
        .await;
        let lease = lease_at(server.addr.port());
        let mut heartbeat = lease.heartbeat_with_interval(Duration::from_millis(20));
        let failure = tokio::time::timeout(Duration::from_secs(5), heartbeat.failure())
            .await
            .expect("a failure must be reported")
            .expect("a failure message");
        assert!(failure.contains("lease expired"), "unexpected {failure}");
        heartbeat.stop().await;
    }

    #[test]
    fn heartbeat_interval_follows_the_configured_lease_ttl() {
        let _guard = crate::ENV_LOCK.lock().expect("env lock");
        let original_heartbeat = std::env::var_os(HEARTBEAT_MS_ENV);
        let original_ttl = std::env::var_os(LEASE_TTL_ENV);
        std::env::remove_var(HEARTBEAT_MS_ENV);

        let lease = LeaseGuard::from_response(client(), "cscli", lease_response());
        std::env::set_var(LEASE_TTL_ENV, "300");
        assert_eq!(lease.renew_interval(), Duration::from_millis(100));

        std::env::set_var(HEARTBEAT_MS_ENV, "50");
        assert_eq!(lease.renew_interval(), Duration::from_millis(50));

        // A tiny TTL is still clamped above zero so it renews in time.
        std::env::remove_var(HEARTBEAT_MS_ENV);
        std::env::set_var(LEASE_TTL_ENV, "10");
        assert_eq!(lease.renew_interval(), MIN_HEARTBEAT_INTERVAL);

        restore_env(HEARTBEAT_MS_ENV, original_heartbeat);
        restore_env(LEASE_TTL_ENV, original_ttl);
    }

    // -- discovery lifecycle and spawn -------------------------------------

    #[test]
    fn discovery_absent_only_for_a_missing_document() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join(DISCOVERY_FILE_NAME);
        assert!(discovery_absent(&missing));

        std::fs::write(&missing, b"not json").expect("write");
        assert!(!discovery_absent(&missing));

        // A directory is not absent even though reading it fails.
        let directory = dir.path().join("as-directory");
        std::fs::create_dir(&directory).expect("mkdir");
        assert!(!discovery_absent(&directory));
    }

    #[test]
    fn spawn_refuses_when_a_discovery_document_already_exists() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(discovery_file_in(dir.path()), b"{}").expect("write");
        let error = spawn_runtime(dir.path()).expect_err("must refuse");
        assert!(matches!(error, ClientError::Discovery(_)));
        assert!(error
            .to_string()
            .contains("refusing to start a second runtime"));
    }

    #[cfg(unix)]
    #[test]
    fn spawn_uses_the_explicit_binary_and_reaps_its_child() {
        use std::os::unix::fs::PermissionsExt;

        let _guard = crate::ENV_LOCK.lock().expect("env lock");
        let dir = tempfile::tempdir().expect("tempdir");
        let script = dir.path().join("fake-runtime");
        std::fs::write(
            &script,
            b"#!/bin/sh\necho 'runtime diagnostic' >&2\nexit 3\n",
        )
        .expect("write script");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .expect("chmod script");

        let original = std::env::var_os(RUNTIME_BINARY_ENV);
        std::env::set_var(RUNTIME_BINARY_ENV, &script);

        let mut child = spawn_runtime(dir.path()).expect("spawn");
        assert!(child.id() > 0);
        let status = loop {
            if let Some(status) = child.try_wait().expect("try_wait") {
                break status;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(!status.success());
        assert_eq!(
            child.diagnostics().as_deref(),
            Some("runtime diagnostic"),
            "startup stderr must be available for diagnostics"
        );

        restore_env(RUNTIME_BINARY_ENV, original);
        // Dropping the handle after reaping must stay panic-free.
        drop(child);
    }

    #[cfg(unix)]
    #[test]
    fn typed_spawn_passes_every_path_and_clears_the_legacy_home() {
        use std::os::unix::fs::PermissionsExt;

        let _guard = crate::ENV_LOCK.lock().expect("env lock");
        let dir = tempfile::tempdir().expect("tempdir");
        let dump = dir.path().join("env-dump");
        let script = dir.path().join("fake-runtime");
        std::fs::write(&script, format!("#!/bin/sh\nenv > '{}'\n", dump.display()))
            .expect("write script");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .expect("chmod script");

        let original_bin = std::env::var_os(RUNTIME_BINARY_ENV);
        let original_home = std::env::var_os(LEGACY_HOME_ENV);
        std::env::set_var(RUNTIME_BINARY_ENV, &script);
        // An inherited legacy home must not survive into the child.
        std::env::set_var(LEGACY_HOME_ENV, "/inherited-home");

        let config = RuntimeLaunchConfig::new(
            "/typed/runtime",
            "/typed/profile/chatspeed.db",
            "/typed/app-data",
        );
        let mut child = spawn_runtime_with_config(&config).expect("spawn");
        loop {
            if child.try_wait().expect("try_wait").is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let env_dump = std::fs::read_to_string(&dump).expect("env dump");

        assert!(
            env_dump
                .lines()
                .any(|line| line == "CHATSPEED_RUNTIME_DIR=/typed/runtime"),
            "{env_dump}"
        );
        assert!(
            env_dump
                .lines()
                .any(|line| line == "CHATSPEED_RUNTIME_DB=/typed/profile/chatspeed.db"),
            "{env_dump}"
        );
        assert!(
            env_dump
                .lines()
                .any(|line| line == "CHATSPEED_RUNTIME_APP_DATA_DIR=/typed/app-data"),
            "{env_dump}"
        );
        assert!(
            !env_dump
                .lines()
                .any(|line| line.starts_with("CHATSPEED_HOME=")),
            "the legacy home must be cleared: {env_dump}"
        );

        restore_env(RUNTIME_BINARY_ENV, original_bin);
        restore_env(LEGACY_HOME_ENV, original_home);
        drop(child);
    }

    // -- typed chat / model surface ----------------------------------------

    fn sse_response(body: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    fn sample_model() -> ModelDetailsDto {
        ModelDetailsDto {
            id: "gpt-4".to_string(),
            name: "GPT-4".to_string(),
            protocol: ChatProtocolDto::OpenAI,
            max_input_tokens: Some(128_000),
            max_output_tokens: None,
            description: None,
            last_updated: None,
            family: None,
            reasoning: Some(true),
            function_call: Some(true),
            image_input: None,
            recommended_temperature: None,
            metadata: None,
        }
    }

    fn sample_response(sequence: u64, event: ChatStreamEvent) -> ChatStreamEnvelope {
        ChatStreamEnvelope::new("chat-1", sequence, event)
    }

    #[tokio::test]
    async fn model_listing_uses_the_typed_snake_case_route() {
        let model = sample_model();
        let body = serde_json::to_string(&vec![model.clone()]).expect("encode models");
        let server = FakeServer::start(move |_| http_response("200 OK", &body)).await;
        let client =
            RuntimeClient::new(&discovery_on(server.addr.port(), "instance-a", 1)).expect("client");

        let request = ListModelsRequest {
            api_protocol: "openai".to_string(),
            api_url: Some("https://example.test/v1".to_string()),
            api_key: Some("secret".to_string()),
            metadata: Some(json!({"proxyType": "bySetting"})),
        };
        let models = client.list_models(&request).await.expect("list models");
        assert_eq!(models, vec![model]);

        let recorded = server.requests();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].method, "POST");
        assert_eq!(recorded[0].path, MODELS_LIST_PATH);
        assert_eq!(
            recorded[0].authorization.as_deref(),
            Some("Bearer test-token")
        );
        let sent: serde_json::Value =
            serde_json::from_str(&recorded[0].body).expect("request body");
        assert_eq!(sent["api_protocol"], json!("openai"));
        assert_eq!(sent["api_key"], json!("secret"));
        assert_eq!(sent["metadata"]["proxyType"], json!("bySetting"));
    }

    #[tokio::test]
    async fn chat_start_and_stop_use_the_documented_routes() {
        let server = FakeServer::start(|request| {
            if request.path.ends_with("/start") {
                http_response("200 OK", r#"{"chat_id":"chat-1","accepted":true}"#)
            } else {
                http_response("200 OK", r#"{"chat_id":"chat-1","stopped":true}"#)
            }
        })
        .await;
        let client =
            RuntimeClient::new(&discovery_on(server.addr.port(), "instance-a", 1)).expect("client");

        let start = ChatStartRequest {
            provider_id: 7,
            model: "gpt-4".to_string(),
            chat_id: "chat-1".to_string(),
            messages: vec![json!({"role": "user", "content": "hi"})],
            network_enabled: Some(false),
            mcp_enabled: None,
            metadata: None,
        };
        let started = client
            .start_chat("chat-1", &start)
            .await
            .expect("start chat");
        assert!(started.accepted);

        let stopped = client
            .stop_chat(
                "chat-1",
                &ChatStopRequest {
                    chat_id: "chat-1".to_string(),
                    api_protocol: Some("openai".to_string()),
                },
            )
            .await
            .expect("stop chat");
        assert!(stopped.stopped);

        let paths: Vec<String> = server
            .requests()
            .into_iter()
            .map(|request| request.path)
            .collect();
        assert_eq!(
            paths,
            vec![chat_start_path("chat-1"), chat_stop_path("chat-1")]
        );
    }

    #[tokio::test]
    async fn chat_stream_decodes_typed_envelopes_and_skips_keepalives() {
        let text = sample_response(
            0,
            ChatStreamEvent::Response {
                response: ChatResponseDto {
                    chat_id: "chat-1".to_string(),
                    chunk: "hel".to_string(),
                    r#type: MessageTypeDto::Text,
                    metadata: Some(json!({"windowLabel": "main"})),
                    finish_reason: None,
                },
            },
        );
        let reasoning = sample_response(
            1,
            ChatStreamEvent::Response {
                response: ChatResponseDto {
                    chat_id: "chat-1".to_string(),
                    chunk: "lo".to_string(),
                    r#type: MessageTypeDto::Reasoning,
                    metadata: None,
                    finish_reason: None,
                },
            },
        );
        let finished = sample_response(
            2,
            ChatStreamEvent::Finished {
                finish_reason: Some(FinishReasonDto::Stop),
            },
        );
        let body = format!(
            ": keepalive\n\nid: 0\ndata: {}\n\ndata: {}\n\ndata: {}\n\n",
            serde_json::to_string(&text).expect("text"),
            serde_json::to_string(&reasoning).expect("reasoning"),
            serde_json::to_string(&finished).expect("finished"),
        );
        let server = FakeServer::start(move |_| sse_response(&body)).await;
        let client =
            RuntimeClient::new(&discovery_on(server.addr.port(), "instance-a", 1)).expect("client");

        let mut stream = client.stream_chat("chat-1").await.expect("open stream");
        let mut events = Vec::new();
        while let Some(event) = stream.next_event().await.expect("event") {
            events.push(event);
        }

        assert_eq!(events.len(), 3);
        assert_eq!(events[0].sequence, 0);
        assert_eq!(events[1].sequence, 1);
        assert_eq!(
            events[2].event,
            ChatStreamEvent::Finished {
                finish_reason: Some(FinishReasonDto::Stop),
            }
        );
        assert!(events[2].is_terminal());

        let recorded = server.requests();
        assert_eq!(recorded[0].method, "GET");
        assert_eq!(recorded[0].path, chat_events_path("chat-1"));
        assert_eq!(
            recorded[0].authorization.as_deref(),
            Some("Bearer test-token")
        );
    }

    #[tokio::test]
    async fn chat_stream_reports_a_structured_error_for_a_non_success_response() {
        let server = FakeServer::start(|_| {
            http_response(
                "503 Service Unavailable",
                r#"{"error":{"code":"runtime_unavailable","message":"no chat owner"}}"#,
            )
        })
        .await;
        let client =
            RuntimeClient::new(&discovery_on(server.addr.port(), "instance-a", 1)).expect("client");

        let error = client.stream_chat("chat-1").await.expect_err("must reject");
        match error {
            ClientError::Server { status, code, .. } => {
                assert_eq!(status, 503);
                assert_eq!(code, "runtime_unavailable");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn chat_stream_reports_a_malformed_envelope() {
        let body = String::from("data: not-json\n\n");
        let server = FakeServer::start(move |_| sse_response(&body)).await;
        let client =
            RuntimeClient::new(&discovery_on(server.addr.port(), "instance-a", 1)).expect("client");

        let mut stream = client.stream_chat("chat-1").await.expect("open stream");
        let error = stream.next_event().await.expect_err("must reject");
        assert!(matches!(error, ClientError::Serialization(_)));
    }

    // -- workflow stream ---------------------------------------------------

    #[tokio::test]
    async fn workflow_stream_parses_envelopes_and_resumes_from_a_cursor() {
        let chunk = json!({
            "schema_version": 1,
            "server_instance_id": "instance-a",
            "sequence": 7,
            "session_id": "session-1",
            "payload": { "type": "chunk", "content": "hello" },
        });
        let state = json!({
            "schema_version": 1,
            "server_instance_id": "instance-a",
            "sequence": 8,
            "session_id": "session-1",
            "payload": { "type": "state", "state": "completed" },
        });
        // A comment keepalive and an `id`-only frame must be skipped.
        let body = format!(
            ": keepalive\n\nid: 7\ndata: {}\n\ndata: {}\n\n",
            serde_json::to_string(&chunk).expect("chunk"),
            serde_json::to_string(&state).expect("state"),
        );
        let server = FakeServer::start(move |_| sse_response(&body)).await;
        let client =
            RuntimeClient::new(&discovery_on(server.addr.port(), "instance-a", 1)).expect("client");

        let mut stream = client
            .stream_workflow("session-1", Some("instance-a:6"))
            .await
            .expect("open stream");

        let first = stream
            .next_event()
            .await
            .expect("event")
            .expect("first event");
        assert_eq!(first.sequence, 7);
        assert_eq!(first.payload["type"].as_str(), Some("chunk"));
        assert_eq!(first.payload["content"].as_str(), Some("hello"));
        assert_eq!(stream_envelope_cursor(&first), "instance-a:7");

        let second = stream
            .next_event()
            .await
            .expect("event")
            .expect("second event");
        assert_eq!(second.sequence, 8);
        assert_eq!(second.payload["state"].as_str(), Some("completed"));

        assert!(stream.next_event().await.expect("end").is_none());

        let recorded = server.requests();
        assert_eq!(recorded[0].method, "GET");
        assert_eq!(recorded[0].path, workflow_stream_path("session-1"));
        assert_eq!(
            recorded[0].authorization.as_deref(),
            Some("Bearer test-token")
        );
        assert_eq!(recorded[0].last_event_id.as_deref(), Some("instance-a:6"));
    }

    #[tokio::test]
    async fn workflow_stream_reports_a_malformed_envelope() {
        let body = String::from("data: not-json\n\n");
        let server = FakeServer::start(move |_| sse_response(&body)).await;
        let client =
            RuntimeClient::new(&discovery_on(server.addr.port(), "instance-a", 1)).expect("client");

        let mut stream = client
            .stream_workflow("session-1", None)
            .await
            .expect("open stream");
        let error = stream.next_event().await.expect_err("must reject");
        assert!(matches!(error, ClientError::Serialization(_)));
    }

    // -- client bridge helpers ---------------------------------------------

    fn bridge_declaration() -> ClientBridgeDeclaration {
        ClientBridgeDeclaration {
            protocol_version: BRIDGE_PROTOCOL_VERSION.to_string(),
            schema_version: BRIDGE_SCHEMA_VERSION.to_string(),
            capabilities: vec![ClientBridgeCapability {
                name: "web_fetch".to_string(),
                schema_version: BRIDGE_SCHEMA_VERSION.to_string(),
            }],
        }
    }

    #[tokio::test]
    async fn register_bridge_posts_the_typed_body_with_the_bearer() {
        let response_body = json!({
            "session_id": "session-1",
            "session_token": "opaque-token",
            "expires_at": "2026-01-01T00:00:00Z",
            "protocol_version": "1",
            "declaration_fingerprint": "abc"
        })
        .to_string();
        let server = FakeServer::start(move |_| http_response("200 OK", &response_body)).await;
        let client =
            RuntimeClient::new(&discovery_on(server.addr.port(), "instance-a", 1)).expect("client");

        let session = client
            .register_bridge(&bridge_declaration(), "tauri-main", "lease-1")
            .await
            .expect("register");
        assert_eq!(session.session_id, "session-1");

        let recorded = server.requests();
        assert_eq!(recorded[0].method, "POST");
        assert_eq!(recorded[0].path, CLIENT_BRIDGE_REGISTER_PATH);
        assert_eq!(
            recorded[0].authorization.as_deref(),
            Some("Bearer test-token")
        );
        assert!(recorded[0].body.contains("\"client_id\":\"tauri-main\""));
        assert!(recorded[0].body.contains("\"lease_id\":\"lease-1\""));
        // The returned secret is never echoed back into a request body.
        assert!(!recorded[0].body.contains("opaque-token"));
    }

    #[tokio::test]
    async fn bridge_stream_sends_the_session_header_and_parses_envelopes() {
        let body = format!(
            "data: {}\n\n",
            json!({
                "schema_version": "1",
                "invocation": {
                    "request_id": "req-1",
                    "capability": "web_fetch",
                    "schema_version": "1",
                    "arguments": {"url": "https://example.com"},
                    "deadline": "unix-100"
                }
            })
        );
        let server = FakeServer::start(move |_| sse_response(&body)).await;
        let client =
            RuntimeClient::new(&discovery_on(server.addr.port(), "instance-a", 1)).expect("client");
        let session = BridgeSession::new(client, "session-1", "opaque-token");

        let mut stream = session.events().await.expect("open bridge stream");
        let envelope: ClientBridgeWorkEnvelope = stream
            .next_event()
            .await
            .expect("read")
            .expect("one envelope");
        assert_eq!(envelope.invocation.capability, "web_fetch");
        assert_eq!(
            envelope.invocation.arguments["url"],
            json!("https://example.com")
        );

        let recorded = server.requests();
        assert_eq!(recorded[0].path, bridge_events_path("session-1"));
        assert_eq!(recorded[0].bridge_session.as_deref(), Some("opaque-token"));
        assert!(!recorded[0].path.contains("opaque-token"));
    }

    #[tokio::test]
    async fn bridge_result_cancel_and_unregister_all_use_the_session_header() {
        let server = FakeServer::start(|_| http_response("204 No Content", "")).await;
        let client =
            RuntimeClient::new(&discovery_on(server.addr.port(), "instance-a", 1)).expect("client");
        let session = BridgeSession::new(client, "session-1", "opaque-token");

        session
            .submit_result(&ClientCapabilityResult {
                request_id: "req-1".to_string(),
                status: ClientCapabilityStatus::Ok,
                result: Some(json!({"content": "x"})),
                error: None,
            })
            .await
            .expect("submit result");
        session.cancel("req-1").await.expect("cancel");
        session.unregister().await.expect("unregister");

        let recorded = server.requests();
        assert_eq!(recorded[0].path, bridge_result_path("session-1"));
        assert_eq!(recorded[1].path, bridge_cancel_path("session-1"));
        assert_eq!(recorded[2].path, bridge_unregister_path("session-1"));
        for request in &recorded {
            assert_eq!(request.bridge_session.as_deref(), Some("opaque-token"));
            assert!(!request.body.contains("opaque-token"));
        }
    }

    // -- runtime terminal client -------------------------------------------

    #[tokio::test]
    async fn terminal_calls_carry_the_lease_proof_in_headers_not_the_url() {
        let response_body =
            json!([{ "name": "bash", "path": "/bin/bash", "is_default": true }]).to_string();
        let server = FakeServer::start(move |_| http_response("200 OK", &response_body)).await;
        let client =
            RuntimeClient::new(&discovery_on(server.addr.port(), "instance-a", 1)).expect("client");

        let shells = client
            .terminal_list_shells("tauri-main", "lease-1")
            .await
            .expect("shells");
        assert_eq!(shells.len(), 1);
        assert_eq!(shells[0].path, "/bin/bash");

        let recorded = server.requests();
        assert_eq!(recorded[0].method, "GET");
        assert_eq!(recorded[0].path, TERMINAL_SHELLS_PATH);
        assert_eq!(recorded[0].terminal_client.as_deref(), Some("tauri-main"));
        assert_eq!(recorded[0].terminal_lease.as_deref(), Some("lease-1"));
        assert_eq!(
            recorded[0].authorization.as_deref(),
            Some("Bearer test-token")
        );
        // The lease proof never appears in the URL or the request body.
        assert!(!recorded[0].path.contains("lease-1"));
        assert!(!recorded[0].body.contains("lease-1"));
    }

    #[tokio::test]
    async fn terminal_create_write_resize_and_close_use_typed_bodies_and_paths() {
        let metadata = json!({
            "session_id": "session-1",
            "shell_name": "bash",
            "shell_path": "/bin/bash",
            "cwd": "/workspace",
            "alive": true,
        })
        .to_string();
        let server = FakeServer::start(move |request| {
            if request.path == TERMINAL_CREATE_PATH {
                http_response("200 OK", &metadata)
            } else {
                http_response("204 No Content", "")
            }
        })
        .await;
        let client =
            RuntimeClient::new(&discovery_on(server.addr.port(), "instance-a", 1)).expect("client");

        let session = client
            .terminal_create(
                "tauri-main",
                "lease-1",
                &TerminalCreateRequest {
                    cwd: Some("/workspace".to_string()),
                    shell_path: None,
                    cols: Some(120),
                    rows: None,
                },
            )
            .await
            .expect("create");
        assert_eq!(session.session_id, "session-1");

        client
            .terminal_write(
                "tauri-main",
                "lease-1",
                &session.session_id,
                &TerminalWriteRequest {
                    input: "ls\n".to_string(),
                },
            )
            .await
            .expect("write");
        client
            .terminal_resize(
                "tauri-main",
                "lease-1",
                &session.session_id,
                &TerminalResizeRequest { cols: 80, rows: 24 },
            )
            .await
            .expect("resize");
        client
            .terminal_close("tauri-main", "lease-1", &session.session_id)
            .await
            .expect("close");

        let recorded = server.requests();
        assert_eq!(recorded[0].path, TERMINAL_CREATE_PATH);
        assert!(recorded[0].body.contains("\"cwd\":\"/workspace\""));
        assert_eq!(recorded[1].path, terminal_write_path("session-1"));
        assert!(recorded[1].body.contains("\"input\""));
        assert!(recorded[1].body.contains("ls"));
        assert_eq!(recorded[2].path, terminal_resize_path("session-1"));
        assert_eq!(recorded[3].path, terminal_close_path("session-1"));
        for request in &recorded {
            assert_eq!(request.terminal_client.as_deref(), Some("tauri-main"));
            assert_eq!(request.terminal_lease.as_deref(), Some("lease-1"));
        }
    }

    #[tokio::test]
    async fn terminal_stream_decodes_typed_envelopes_and_a_terminal_reset() {
        let output = TerminalStreamEnvelope::new(
            "session-1",
            0,
            TerminalStreamEvent::Output(TerminalOutputEvent {
                session_id: "session-1".to_string(),
                data_base64: "aGVsbG8=".to_string(),
            }),
        );
        let reset = TerminalStreamEnvelope::new(
            "session-1",
            1,
            TerminalStreamEvent::Reset {
                reason: "stream_lagged".to_string(),
            },
        );
        let body = format!(
            ": keepalive\n\nid: 0\ndata: {}\n\ndata: {}\n\n",
            serde_json::to_string(&output).expect("output"),
            serde_json::to_string(&reset).expect("reset"),
        );
        let server = FakeServer::start(move |_| sse_response(&body)).await;
        let client =
            RuntimeClient::new(&discovery_on(server.addr.port(), "instance-a", 1)).expect("client");

        let mut stream = client
            .terminal_stream("tauri-main", "lease-1", "session-1")
            .await
            .expect("open stream");
        let first = stream
            .next_event()
            .await
            .expect("read")
            .expect("output envelope");
        assert_eq!(
            first.event,
            TerminalStreamEvent::Output(TerminalOutputEvent {
                session_id: "session-1".to_string(),
                data_base64: "aGVsbG8=".to_string(),
            })
        );
        let terminal = stream
            .next_event()
            .await
            .expect("read")
            .expect("reset envelope");
        assert!(terminal.is_terminal());
        assert_eq!(
            terminal.event,
            TerminalStreamEvent::Reset {
                reason: "stream_lagged".to_string(),
            }
        );

        let recorded = server.requests();
        assert_eq!(recorded[0].path, terminal_stream_path("session-1"));
        assert_eq!(recorded[0].terminal_client.as_deref(), Some("tauri-main"));
        assert_eq!(recorded[0].terminal_lease.as_deref(), Some("lease-1"));
    }

    #[tokio::test]
    async fn web_provider_registration_proves_identity_in_headers_only() {
        let response_body = json!({
            "server_name": chatspeed_contracts::WEB_MCP_SERVER_NAME,
            "generation": 2,
            "expires_at": "unix-100",
        })
        .to_string();
        let server = FakeServer::start(move |_| http_response("200 OK", &response_body)).await;
        let client =
            RuntimeClient::new(&discovery_on(server.addr.port(), "instance-a", 1)).expect("client");

        let response = client
            .register_web_mcp_provider(
                &WebMcpProviderRegistration { port: 41234 },
                "tauri-main",
                "lease-1",
                "desktop-instance",
                "provider-proof",
            )
            .await
            .expect("register");
        assert_eq!(response.server_name, chatspeed_contracts::WEB_MCP_SERVER_NAME);
        assert_eq!(response.generation, 2);

        client
            .unregister_web_mcp_provider(
                "tauri-main",
                "lease-1",
                "desktop-instance",
                "provider-proof",
            )
            .await
            .expect("unregister");

        let recorded = server.requests();
        assert_eq!(recorded[0].method, "POST");
        assert_eq!(recorded[0].path, WEB_MCP_REGISTER_PATH);
        // Only the loopback port is in the body.
        assert_eq!(recorded[0].body, "{\"port\":41234}");
        assert_eq!(recorded[1].path, WEB_MCP_UNREGISTER_PATH);
        for request in &recorded {
            assert_eq!(request.web_proof.as_deref(), Some("provider-proof"));
            assert_eq!(request.web_client.as_deref(), Some("tauri-main"));
            assert_eq!(request.web_lease.as_deref(), Some("lease-1"));
            assert_eq!(request.web_instance.as_deref(), Some("desktop-instance"));
            assert_eq!(
                request.authorization.as_deref(),
                Some("Bearer test-token")
            );
            // The proof never appears in the URL or the body.
            assert!(!request.path.contains("provider-proof"));
            assert!(!request.body.contains("provider-proof"));
        }
    }

    #[tokio::test]
    async fn web_provider_status_reads_an_absent_slot_as_none() {
        let server = FakeServer::start(move |_| http_response("404 Not Found", "{}")).await;
        let client =
            RuntimeClient::new(&discovery_on(server.addr.port(), "instance-a", 1)).expect("client");
        assert!(client
            .web_mcp_provider_status()
            .await
            .expect("status")
            .is_none());
        assert_eq!(server.requests()[0].path, WEB_MCP_STATUS_PATH);
    }

    #[test]
    fn bridge_session_debug_redacts_the_token() {
        let session = BridgeSession::new(client(), "session-1", "opaque-token");
        let debug = format!("{session:?}");
        assert!(!debug.contains("opaque-token"), "{debug}");
        assert!(debug.contains("<redacted>"), "{debug}");
    }
}
