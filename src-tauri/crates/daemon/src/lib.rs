//! Headless ChatSpeed runtime (U-3).
//!
//! This crate is the standalone `chatspeed-runtime` target: it runs without
//! Tauri, Wry, GTK or the desktop application crate, owns the runtime directory
//! lock, and serves the single canonical `/control/v1` HTTP/JSON + SSE control
//! plane through the shared [`chatspeed_runtime_backend`] modules.
//!
//! What this unit now owns end to end:
//! - instance identity and `${CHATSPEED_HOME:-~/.chatspeed}/runtime` resolution;
//! - a single-instance runtime-directory lock (`create_new` + owner-fenced
//!   cleanup) whose stale takeover fails closed when staleness cannot be proven;
//! - exactly one canonical backend owner (database, chat/tool state, workflow
//!   hub, session manager, sub-agent factory, application service) assembled by
//!   [`RuntimeOwner`], with a persistent database file and no `:memory:` path;
//! - the canonical workflow, capability, automation and SSE routes plus the
//!   client-lease lifecycle (`register`/`renew`/`release`) on that one server;
//! - instance-fenced discovery publication and cleanup, and an idle-grace
//!   supervisor that shuts the instance down once the last lease is gone.
//!
//! The runtime-directory lock is held until the canonical HTTP server has fully
//! stopped, so a successor instance can never observe a half-stopped owner.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chatspeed_contracts::{
    ClientLease, ClientLeaseRequest, ClientLeaseResponse, RuntimeAutomationHealth,
    RuntimeAutomationSummaryDto, RuntimeLifecyclePhase, RuntimeOperationDto, RuntimeOperationStage,
    RuntimeShutdownAcceptedDto, RuntimeShutdownOutcome, RuntimeShutdownRequest, RuntimeStatusDto,
    PROTOCOL_VERSION, RUNTIME_LIFECYCLE_SCHEMA_VERSION,
};
use chatspeed_runtime_backend::background::RuntimeBackground;
use chatspeed_runtime_backend::error_diagnostics::{log_error_diagnostic, ErrorProjection};
use chatspeed_runtime_backend::lifecycle::{
    AdmissionTicket, RuntimeLifecycleGate, ShutdownHandoff, WorkAdmission, WorkKind,
};
use chatspeed_runtime_backend::owner::{RuntimeOwner, RuntimeOwnerConfig};
use chatspeed_runtime_backend::web_provider::{ToolManagerProviderInstaller, WebProviderRegistry};
use chatspeed_runtime_backend::workflow::automation::service::normalize_datetime_for_db;
use chatspeed_runtime_backend::workflow::automation::types::{
    AutomationRuntimeHealth, AutomationRuntimeSummary,
};
use chatspeed_runtime_backend::workflow::react::application::WorkflowApplicationService;
use chatspeed_runtime_backend::workflow::react::client::http::server::{
    self, RuntimeControlPlane, RuntimeControlPlaneOptions, RuntimeLeaseError, RuntimeShutdownError,
    RuntimeWebMcpPlane,
};
use rand::Rng;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

/// Database file name inside the runtime-owned application-data directory.
///
/// Shared with the client resolver so a spawn and the runtime binary it starts
/// never disagree about the file name.
pub use chatspeed_runtime_client::DB_FILE_NAME;

/// Service name the standalone runtime reports on `GET /control/v1/meta`.
///
/// A client verifies this value before registering a lease, so the runtime must
/// report its own identity rather than the desktop in-process control plane's.
pub const SERVICE_NAME: &str = "chatspeed-runtime";
/// Runtime-directory lock file name.
pub const LOCK_FILE_NAME: &str = "runtime.lock";
/// Suffix appended to the database file name for the DB-authority sidecar lock.
const DB_LOCK_SUFFIX: &str = ".authority.lock";
/// Idle grace period applied when no client lease is active.
pub const DEFAULT_GRACE: Duration = Duration::from_secs(30);
/// Default lease TTL.
pub const DEFAULT_LEASE_TTL: Duration = Duration::from_secs(60);

/// Longest accepted `client_id`; bounding it keeps the in-memory lease map from
/// being grown by an unauthenticated-shaped payload.
const MAX_CLIENT_ID_LEN: usize = 128;
/// Longest accepted `client_kind`.
const MAX_CLIENT_KIND_LEN: usize = 64;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Runtime lifecycle and I/O failures.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("runtime I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid configuration: {0}")]
    Config(String),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("another runtime instance already owns {dir} (pid {pid})")]
    AlreadyRunning { dir: PathBuf, pid: u32 },
    #[error("cannot verify the existing lock owner in {path}; refusing to take over")]
    LockUnverifiable { path: PathBuf },
    #[error("lock file {path} is unreadable or malformed; refusing to take over")]
    LockMalformed { path: PathBuf },
    #[error("failed to assemble the runtime owner: {0}")]
    Assembly(String),
    #[error("failed to start the control plane: {0}")]
    ControlPlane(String),
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Runtime launch configuration.
///
/// The three paths are resolved by the shared, Tauri-free
/// [`chatspeed_runtime_client::resolve_launch_config`] rule so the runtime
/// binary, the desktop and `cscli` can never disagree: the runtime directory
/// owns the lock and discovery document, the database file is the persistence
/// authority, and the application-data directory holds server-owned storage.
/// Each path has a `with_*` override for tests and explicit sandboxes. There is
/// no `:memory:` database option.
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    runtime_dir: PathBuf,
    db_path: PathBuf,
    app_data_dir: PathBuf,
    grace: Duration,
    lease_ttl: Duration,
    /// Total budget shared by the cancelling, draining and stop phases.
    shutdown_budget: Duration,
    /// Extra budget granted only to an explicitly forced shutdown.
    force_budget: Duration,
    /// The injectable last-resort termination fallback. Defaults to the log-only
    /// library hook so embedding the daemon never kills the host; the real binary
    /// installs [`ProcessExitTerminationHook`].
    termination: Arc<dyn TerminationHook>,
}

impl RuntimeConfig {
    /// Builds the build-profile default configuration.
    ///
    /// Production resolves to `<platform data dir>/ai.aidyou.chatspeed` and
    /// development to the repository's `dev_data`, exactly as
    /// [`chatspeed_runtime_client::resolve_launch_config`] decides. It fails
    /// closed when no platform directory is available; there is no
    /// `.`-relative fallback. A caller that already owns a runtime directory
    /// uses [`RuntimeConfig::with_runtime_dir`] instead.
    pub fn new() -> Result<Self, RuntimeError> {
        let launch = chatspeed_runtime_client::resolve_launch_config()
            .map_err(|error| RuntimeError::Config(error.to_string()))?;
        Ok(Self::from_launch_config(launch))
    }

    /// Builds a configuration with an explicitly owned runtime directory.
    ///
    /// The database and application-data directory default under `runtime_dir`.
    /// This sandbox never consults the environment or the platform profile, so
    /// two sandboxes on the same machine are always isolated.
    pub fn with_runtime_dir(runtime_dir: impl Into<PathBuf>) -> Self {
        Self::from_launch_config(chatspeed_runtime_client::sandbox_launch_config(runtime_dir))
    }

    /// Resolves the configuration from the process environment and build profile.
    ///
    /// - `CHATSPEED_RUNTIME_DIR` selects an explicit runtime-directory sandbox;
    /// - `CHATSPEED_HOME` selects the legacy `${home}/runtime` sandbox;
    /// - `CHATSPEED_RUNTIME_DB` overrides the database file path;
    /// - `CHATSPEED_RUNTIME_APP_DATA_DIR` overrides the application-data directory;
    /// - `CHATSPEED_RUNTIME_GRACE_MS` / `CHATSPEED_RUNTIME_LEASE_TTL_MS`
    ///   override the grace and lease TTL in milliseconds.
    pub fn from_env() -> Result<Self, RuntimeError> {
        let launch = chatspeed_runtime_client::resolve_launch_config()
            .map_err(|error| RuntimeError::Config(error.to_string()))?;
        let mut config = Self::from_launch_config(launch);
        if let Some(value) = read_duration_ms("CHATSPEED_RUNTIME_GRACE_MS")? {
            config.grace = value;
        }
        if let Some(value) = read_duration_ms("CHATSPEED_RUNTIME_LEASE_TTL_MS")? {
            config.lease_ttl = value;
        }
        Ok(config)
    }

    /// Wraps a resolved launch configuration with the default grace and TTL.
    fn from_launch_config(launch: chatspeed_runtime_client::RuntimeLaunchConfig) -> Self {
        Self {
            runtime_dir: launch.runtime_dir().to_path_buf(),
            db_path: launch.db_path().to_path_buf(),
            app_data_dir: launch.app_data_dir().to_path_buf(),
            grace: DEFAULT_GRACE,
            lease_ttl: DEFAULT_LEASE_TTL,
            shutdown_budget: DEFAULT_SHUTDOWN_BUDGET,
            force_budget: DEFAULT_FORCE_BUDGET,
            termination: Arc::new(LogOnlyTerminationHook),
        }
    }

    /// Directory that the runtime exclusively owns.
    pub fn runtime_dir(&self) -> &Path {
        &self.runtime_dir
    }

    /// Persistent database file the runtime owns.
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Application-data directory used by server-owned capability storage.
    pub fn app_data_dir(&self) -> &Path {
        &self.app_data_dir
    }

    /// Idle grace period before shutdown once the last lease is gone.
    pub fn grace(&self) -> Duration {
        self.grace
    }

    /// Lease TTL applied on register and renew.
    pub fn lease_ttl(&self) -> Duration {
        self.lease_ttl
    }

    /// Total budget shared by the cancelling, draining and stop phases.
    pub fn shutdown_budget(&self) -> Duration {
        self.shutdown_budget
    }

    /// Extra budget granted only to an explicitly forced shutdown.
    pub fn force_budget(&self) -> Duration {
        self.force_budget
    }

    /// The last-resort termination fallback for an exhausted forced shutdown.
    pub fn termination_hook(&self) -> Arc<dyn TerminationHook> {
        self.termination.clone()
    }

    /// Overrides the persistent database file path.
    pub fn with_db_path(mut self, db_path: impl Into<PathBuf>) -> Self {
        self.db_path = db_path.into();
        self
    }

    /// Overrides the application-data directory.
    pub fn with_app_data_dir(mut self, app_data_dir: impl Into<PathBuf>) -> Self {
        self.app_data_dir = app_data_dir.into();
        self
    }

    /// Overrides the idle grace period.
    pub fn with_grace(mut self, grace: Duration) -> Self {
        self.grace = grace;
        self
    }

    /// Overrides the lease TTL.
    pub fn with_lease_ttl(mut self, lease_ttl: Duration) -> Self {
        self.lease_ttl = lease_ttl;
        self
    }

    /// Overrides the total graceful-shutdown budget.
    pub fn with_shutdown_budget(mut self, budget: Duration) -> Self {
        self.shutdown_budget = budget;
        self
    }

    /// Overrides the extra budget granted only to a forced shutdown.
    pub fn with_force_budget(mut self, budget: Duration) -> Self {
        self.force_budget = budget;
        self
    }

    /// Installs a last-resort termination fallback. The default is log-only so
    /// embedding the daemon never terminates the host process.
    pub fn with_termination_hook(mut self, hook: Arc<dyn TerminationHook>) -> Self {
        self.termination = hook;
        self
    }

    /// Lease sweep cadence: half the TTL, clamped to `[10ms, 1s]` so a short
    /// test TTL is still observed promptly and a long TTL is not swept hot.
    fn sweep_interval(&self) -> Duration {
        let half = self.lease_ttl / 2;
        let floor = Duration::from_millis(10);
        let ceiling = Duration::from_millis(1000);
        half.clamp(floor, ceiling)
    }
}

fn read_duration_ms(key: &str) -> Result<Option<Duration>, RuntimeError> {
    match std::env::var(key) {
        Ok(value) if !value.trim().is_empty() => {
            let millis = value.trim().parse::<u64>().map_err(|error| {
                RuntimeError::Config(format!("{key} must be milliseconds: {error}"))
            })?;
            Ok(Some(Duration::from_millis(millis)))
        }
        _ => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Runtime-directory lock
// ---------------------------------------------------------------------------

/// Identity recorded inside the runtime-directory lock file.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct LockRecord {
    instance_id: String,
    pid: u32,
    started_at: String,
}

/// Path of the runtime-directory lock file.
pub fn instance_lock_path(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join(LOCK_FILE_NAME)
}

/// Exclusive ownership of a stable runtime-directory lock file.
///
/// The kernel lock, rather than a pid probe or lock-file deletion, arbitrates
/// concurrent first starts and stale takeover. The inode is never removed:
/// unlinking a locked file would let a new opener lock a different inode.
#[derive(Debug)]
pub struct RuntimeDirLock {
    path: PathBuf,
    instance_id: String,
    file: fs::File,
}

impl RuntimeDirLock {
    pub fn acquire(runtime_dir: &Path) -> Result<Self, RuntimeError> {
        prepare_runtime_dir(runtime_dir)?;
        let path = instance_lock_path(runtime_dir);
        let mut options = fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path)?;
        file.try_lock().map_err(|error| {
            let pid = read_lock_record(&path)
                .map(|record| record.pid)
                .unwrap_or(0);
            match error {
                std::fs::TryLockError::WouldBlock => RuntimeError::AlreadyRunning {
                    dir: runtime_dir.to_path_buf(),
                    pid,
                },
                std::fs::TryLockError::Error(error) => RuntimeError::Io(error),
            }
        })?;
        restrict_permissions(&path, 0o600)?;
        if file.metadata()?.len() > 0 && read_lock_record(&path).is_none() {
            return Err(RuntimeError::LockMalformed { path });
        }
        let instance_id = generate_instance_id();
        let record = LockRecord {
            instance_id: instance_id.clone(),
            pid: std::process::id(),
            started_at: now_rfc3339(),
        };
        let body = serde_json::to_vec_pretty(&record)
            .map_err(|error| RuntimeError::Serialization(error.to_string()))?;
        file.set_len(0)?;
        file.write_all(&body)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        Ok(Self {
            path,
            instance_id,
            file,
        })
    }

    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for RuntimeDirLock {
    fn drop(&mut self) {
        // The record remains for diagnostics. Releasing the kernel lock is
        // enough for the next process to acquire the same stable file.
        let _ = self.file.unlock();
    }
}

fn read_lock_record(path: &Path) -> Option<LockRecord> {
    let body = fs::read(path).ok()?;
    serde_json::from_slice(&body).ok()
}

fn prepare_runtime_dir(runtime_dir: &Path) -> Result<(), RuntimeError> {
    fs::create_dir_all(runtime_dir)?;
    restrict_permissions(runtime_dir, 0o700)
}

#[cfg(unix)]
fn restrict_permissions(path: &Path, mode: u32) -> Result<(), RuntimeError> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(mode);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path, _mode: u32) -> Result<(), RuntimeError> {
    // Windows inherits the current-user ACL from the profile directory.
    Ok(())
}

// ---------------------------------------------------------------------------
// Database-authority lock
// ---------------------------------------------------------------------------

/// Path of the database-authority sidecar lock for `db_path`.
///
/// The lock lives next to the *canonical* database file and is never the
/// database itself: the database content is never opened or read, only its
/// metadata and canonical path are used. Resolving the canonical file target is
/// what makes two aliases of one database agree on a single lock: a symlinked
/// directory, a `.`/`..` component, or a symlink to the database file itself all
/// map to the same file's lock, while a genuinely different file keeps its own.
///
/// A database that exists is keyed by its canonical file path; a database that
/// does not exist yet is keyed by its canonical parent directory plus its file
/// name. Resolution fails closed: an unresolvable symlink, a parent that cannot
/// be canonicalized, or a database reachable through extra hard links (whose
/// aliases cannot be proven unique without scanning the filesystem) is refused
/// rather than given an ambiguity-prone lock name.
pub fn database_lock_path(db_path: &Path) -> Result<PathBuf, RuntimeError> {
    let parent = db_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| {
            RuntimeError::Config(format!(
                "database path {} has no parent directory",
                db_path.display()
            ))
        })?;
    fs::create_dir_all(parent)?;

    match fs::canonicalize(db_path) {
        Ok(canonical_path) => {
            reject_ambiguous_database_links(&canonical_path)?;
            let mut lock_path = canonical_path.into_os_string();
            lock_path.push(DB_LOCK_SUFFIX);
            Ok(PathBuf::from(lock_path))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // A symlink whose target does not resolve is an alias we cannot prove
            // unique; refuse rather than guess at a lock name.
            if db_path
                .symlink_metadata()
                .map(|metadata| metadata.file_type().is_symlink())
                .unwrap_or(false)
            {
                return Err(RuntimeError::Config(format!(
                    "database path {} is an unresolvable symlink; refusing to derive an ambiguity-prone authority lock",
                    db_path.display()
                )));
            }
            // The database does not exist yet: anchor the lock to the canonical
            // parent so directory aliases still coincide. No `.`-relative
            // fallback: a parent that cannot be canonicalized fails closed.
            let normalized_parent = fs::canonicalize(parent)?;
            let mut lock_name = db_path
                .file_name()
                .map(OsStr::to_os_string)
                .unwrap_or_else(|| OsString::from(DB_FILE_NAME));
            lock_name.push(DB_LOCK_SUFFIX);
            Ok(normalized_parent.join(lock_name))
        }
        Err(error) => Err(RuntimeError::Io(error)),
    }
}

/// Rejects a database whose canonical file has extra hard links.
///
/// The lock is keyed by canonical path, which cannot unify two hard links to one
/// inode; proving a single authority would require scanning the filesystem for
/// every alias. Failing closed for the uncommon hard-linked database is the
/// minimal safe behavior. The link count is metadata only: the database is never
/// opened or read.
#[cfg(unix)]
fn reject_ambiguous_database_links(canonical_path: &Path) -> Result<(), RuntimeError> {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::metadata(canonical_path)?;
    // Only a regular file is a database. A directory carries an inherent link
    // count and is not a candidate here; pointing the database at one fails later
    // when it cannot be opened, which is the caller's concern, not the lock's.
    if metadata.is_file() && metadata.nlink() > 1 {
        return Err(RuntimeError::Config(format!(
            "database file {} has {} hard links; a unique authority lock cannot be proven without scanning the filesystem",
            canonical_path.display(),
            metadata.nlink()
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
fn reject_ambiguous_database_links(_canonical_path: &Path) -> Result<(), RuntimeError> {
    Ok(())
}

/// Kernel-level exclusive authority over one database file.
///
/// The runtime-directory lock makes one runtime win per directory; this sidecar
/// lock is what makes one runtime win per *database*. Two runtimes with
/// different runtime directories but the same database would otherwise both open
/// it, so acquiring this lock fails before the owner is assembled. The file's
/// inode is never removed (unlinking a locked file would let a new opener lock a
/// different inode), and the database content is never touched.
#[derive(Debug)]
pub struct RuntimeDbLock {
    path: PathBuf,
    file: fs::File,
}

impl RuntimeDbLock {
    /// Acquires the database authority for `db_path`.
    pub fn acquire(db_path: &Path) -> Result<Self, RuntimeError> {
        let path = database_lock_path(db_path)?;
        let mut options = fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path)?;
        file.try_lock().map_err(|error| {
            let pid = read_lock_record(&path)
                .map(|record| record.pid)
                .unwrap_or(0);
            match error {
                std::fs::TryLockError::WouldBlock => RuntimeError::AlreadyRunning {
                    dir: db_path.to_path_buf(),
                    pid,
                },
                std::fs::TryLockError::Error(error) => RuntimeError::Io(error),
            }
        })?;
        restrict_permissions(&path, 0o600)?;
        let record = LockRecord {
            instance_id: generate_instance_id(),
            pid: std::process::id(),
            started_at: now_rfc3339(),
        };
        let body = serde_json::to_vec_pretty(&record)
            .map_err(|error| RuntimeError::Serialization(error.to_string()))?;
        file.set_len(0)?;
        file.write_all(&body)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        Ok(Self { path, file })
    }

    /// Path of the sidecar lock file.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for RuntimeDbLock {
    fn drop(&mut self) {
        // The record remains for diagnostics; releasing the kernel lock is what
        // lets a successor acquire the same stable inode.
        let _ = self.file.unlock();
    }
}

// ---------------------------------------------------------------------------
// Instance identity / secrets
// ---------------------------------------------------------------------------

/// Generates a fresh runtime instance id.
pub fn generate_instance_id() -> String {
    random_hex::<16>()
}

fn generate_lease_id() -> String {
    random_hex::<16>()
}

/// Constant-time comparison of two opaque lease ids.
fn lease_id_matches(expected: &str, presented: &str) -> bool {
    let expected = expected.as_bytes();
    let presented = presented.as_bytes();
    if expected.len() != presented.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in expected.iter().zip(presented.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn random_hex<const N: usize>() -> String {
    let mut bytes = [0u8; N];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn expires_at_rfc3339(ttl: Duration) -> String {
    let ttl = chrono::Duration::seconds(ttl.as_secs() as i64)
        + chrono::Duration::nanoseconds(ttl.subsec_nanos() as i64);
    (chrono::Utc::now() + ttl).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

// ---------------------------------------------------------------------------
// Leases
// ---------------------------------------------------------------------------

/// Lease mutation failures.
#[derive(Debug, thiserror::Error)]
pub enum LeaseError {
    #[error("invalid lease request: {0}")]
    InvalidInput(String),
    #[error("no active lease for client `{0}`")]
    NotFound(String),
    /// The runtime is quiescing and refuses the registration.
    #[error("runtime is stopping and refuses the registration: {0}")]
    Stopping(String),
}

impl From<LeaseError> for RuntimeLeaseError {
    fn from(error: LeaseError) -> Self {
        match error {
            LeaseError::InvalidInput(message) => RuntimeLeaseError::InvalidInput(message),
            LeaseError::NotFound(client_id) => RuntimeLeaseError::NotFound(client_id),
            LeaseError::Stopping(message) => RuntimeLeaseError::Stopping(message),
        }
    }
}

struct LeaseRecord {
    client_kind: String,
    lease_id: String,
    expires_at: Instant,
    expires_at_text: String,
}

/// In-memory lease registry.
///
/// Leases are instance-local and never persisted. TTL accounting uses a
/// monotonic `Instant`, so a wall-clock jump cannot extend a lease. Expiry is
/// applied eagerly on every mutation and periodically by the runtime sweeper,
/// which is what lets the idle supervisor observe the count reaching zero.
pub struct LeaseRegistry {
    ttl: Duration,
    inner: std::sync::Mutex<HashMap<String, LeaseRecord>>,
    count_tx: watch::Sender<usize>,
    /// The one runtime admission gate, installed before discovery is published.
    ///
    /// Fencing the registration here, at the lowest canonical layer, means a
    /// direct (non-HTTP) caller cannot bypass the quiesce the way a handler-only
    /// check would allow.
    admission: std::sync::OnceLock<Arc<dyn WorkAdmission>>,
}

impl LeaseRegistry {
    /// Creates an empty registry applying `ttl` to every lease.
    pub fn new(ttl: Duration) -> Self {
        let (count_tx, _count_rx) = watch::channel(0);
        Self {
            ttl,
            inner: std::sync::Mutex::new(HashMap::new()),
            count_tx,
            admission: std::sync::OnceLock::new(),
        }
    }

    /// Installs the one runtime work-admission gate. First-wins and idempotent.
    pub fn install_admission(&self, gate: Arc<dyn WorkAdmission>) {
        let _ = self.admission.set(gate);
    }

    fn admit_registration(&self) -> Result<AdmissionTicket, LeaseError> {
        match self.admission.get() {
            Some(gate) => gate
                .admit(WorkKind::LeaseRegister)
                .map_err(|denied| LeaseError::Stopping(denied.message)),
            None => Ok(AdmissionTicket::permissive()),
        }
    }

    /// Registers (or replaces) a lease and returns the wire response.
    pub fn register(
        &self,
        request: &ClientLeaseRequest,
    ) -> Result<ClientLeaseResponse, LeaseError> {
        let client_id = request.client_id.trim().to_string();
        let client_kind = request.client_kind.trim().to_string();
        if client_id.is_empty() || client_id.len() > MAX_CLIENT_ID_LEN {
            return Err(LeaseError::InvalidInput(format!(
                "client_id must be 1..={MAX_CLIENT_ID_LEN} characters"
            )));
        }
        if client_kind.is_empty() || client_kind.len() > MAX_CLIENT_KIND_LEN {
            return Err(LeaseError::InvalidInput(format!(
                "client_kind must be 1..={MAX_CLIENT_KIND_LEN} characters"
            )));
        }

        // Fence the registration through the one gate so a quiescing runtime
        // never resurrects a lease, and the idle supervisor observes the
        // registration as activity even without a request-level ticket. The
        // ticket is held for the (short) registration and released on return.
        let _ticket = self.admit_registration()?;

        let now = Instant::now();
        let record = LeaseRecord {
            client_kind,
            lease_id: generate_lease_id(),
            expires_at: now + self.ttl,
            expires_at_text: expires_at_rfc3339(self.ttl),
        };
        let response = ClientLeaseResponse {
            client_id: client_id.clone(),
            lease_id: record.lease_id.clone(),
            expires_at: record.expires_at_text.clone(),
        };

        let client_kind = record.client_kind.clone();
        let count = {
            let mut leases = self.lock();
            leases.retain(|_, entry| entry.expires_at > now);
            leases.insert(client_id, record);
            leases.len()
        };
        self.publish_count(count);
        log::debug!("[Runtime] registered lease for client of kind {client_kind}");
        Ok(response)
    }

    /// Extends an existing, non-expired lease, keeping its lease id.
    pub fn renew(&self, client_id: &str) -> Result<ClientLeaseResponse, LeaseError> {
        let now = Instant::now();
        let mut leases = self.lock();
        leases.retain(|_, entry| entry.expires_at > now);
        let count = leases.len();
        let Some(record) = leases.get_mut(client_id) else {
            drop(leases);
            self.publish_count(count);
            return Err(LeaseError::NotFound(client_id.to_string()));
        };
        record.expires_at = now + self.ttl;
        record.expires_at_text = expires_at_rfc3339(self.ttl);
        let response = ClientLeaseResponse {
            client_id: client_id.to_string(),
            lease_id: record.lease_id.clone(),
            expires_at: record.expires_at_text.clone(),
        };
        drop(leases);
        self.publish_count(count);
        Ok(response)
    }

    /// Resolves a live lease for a `(client_id, lease_id)` proof.
    ///
    /// The lease id is opaque; a mismatch or an expired lease resolves to
    /// `NotFound` so a caller cannot probe which leases exist. A monotonic
    /// expiry check prevents a wall-clock jump from resurrecting a lease.
    pub fn validate(&self, client_id: &str, lease_id: &str) -> Result<ClientLease, LeaseError> {
        let now = Instant::now();
        let mut leases = self.lock();
        leases.retain(|_, entry| entry.expires_at > now);
        let count = leases.len();
        let resolved = leases
            .get(client_id)
            .filter(|record| lease_id_matches(&record.lease_id, lease_id))
            .map(|record| ClientLease {
                client_id: client_id.to_string(),
                lease_id: record.lease_id.clone(),
                client_kind: record.client_kind.clone(),
                expires_at: record.expires_at_text.clone(),
            });
        drop(leases);
        self.publish_count(count);
        resolved.ok_or_else(|| LeaseError::NotFound(client_id.to_string()))
    }

    /// Releases a lease. A lease that already expired counts as unknown.
    pub fn release(&self, client_id: &str) -> Result<(), LeaseError> {
        self.release_with_kind(client_id).map(|_| ())
    }

    fn release_with_kind(&self, client_id: &str) -> Result<String, LeaseError> {
        let now = Instant::now();
        let (removed, count) = {
            let mut leases = self.lock();
            leases.retain(|_, entry| entry.expires_at > now);
            let removed = leases.remove(client_id);
            (removed, leases.len())
        };
        self.publish_count(count);
        let record = removed.ok_or_else(|| LeaseError::NotFound(client_id.to_string()))?;
        log::debug!("[Runtime] released lease for client");
        Ok(record.client_kind)
    }

    /// Drops expired leases; used by the runtime sweeper.
    pub fn sweep_expired(&self) {
        let now = Instant::now();
        let count = {
            let mut leases = self.lock();
            let before = leases.len();
            leases.retain(|_, entry| entry.expires_at > now);
            if leases.len() == before {
                return;
            }
            leases.len()
        };
        log::debug!("[Runtime] swept expired leases");
        self.publish_count(count);
    }

    /// Number of non-expired leases (pruning expired entries first).
    pub fn active_count(&self) -> usize {
        let now = Instant::now();
        let count = {
            let mut leases = self.lock();
            leases.retain(|_, entry| entry.expires_at > now);
            leases.len()
        };
        self.publish_count(count);
        count
    }

    /// Subscribes to lease-count changes for the idle supervisor.
    pub fn subscribe(&self) -> watch::Receiver<usize> {
        self.count_tx.subscribe()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, LeaseRecord>> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn publish_count(&self, count: usize) {
        if *self.count_tx.borrow() != count {
            let _ = self.count_tx.send_replace(count);
        }
    }
}

// ---------------------------------------------------------------------------
// Coordinated shutdown and the runtime control plane
// ---------------------------------------------------------------------------

/// Total budget for a graceful coordinated shutdown (U-5).
///
/// The cancelling phase, the drain and the server/background stop all share this
/// one deadline: owned work is cancelled, admitted work runs to its drain and the
/// control plane stops within it. A budget that elapses never becomes a false
/// `stopped` claim.
pub const DEFAULT_SHUTDOWN_BUDGET: Duration = Duration::from_secs(15);

/// Extra budget granted only to an explicitly forced shutdown (U-5).
///
/// After the graceful budget is exhausted, a *forced* request may spend this much
/// more waiting for the instance to finish before the last-resort termination
/// fallback fires. An idle, non-forced shutdown never gets it.
pub const DEFAULT_FORCE_BUDGET: Duration = Duration::from_secs(5);

/// Last-resort termination for a forced shutdown that could not clean up.
///
/// The runtime library must never terminate the process on its own: a caller
/// that embeds the daemon (or a test) would be killed too. The default hook only
/// logs, and the real `chatspeed-runtime` binary installs
/// [`ProcessExitTerminationHook`], which performs the final self-exit fallback
/// once a forced shutdown has exhausted both budgets. Tests inject their own hook
/// to observe the fallback without exiting.
pub trait TerminationHook: Send + Sync + std::fmt::Debug {
    /// Invoked once, after a forced shutdown exhausted the graceful and forced
    /// budgets without the instance reaching `stopped`.
    fn force_exit(&self, instance_id: &str, pid: u32);
}

/// The library default: record the fallback and never exit the process.
#[derive(Debug, Default)]
pub struct LogOnlyTerminationHook;

impl TerminationHook for LogOnlyTerminationHook {
    fn force_exit(&self, instance_id: &str, pid: u32) {
        log::error!(
            "[Runtime] forced shutdown of instance {instance_id} (pid {pid}) could not complete within budget; the caller must terminate it (the library default never self-exits)"
        );
    }
}

/// The real binary hook: the last-resort self-exit fallback.
#[derive(Debug, Default)]
pub struct ProcessExitTerminationHook;

impl TerminationHook for ProcessExitTerminationHook {
    fn force_exit(&self, instance_id: &str, pid: u32) {
        log::error!(
            "[Runtime] forced shutdown of instance {instance_id} (pid {pid}) exhausted its budget; performing the last-resort self-exit"
        );
        // Only the real binary reaches here: a caller that embedded the library
        // installs its own hook instead, so this never kills a host process.
        std::process::exit(0);
    }
}

/// Cadence of the authoritative automation keep-alive re-read.
///
/// The idle supervisor re-reads the automation summary on this interval as well
/// as at grace expiry, so a newly enabled schedule or a live run keeps the
/// runtime alive even without a client lease.
const SUMMARY_POLL: Duration = Duration::from_secs(2);

/// An in-flight shutdown operation.
struct OperationRecord {
    id: String,
    stage: RuntimeOperationStage,
    force: bool,
    accepted_at: String,
    /// Terminal outcome once known: graceful, forced or failed. Absent while the
    /// operation is still in flight.
    outcome: Option<RuntimeShutdownOutcome>,
}

impl OperationRecord {
    fn dto(&self) -> RuntimeOperationDto {
        RuntimeOperationDto {
            operation_id: self.id.clone(),
            stage: self.stage,
            force: self.force,
            accepted_at: self.accepted_at.clone(),
            outcome: self.outcome,
        }
    }
}

/// The one coordinated-shutdown authority.
///
/// Every shutdown source — a termination signal, the idle supervisor and an
/// explicit `/control/v1/runtime/shutdown` — funnels through [`Self::request`],
/// so the gate is closed exactly once and the same operation is reported to
/// every observer. The worker drains admitted work before it signals the server
/// to stop, so the runtime never cuts in-flight work off and never drifts from
/// the gate observation.
struct RuntimeShutdown {
    instance_id: String,
    gate: Arc<RuntimeLifecycleGate>,
    operation: std::sync::Mutex<Option<OperationRecord>>,
    /// Flips the control plane's graceful-shutdown signal.
    trigger: watch::Sender<bool>,
    /// Weak handle to the owner so the cancelling phase can reach its live work
    /// without keeping the database owner alive past the drain.
    owner: std::sync::Weak<RuntimeOwner>,
    /// Flips to `true` once the serve task has fully stopped and released its
    /// resources. A forced shutdown waits on this before its last-resort fallback.
    finished: watch::Receiver<bool>,
    /// Total budget shared by the cancelling, draining and stop phases.
    shutdown_budget: Duration,
    /// Extra budget granted only to an explicitly forced shutdown.
    force_budget: Duration,
    /// Last-resort termination fallback for an exhausted forced shutdown.
    termination: Arc<dyn TerminationHook>,
}

/// The outcome to record when the instance reaches `stopped`.
///
/// A cancellation that could not prove its owned teardown (an MCP client left
/// unproven, or a cancel pass that exceeded its budget) already recorded
/// `Failed`; reaching `stopped` must not re-label that as a clean graceful exit.
/// `stopped` records the lifecycle phase, while the outcome records whether the
/// shutdown was actually clean.
fn stopped_outcome(
    force: bool,
    current: Option<RuntimeShutdownOutcome>,
) -> Option<RuntimeShutdownOutcome> {
    match current {
        Some(RuntimeShutdownOutcome::Failed) => Some(RuntimeShutdownOutcome::Failed),
        _ => Some(if force {
            RuntimeShutdownOutcome::Forced
        } else {
            RuntimeShutdownOutcome::Graceful
        }),
    }
}

impl RuntimeShutdown {
    /// Requests a coordinated shutdown, starting the worker on the first call.
    ///
    /// Returns the accepted operation and the [`ShutdownHandoff`] that releases
    /// the worker. Idempotent: a later call returns the same operation with an
    /// inert hand-off, so a signal, an idle expiry and an explicit request cannot
    /// start three competing shutdowns.
    fn request(self: &Arc<Self>, force: bool) -> (RuntimeOperationDto, ShutdownHandoff) {
        let (handoff, handoff_rx) = ShutdownHandoff::channel();
        let (record, is_new, upgraded) = {
            let mut slot = self
                .operation
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match slot.as_mut() {
                Some(record) => {
                    // An explicit force request takes priority over an
                    // already-running graceful (idle) shutdown: it upgrades the
                    // recorded operation so the worker escalates to the forced
                    // cleanup/fallback path instead of only draining.
                    let upgraded = force && !record.force;
                    if upgraded {
                        record.force = true;
                    }
                    (record.dto(), false, upgraded)
                }
                None => {
                    // Close the gate under the same authority that records the
                    // operation, so no admission can slip in after the close.
                    self.gate.begin_quiesce();
                    let record = OperationRecord {
                        id: random_hex::<12>(),
                        stage: RuntimeOperationStage::Accepted,
                        force,
                        accepted_at: now_rfc3339(),
                        outcome: None,
                    };
                    let dto = record.dto();
                    *slot = Some(record);
                    (dto, true, false)
                }
            }
        };
        if is_new {
            self.spawn_worker(handoff_rx, force);
        } else if upgraded {
            // The worker re-reads the recorded operation at its force decision, so
            // an upgrade that lands before that point is observed; the harness
            // hand-off of this later call stays inert.
            log::info!(
                "[Runtime] forced shutdown upgraded the in-flight operation (instance {})",
                self.instance_id
            );
        }
        (record, handoff)
    }

    /// Whether the recorded operation is a forced shutdown.
    ///
    /// Re-read rather than captured so a later explicit force request can upgrade
    /// an in-flight graceful shutdown before the worker takes its force decision.
    fn operation_force(&self) -> bool {
        self.operation
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .is_some_and(|record| record.force)
    }

    /// The in-flight operation, for the status route.
    fn operation_dto(&self) -> Option<RuntimeOperationDto> {
        self.operation
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .map(OperationRecord::dto)
    }

    fn set_stage(&self, stage: RuntimeOperationStage) {
        let mut slot = self
            .operation
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(record) = slot.as_mut() {
            record.stage = stage;
        }
    }

    /// Drives the shutdown worker: quiesce -> cancel owned work -> drain -> signal
    /// the server -> (forced only) wait for `stopped` or fire the termination
    /// fallback. The worker waits for the response hand-off first, so an explicit
    /// request has answered its `202` before anything is torn down, and it never
    /// awaits the runtime's own server shutdown.
    fn spawn_worker(self: &Arc<Self>, handoff_rx: tokio::sync::oneshot::Receiver<()>, force: bool) {
        let this = Arc::clone(self);
        tokio::spawn(async move {
            // Released when the `202` response has been handed back.
            let _ = handoff_rx.await;

            // The one deadline the cancelling and draining phases share.
            let deadline = tokio::time::Instant::now() + this.shutdown_budget;

            this.set_stage(RuntimeOperationStage::Quiescing);

            // Cancelling (U-5): cancel every owned unit of work so the drain
            // observes live work stopping instead of only a closed gate.
            this.gate.advance(RuntimeLifecyclePhase::Cancelling);
            this.set_stage(RuntimeOperationStage::Cancelling);
            match this.owner.upgrade() {
                Some(owner) => {
                    match tokio::time::timeout_at(deadline, owner.cancel_owned_work()).await {
                        Ok(report) => {
                            log::info!(
                                "[Runtime] cancelling phase interrupted {} chat turn(s), stopped {}/{} live workflow(s), cancelled {} owned process(es) (pre-workflow shells + shell-tool children), closed interactive PTYs, stopped {}/{} MCP server(s)",
                                report.chats_interrupted,
                                report.workflows_stopped,
                                report.workflows_live,
                                report.owned_processes_cancelled,
                                report.mcp.stopped,
                                report.mcp.servers
                            );
                            if report.mcp.unproven > 0 {
                                // The cancel pass could not prove every owned MCP
                                // teardown; record the outcome as failed so the
                                // status route never reports a clean stop. The
                                // forced-termination escalation stays gated on an
                                // explicit force request (U-5), so an unproven MCP
                                // stop does not, by itself, kill a healthy process.
                                log::error!(
                                    "[Runtime] cancelling phase could not prove teardown of {} of {} owned MCP server(s); recording the shutdown as failed",
                                    report.mcp.unproven,
                                    report.mcp.servers
                                );
                                this.set_outcome(Some(RuntimeShutdownOutcome::Failed));
                            }
                        }
                        Err(_) => {
                            // The cancel pass itself did not finish inside the
                            // shared budget, so owned teardown is also unproven.
                            log::warn!(
                                "[Runtime] cancelling phase exceeded the shutdown budget; draining with owned work possibly still running"
                            );
                            this.set_outcome(Some(RuntimeShutdownOutcome::Failed));
                        }
                    }
                }
                None => log::warn!(
                    "[Runtime] cancelling phase found no live owner; owned work is already gone"
                ),
            }

            // Draining: wait for admitted work to release its tickets within the
            // same deadline.
            this.gate.advance(RuntimeLifecyclePhase::Draining);
            this.set_stage(RuntimeOperationStage::Draining);
            let drained = tokio::time::timeout_at(deadline, this.gate.wait_drained())
                .await
                .is_ok();
            if !drained {
                // A budget that elapses is never turned into a `stopped` claim:
                // the serve task still owns the mark-stopped transition and the
                // lock, and it further withholds the claim while admissions are
                // held, so the instance is not falsely declared gone.
                log::warn!(
                    "[Runtime] shutdown budget elapsed with {} admission(s) still held; the serve task will withhold the `stopped` claim until they drain",
                    this.gate.active_admissions()
                );
            }

            // Signal the server to stop. Its shutdown broadcast ends open SSE
            // streams so graceful shutdown completes instead of hanging.
            let _ = this.trigger.send_replace(true);

            // Forced escalation (U-5). Only an explicitly forced request spends
            // the extra budget, and only after the graceful budget elapsed without
            // the instance reaching `stopped`. Idle/non-forced shutdowns never
            // force.
            if force || this.operation_force() {
                let stopped = tokio::time::timeout(this.force_budget, async {
                    if *this.finished.borrow() {
                        return;
                    }
                    let mut finished = this.finished.clone();
                    while !*finished.borrow() {
                        if finished.changed().await.is_err() {
                            return;
                        }
                    }
                })
                .await
                .is_ok();
                if stopped {
                    log::info!(
                        "[Runtime] forced shutdown completed within budget (instance {})",
                        this.instance_id
                    );
                } else {
                    log::error!(
                        "[Runtime] forced shutdown of instance {} exhausted both budgets without reaching `stopped`; invoking the termination fallback",
                        this.instance_id
                    );
                    this.set_outcome(Some(RuntimeShutdownOutcome::Failed));
                    this.termination
                        .force_exit(&this.instance_id, std::process::id());
                }
            }

            log::info!(
                "[Runtime] coordinated shutdown dispatched (instance {})",
                this.instance_id
            );
        });
    }

    /// Marks the operation stopped once the control plane has fully stopped.
    ///
    /// Records whether the exit was graceful or forced, so a client can tell the
    /// two apart on the status route. A cancellation that could not prove its
    /// owned teardown already recorded `Failed`; that is preserved here so a
    /// client never mistakes it for a clean graceful exit.
    fn mark_stopped(&self) {
        self.set_stage(RuntimeOperationStage::Stopped);
        self.set_outcome(stopped_outcome(
            self.operation_force(),
            self.recorded_outcome(),
        ));
        self.gate.advance(RuntimeLifecyclePhase::Stopped);
    }

    /// The terminal outcome already recorded on the in-flight operation, if any.
    fn recorded_outcome(&self) -> Option<RuntimeShutdownOutcome> {
        self.operation
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .and_then(|record| record.outcome)
    }

    /// Records the terminal outcome on the in-flight operation, if any.
    fn set_outcome(&self, outcome: Option<RuntimeShutdownOutcome>) {
        let mut slot = self
            .operation
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(record) = slot.as_mut() {
            record.outcome = outcome;
        }
    }
}

/// The runtime's control-plane extension: the lease lifecycle, the lifecycle
/// status and the coordinated shutdown on the one canonical server.
struct RuntimeControl {
    instance_id: String,
    leases: Arc<LeaseRegistry>,
    gate: Arc<RuntimeLifecycleGate>,
    service: Arc<WorkflowApplicationService>,
    shutdown: Arc<RuntimeShutdown>,
}

impl RuntimeControl {
    /// The authoritative automation keep-alive summary.
    ///
    /// A read failure is never reported as "nothing to do": it degrades to the
    /// unknown summary, whose `keep_alive` is true, so an unobservable runtime
    /// stays alive.
    fn automation_summary(&self) -> RuntimeAutomationSummaryDto {
        let now = normalize_datetime_for_db(chrono::Local::now());
        match self.service.automation_runtime_summary(&now) {
            Ok(summary) => automation_summary_dto(&summary),
            Err(error) => {
                log::warn!(
                    "[Runtime] automation summary unreadable; treating the runtime as busy: {error}"
                );
                unknown_automation_summary()
            }
        }
    }

    fn build_status(&self) -> RuntimeStatusDto {
        let snapshot = self.gate.snapshot();
        RuntimeStatusDto {
            protocol_version: PROTOCOL_VERSION.to_string(),
            schema_version: RUNTIME_LIFECYCLE_SCHEMA_VERSION.to_string(),
            service: SERVICE_NAME.to_string(),
            instance_id: self.instance_id.clone(),
            pid: std::process::id(),
            phase: snapshot.phase,
            epoch: snapshot.epoch,
            active_admissions: snapshot.active,
            active_leases: self.leases.active_count(),
            quiesced: snapshot.phase != RuntimeLifecyclePhase::Running,
            automation: self.automation_summary(),
            operation: self.shutdown.operation_dto(),
        }
    }
}

/// Projects the runtime's automation summary onto the shared wire DTO.
fn automation_summary_dto(summary: &AutomationRuntimeSummary) -> RuntimeAutomationSummaryDto {
    RuntimeAutomationSummaryDto {
        effective_schedules: summary.effective_schedules,
        live_automation_runs: summary.live_automation_runs,
        in_flight_admissions: summary.in_flight_admissions,
        blocked_reconcile: summary.blocked_reconcile,
        health: match summary.health {
            AutomationRuntimeHealth::Healthy => RuntimeAutomationHealth::Healthy,
            AutomationRuntimeHealth::Degraded => RuntimeAutomationHealth::Degraded,
            AutomationRuntimeHealth::Unknown => RuntimeAutomationHealth::Unknown,
        },
        keep_alive: summary.keep_alive,
    }
}

/// The unobservable summary: keep alive, retry next tick, never claim idle.
fn unknown_automation_summary() -> RuntimeAutomationSummaryDto {
    RuntimeAutomationSummaryDto {
        effective_schedules: 0,
        live_automation_runs: 0,
        in_flight_admissions: 0,
        blocked_reconcile: 0,
        health: RuntimeAutomationHealth::Unknown,
        keep_alive: true,
    }
}

impl RuntimeControlPlane for RuntimeControl {
    fn service_name(&self) -> &str {
        SERVICE_NAME
    }

    fn register_lease(
        &self,
        request: &ClientLeaseRequest,
    ) -> Result<ClientLeaseResponse, RuntimeLeaseError> {
        // The fence lives in the lease registry itself, the lowest canonical
        // layer, so both this authority and a direct registry caller are refused
        // on the one gate while the runtime is quiescing.
        self.leases
            .register(request)
            .map_err(RuntimeLeaseError::from)
    }

    fn renew_lease(&self, client_id: &str) -> Result<ClientLeaseResponse, RuntimeLeaseError> {
        self.leases
            .renew(client_id)
            .map_err(RuntimeLeaseError::from)
    }

    fn validate_lease(
        &self,
        client_id: &str,
        lease_id: &str,
    ) -> Result<ClientLease, RuntimeLeaseError> {
        self.leases
            .validate(client_id, lease_id)
            .map_err(RuntimeLeaseError::from)
    }

    fn release_lease(&self, client_id: &str) -> Result<(), RuntimeLeaseError> {
        self.leases
            .release(client_id)
            .map_err(RuntimeLeaseError::from)
    }

    fn release_lease_for_desktop_exit(
        &self,
        client_id: &str,
    ) -> Result<Option<ShutdownHandoff>, RuntimeLeaseError> {
        let client_kind = self
            .leases
            .release_with_kind(client_id)
            .map_err(RuntimeLeaseError::from)?;
        if client_kind != "tauri" {
            return Ok(None);
        }
        let activity_epoch = self.gate.activity_epoch();
        let mut last_unknown_log = Instant::now();
        if keep_alive(
            &self.leases.subscribe(),
            &self.service,
            &self.gate,
            &mut last_unknown_log,
        ) || !self.gate.try_begin_idle(activity_epoch, 0)
        {
            return Ok(None);
        }
        log::info!("[Runtime] desktop released the last lease; shutting down while idle");
        let (_, handoff) = self.shutdown.request(false);
        Ok(Some(handoff))
    }

    fn runtime_status(&self) -> RuntimeStatusDto {
        self.build_status()
    }

    fn validate_shutdown_proof(
        &self,
        client_id: &str,
        lease_id: &str,
        request: &RuntimeShutdownRequest,
    ) -> Result<(), RuntimeShutdownError> {
        if request.expected_instance_id != self.instance_id {
            return Err(RuntimeShutdownError::InstanceMismatch {
                expected: request.expected_instance_id.clone(),
                actual: self.instance_id.clone(),
            });
        }
        if !request.confirm {
            return Err(RuntimeShutdownError::NotConfirmed);
        }
        // The lease proof is resolved by the runtime's own authority, so a
        // bearer-only client cannot stop the runtime.
        self.leases
            .validate(client_id, lease_id)
            .map_err(|_| RuntimeShutdownError::LeaseInvalid(client_id.to_string()))?;
        Ok(())
    }

    fn request_shutdown(
        &self,
        client_id: &str,
        lease_id: &str,
        request: &RuntimeShutdownRequest,
    ) -> Result<(RuntimeShutdownAcceptedDto, ShutdownHandoff), RuntimeShutdownError> {
        // Re-validate authoritatively: the handler already validated before the
        // idempotency cache, but the effect must never start on a proof that has
        // since become invalid.
        self.validate_shutdown_proof(client_id, lease_id, request)?;
        let (operation, handoff) = self.shutdown.request(request.force);
        let accepted = RuntimeShutdownAcceptedDto {
            schema_version: RUNTIME_LIFECYCLE_SCHEMA_VERSION.to_string(),
            service: SERVICE_NAME.to_string(),
            instance_id: self.instance_id.clone(),
            operation_id: operation.operation_id,
            stage: operation.stage,
            accepted: true,
            force: operation.force,
        };
        Ok((accepted, handoff))
    }
}

// ---------------------------------------------------------------------------
// Runtime lifecycle
// ---------------------------------------------------------------------------

/// Handle for a running runtime instance.
pub struct RuntimeHandle {
    port: u16,
    instance_id: String,
    runtime_dir: PathBuf,
    db_path: PathBuf,
    leases: Arc<LeaseRegistry>,
    gate: Arc<RuntimeLifecycleGate>,
    shutdown: Arc<RuntimeShutdown>,
    finished_rx: watch::Receiver<bool>,
}

impl RuntimeHandle {
    /// Bound loopback port of the canonical control plane.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// This instance's identity (also the lock and discovery instance id).
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    /// Directory this runtime exclusively owns.
    pub fn runtime_dir(&self) -> &Path {
        &self.runtime_dir
    }

    /// Persistent database file this runtime owns.
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Lease registry, exposed for lifecycle control and tests.
    pub fn leases(&self) -> &Arc<LeaseRegistry> {
        &self.leases
    }

    /// The runtime work-admission gate, exposed for lifecycle tests.
    pub fn gate(&self) -> &Arc<RuntimeLifecycleGate> {
        &self.gate
    }

    /// Whether the instance has finished shutting down.
    pub fn is_finished(&self) -> bool {
        *self.finished_rx.borrow()
    }

    /// Requests a coordinated graceful shutdown.
    ///
    /// Idempotent, and the single exit every source funnels through: the gate is
    /// closed synchronously, the worker drains admitted work and only then is the
    /// server told to stop, so in-flight work is never cut off.
    pub fn shutdown(&self) {
        let _ = self.shutdown.request(false);
    }

    /// Waits until the instance has fully stopped and cleaned up.
    pub async fn wait(&self) {
        let mut rx = self.finished_rx.clone();
        while !*rx.borrow() {
            if rx.changed().await.is_err() {
                return;
            }
        }
    }
}

/// Initializes the shared resource locator for a standalone runtime process.
///
/// In development the shared constants already derive `src-tauri/assets`.
/// Release builds have no Tauri resource resolver, so use the conventional
/// sidecar layouts relative to the executable and retain an empty value only
/// when no candidate exists; the owner then reports a clear resource warning.
fn initialize_runtime_resource_dir() {
    let candidates = [
        std::env::var_os(chatspeed_runtime_client::RUNTIME_RESOURCE_DIR_ENV).map(PathBuf::from),
        std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .map(|dir| dir.join("assets")),
        std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .map(|dir| dir.join("../Resources/assets")),
    ];
    if let Some(path) = candidates.into_iter().flatten().find(|path| path.exists()) {
        *chatspeed_runtime_backend::constants::RESOURCE_DIR.write() = path;
    }
}

/// Starts the runtime: acquires the runtime-directory lock, assembles the single
/// canonical owner (opening the persistent database and running migrations),
/// binds the canonical control plane, publishes discovery, then serves until
/// shutdown.
///
/// Discovery is published by the control plane only after the lock, the owner
/// and the listener are ready, so a failure at any earlier step leaves no
/// discoverable endpoint behind and releases the lock.
pub async fn start_runtime(config: RuntimeConfig) -> Result<RuntimeHandle, RuntimeError> {
    if config.db_path() == Path::new(":memory:") {
        return Err(RuntimeError::Config(
            "the runtime database must be a persistent file, not :memory:".to_string(),
        ));
    }

    let startup_started = std::time::Instant::now();
    log::info!("[Runtime] acquiring runtime and database authority locks");
    let lock = RuntimeDirLock::acquire(config.runtime_dir())?;
    // Take the database authority before assembling the owner: a second runtime
    // with a different runtime directory but the same database must be refused
    // here, before it can open the database or publish discovery.
    let db_lock = RuntimeDbLock::acquire(config.db_path())?;
    let instance_id = lock.instance_id().to_string();

    // The standalone binary has no Tauri AppHandle to provide the bundled
    // resource directory. Resolve the packaged assets before owner assembly so
    // built-in agents and other resource-backed runtime data are synchronized
    // in production as well as in development.
    initialize_runtime_resource_dir();

    // Assemble the one owner before anything is discoverable. A migration or
    // database failure exits here with the lock released on return.
    log::info!(
        "[Runtime] assembling owner (db={}, app_data={})",
        config.db_path().display(),
        config.app_data_dir().display()
    );
    let owner = Arc::new(
        RuntimeOwner::assemble(RuntimeOwnerConfig {
            instance_id: instance_id.clone(),
            db_path: config.db_path().to_path_buf(),
            app_data_dir: config.app_data_dir().to_path_buf(),
        })
        .map_err(|error| RuntimeError::Assembly(error.to_string()))?,
    );
    log::info!(
        "[Runtime] owner assembly complete after {:?}",
        startup_started.elapsed()
    );

    // Fail-closed ordering: classify crash-interrupted capability operations and
    // load the desktop-free core tool surface *before* discovery is published or
    // a lease is accepted, so a client can never observe a half-initialized
    // owner. Recovery failures are logged, not fatal, because the durable
    // journal keeps the operation repairable on the next start.
    log::info!("[Runtime] recovering capability state and registering core tools");
    owner.recover_capability_state();
    if let Err(error) = owner.register_core_tools().await {
        let mut diagnostic = error.diagnostic();
        diagnostic.module = "runtime.startup".to_string();
        diagnostic.operation = "register_core_tools".to_string();
        log_error_diagnostic(&diagnostic);
    }
    log::info!("[Runtime] core tool registration complete");

    // The one-time startup normalization runs once, before the control plane is
    // published and before the first scheduler tick, with a cutoff captured here
    // and never re-read. A failure is fatal: the daemon must not continue into
    // the scheduler on the strength of a normalization that never ran.
    log::info!("[Runtime] normalizing automation schedules");
    let startup_cutoff = normalize_datetime_for_db(chrono::Local::now());
    if let Err(error) = owner
        .service()
        .automation_normalize_at_startup(&startup_cutoff)
    {
        return Err(RuntimeError::Assembly(format!(
            "automation startup normalization failed: {error}"
        )));
    }

    let leases = Arc::new(LeaseRegistry::new(config.lease_ttl()));

    // The one work-admission gate. It is installed into the owner's service
    // before the control plane is published, so every canonical work start is
    // fenced from the first accepted request, and the daemon drives its phase.
    // The same gate fences the lease registry's own registration, so a direct
    // (non-HTTP) caller cannot bypass the quiesce either.
    let gate = Arc::new(RuntimeLifecycleGate::new());
    owner
        .service()
        .install_admission(gate.clone() as Arc<dyn WorkAdmission>);
    leases.install_admission(gate.clone() as Arc<dyn WorkAdmission>);
    // The owner-held canonical work starts with no transport of their own (the
    // terminal manager's create/write) are fenced on the same gate.
    owner.install_admission(gate.clone() as Arc<dyn WorkAdmission>);

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (finished_tx, finished_rx) = watch::channel(false);

    // The one coordinated-shutdown authority every source funnels through: a
    // termination signal, the idle supervisor and an explicit request all close
    // the same gate and report the same operation. It reaches owned work through
    // a weak owner handle, so the cancelling phase can cancel it without keeping
    // the database owner alive past the drain.
    let shutdown = Arc::new(RuntimeShutdown {
        instance_id: instance_id.clone(),
        gate: gate.clone(),
        operation: std::sync::Mutex::new(None),
        trigger: shutdown_tx,
        owner: Arc::downgrade(&owner),
        finished: finished_rx.clone(),
        shutdown_budget: config.shutdown_budget(),
        force_budget: config.force_budget(),
        termination: config.termination_hook(),
    });

    // The runtime owns its own background work: capability reconcile, the
    // configured MCP servers, the automation tick and the chat-completion proxy.
    // It starts before the control plane so a connecting client can rely on the
    // same paths the desktop wires in `tauri::setup`. The gate fences the
    // automation tick's dispatch, which is a canonical non-HTTP work start.
    log::info!(
        "[Runtime] starting background services after {:?}",
        startup_started.elapsed()
    );
    let background =
        RuntimeBackground::start(&owner, env!("CARGO_PKG_VERSION").to_string(), gate.clone()).await;

    let runtime_control: Arc<dyn RuntimeControlPlane> = Arc::new(RuntimeControl {
        instance_id: instance_id.clone(),
        leases: leases.clone(),
        gate: gate.clone(),
        service: owner.service().clone(),
        shutdown: shutdown.clone(),
    });

    // The idle supervisor reads the automation summary through the same service
    // owner the serve task takes, so its handle is cloned before the owner moves.
    let idle_service = owner.service().clone();

    log::info!(
        "[Runtime] binding and publishing control plane after {:?}",
        startup_started.elapsed()
    );
    let control = match server::start_runtime_control_plane(
        owner.service().clone(),
        RuntimeControlPlaneOptions {
            discovery_dir: config.runtime_dir().to_path_buf(),
            leases: runtime_control,
            chat: Some(owner.chat_plane()),
            terminal: Some(owner.terminal_plane()),
            // The desktop registers its loopback Web MCP provider onto this
            // single lease-bound slot; the runtime reaches it as an ordinary MCP
            // server, so the fixed web tools stay on the canonical tool path.
            web_provider: Some(Arc::new(WebProviderRegistry::new(Arc::new(
                ToolManagerProviderInstaller::new(owner.chat_state().clone()),
            ))) as Arc<dyn RuntimeWebMcpPlane>),
        },
    )
    .await
    {
        Ok(control) => control,
        Err(error) => {
            // Nothing was published, so stop the tasks we already started and
            // release the lock with the assembly error.
            background.shutdown().await;
            return Err(RuntimeError::ControlPlane(error));
        }
    };

    log::info!(
        "[Runtime] ready after {:?}; pid={}",
        startup_started.elapsed(),
        std::process::id()
    );
    let port = control.port;
    let runtime_dir = config.runtime_dir().to_path_buf();
    let db_path = config.db_path().to_path_buf();

    // The serve task owns the owner, the background tasks, the lock and the
    // discovery cleanup: the lock is only released after the canonical HTTP
    // server has fully stopped and no runtime-owned task is left running. It
    // marks the operation fully stopped only once that drain is complete.
    let mut serve_shutdown = shutdown_rx.clone();
    let serve_instance = instance_id.clone();
    let serve_authority = shutdown.clone();
    let serve_budget = config.shutdown_budget();
    tokio::spawn(async move {
        let _ = serve_shutdown.changed().await;
        control.shutdown();
        // Bound the control-plane wait: the SSE broadcast ends open streams, so
        // this completes promptly, and the timeout keeps a stuck connection from
        // holding the lock forever.
        if tokio::time::timeout(serve_budget, control.wait())
            .await
            .is_err()
        {
            log::error!(
                "[Runtime] control plane did not stop within {:?}; withholding the `stopped` claim and holding the lock until a forced shutdown terminates this instance",
                serve_budget
            );
            std::future::pending::<()>().await;
        }
        // Stop the automation tick, the reconcile/MCP tasks and the ccproxy
        // listener only after the control plane has drained, so an in-flight
        // request is never cut off by background shutdown.
        if tokio::time::timeout(serve_budget, background.shutdown())
            .await
            .is_err()
        {
            log::error!(
                "[Runtime] background tasks did not stop within {:?}; withholding the `stopped` claim and holding the lock",
                serve_budget
            );
            std::future::pending::<()>().await;
        }
        // Never claim `stopped` while admitted work is still held: a false claim
        // would release the lock and let a successor open the database while this
        // instance still runs work. Park instead, holding the lock, until the work
        // drains or a forced request terminates the instance.
        if serve_authority.gate.active_admissions() > 0 {
            log::error!(
                "[Runtime] {} admission(s) still held after the control plane stopped; withholding the `stopped` claim and holding the lock",
                serve_authority.gate.active_admissions()
            );
            std::future::pending::<()>().await;
        }
        drop(owner);
        // Release the database authority only after the owner (and the control
        // plane that serves it) has fully drained, so a successor never opens
        // the database while this instance is still using it.
        drop(db_lock);
        drop(lock);
        serve_authority.mark_stopped();
        let _ = finished_tx.send_replace(true);
        log::info!(
            "[Runtime] control plane stopped (instance {})",
            serve_instance
        );
    });

    // Lease sweeper: expires TTLs on a bounded cadence so the supervisor sees
    // the count drop without waiting for a request.
    let sweep_leases = leases.clone();
    let mut sweep_shutdown = shutdown_rx.clone();
    let sweep_interval = config.sweep_interval();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(sweep_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = ticker.tick() => sweep_leases.sweep_expired(),
                _ = sweep_shutdown.changed() => break,
            }
        }
    });

    // Idle supervisor: grace-based shutdown only when clients, live workflows,
    // admissions and automation all agree the runtime is idle.
    tokio::spawn(supervise_idle(
        leases.subscribe(),
        shutdown_rx.clone(),
        idle_service,
        config.grace(),
        shutdown.clone(),
        gate.clone(),
    ));

    log::info!(
        "[Runtime] ready on port {} (instance {}, pid {}, runtime dir {}, db {})",
        port,
        instance_id,
        std::process::id(),
        runtime_dir.display(),
        db_path.display()
    );

    Ok(RuntimeHandle {
        port,
        instance_id,
        runtime_dir,
        db_path,
        leases,
        gate,
        shutdown,
        finished_rx,
    })
}

/// Shuts the instance down only when it is genuinely idle.
///
/// Four authorities decide: the live lease count (a registration during the
/// grace window wins and restarts a full grace), live workflow sessions owned
/// by the manager, the count of in-flight admissions on the one work gate, and
/// the authoritative automation keep-alive summary. The summary is re-read on
/// a bounded cadence and once more at grace expiry, so a newly enabled schedule
/// or a live run keeps the runtime alive without a lease and a last-moment work
/// arrival is observed instead of lost. A summary it cannot read is unknown and
/// keeps the runtime alive: an unobservable state is never mistaken for
/// "nothing to do".
///
/// The close itself is atomic: the supervisor captures the gate's activity epoch
/// before it waits the grace and re-checks that epoch (and that no admission is
/// held) inside [`RuntimeLifecycleGate::try_begin_idle`], so a registration, a
/// claim or any other admission that lands between the summary read and the
/// close refuses the close instead of racing it.
///
/// The idle path never forces and funnels into the same coordinated exit the
/// signal and an explicit request use.
async fn supervise_idle(
    mut count_rx: watch::Receiver<usize>,
    mut shutdown_rx: watch::Receiver<bool>,
    service: Arc<WorkflowApplicationService>,
    grace: Duration,
    shutdown: Arc<RuntimeShutdown>,
    gate: Arc<RuntimeLifecycleGate>,
) {
    // Throttles the unknown-health warning so a persistently unreadable store
    // does not flood the log at the poll cadence.
    let mut last_unknown_log = Instant::now()
        .checked_sub(Duration::from_secs(60))
        .unwrap_or_else(Instant::now);

    loop {
        if *shutdown_rx.borrow() {
            return;
        }

        // A live lease, in-flight admitted work, a live automation run or an
        // effective schedule keeps the runtime alive; so does an unreadable
        // summary.
        if keep_alive(&count_rx, &service, &gate, &mut last_unknown_log) {
            tokio::select! {
                changed = count_rx.changed() => {
                    if changed.is_err() {
                        return;
                    }
                }
                _ = tokio::time::sleep(SUMMARY_POLL) => {}
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        return;
                    }
                }
            }
            continue;
        }

        // Candidate idle: capture the activity epoch, then wait a full grace,
        // cancelling the pending grace on any activity so a short register /
        // release pair cannot drift the supervisor into an early exit.
        let expected_activity_epoch = gate.activity_epoch();
        let mut activity_during_grace = false;
        {
            let sleep = tokio::time::sleep(grace);
            tokio::pin!(sleep);
            loop {
                tokio::select! {
                    _ = &mut sleep => break,
                    changed = count_rx.changed() => {
                        if changed.is_err() {
                            return;
                        }
                        // A lease registration or release is activity: cancel the
                        // pending grace and re-evaluate from the top with a fresh
                        // epoch and a full grace.
                        activity_during_grace = true;
                        log::debug!("[Runtime] lease activity cancelled pending idle shutdown");
                        break;
                    }
                    _ = tokio::time::sleep(SUMMARY_POLL) => {
                        if keep_alive(&count_rx, &service, &gate, &mut last_unknown_log) {
                            activity_during_grace = true;
                            break;
                        }
                    }
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            return;
                        }
                    }
                }
            }
        }
        if activity_during_grace {
            continue;
        }

        // Final authoritative re-check at grace expiry, then an atomic close: the
        // activity epoch and the held-work count are compared under the gate
        // lock, so an admission or release that landed after the capture refuses
        // the close instead of being lost to a race.
        if keep_alive(&count_rx, &service, &gate, &mut last_unknown_log) {
            continue;
        }
        if !gate.try_begin_idle(expected_activity_epoch, 0) {
            log::debug!(
                "[Runtime] idle shutdown deferred: activity arrived after the idle epoch capture"
            );
            continue;
        }
        log::info!(
            "[Runtime] idle grace of {:?} elapsed with no leases, no admitted work and no automation keep-alive; shutting down",
            grace
        );
        let _ = shutdown.request(false);
        return;
    }
}

/// Whether the runtime must stay alive right now.
///
/// A live client lease, a live workflow session owned by this runtime, or any
/// in-flight admitted work keeps it alive; otherwise the authoritative
/// automation summary decides. A summary the runtime cannot read is unknown
/// and keeps it alive, so it is never shut down on an unobserved state.
fn keep_alive(
    count_rx: &watch::Receiver<usize>,
    service: &Arc<WorkflowApplicationService>,
    gate: &RuntimeLifecycleGate,
    last_unknown_log: &mut Instant,
) -> bool {
    if *count_rx.borrow() > 0 {
        return true;
    }
    // A held admission is work in flight (a chat turn, a workflow start, a
    // capability start, ...). The gate counts them even without a lease, so the
    // supervisor never shuts the runtime down mid-work.
    if gate.active_admissions() > 0 {
        return true;
    }
    // The HTTP admission ticket only covers starting/resuming the workflow.
    // Once the ReAct run loop owns the session, the manager is the authority
    // that it is still executing or waiting and must keep the runtime alive.
    if service.live_workflow_count() > 0 {
        return true;
    }
    let now = normalize_datetime_for_db(chrono::Local::now());
    match service.automation_runtime_summary(&now) {
        Ok(summary) => {
            if summary.health == AutomationRuntimeHealth::Unknown
                && last_unknown_log.elapsed() >= Duration::from_secs(30)
            {
                log::warn!(
                    "[Runtime] automation summary health is unknown; keeping the runtime alive and retrying"
                );
                *last_unknown_log = Instant::now();
            }
            summary.keep_alive
        }
        Err(error) => {
            if last_unknown_log.elapsed() >= Duration::from_secs(30) {
                log::warn!(
                    "[Runtime] automation summary unreadable; keeping the runtime alive: {error}"
                );
                *last_unknown_log = Instant::now();
            }
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chatspeed_contracts::{
        RUNTIME_CLIENT_HEADER, RUNTIME_LEASE_HEADER, RUNTIME_SHUTDOWN_PATH, WEB_MCP_CLIENT_HEADER,
        WEB_MCP_INSTANCE_HEADER, WEB_MCP_LEASE_HEADER, WEB_MCP_PROOF_HEADER, WEB_MCP_REGISTER_PATH,
        WEB_MCP_SERVER_NAME, WEB_MCP_UNREGISTER_PATH,
    };
    use chatspeed_runtime_backend::workflow::react::client::http::discovery;
    use serde_json::Value;

    /// Serializes tests that mutate the process environment.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// A cancellation that could not prove its owned teardown records `Failed`,
    /// so reaching `stopped` must not re-label it as a clean graceful exit.
    #[test]
    fn stopped_outcome_preserves_a_recorded_failure() {
        assert_eq!(
            stopped_outcome(false, None),
            Some(RuntimeShutdownOutcome::Graceful)
        );
        assert_eq!(
            stopped_outcome(true, None),
            Some(RuntimeShutdownOutcome::Forced)
        );
        assert_eq!(
            stopped_outcome(false, Some(RuntimeShutdownOutcome::Failed)),
            Some(RuntimeShutdownOutcome::Failed),
            "an unproven cancellation must stay failed, never graceful"
        );
        assert_eq!(
            stopped_outcome(true, Some(RuntimeShutdownOutcome::Failed)),
            Some(RuntimeShutdownOutcome::Failed),
            "a forced shutdown must not erase an unproven cancellation"
        );
    }

    fn lease_request(client_id: &str) -> ClientLeaseRequest {
        ClientLeaseRequest {
            client_id: client_id.to_string(),
            client_kind: "tauri".to_string(),
        }
    }

    fn short_config(dir: &Path, grace: Duration, ttl: Duration) -> RuntimeConfig {
        RuntimeConfig::with_runtime_dir(dir)
            .with_grace(grace)
            .with_lease_ttl(ttl)
    }

    // -- configuration resolution -------------------------------------------

    /// Clears the runtime path environment keys and restores them on drop.
    struct EnvGuard {
        saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
    }

    impl EnvGuard {
        fn clear() -> Self {
            let keys = [
                "CHATSPEED_RUNTIME_DIR",
                "CHATSPEED_RUNTIME_DB",
                "CHATSPEED_RUNTIME_APP_DATA_DIR",
                "CHATSPEED_HOME",
                "HOME",
                "USERPROFILE",
                "XDG_DATA_HOME",
            ];
            let saved = keys
                .iter()
                .map(|key| (*key, std::env::var_os(key)))
                .collect();
            for key in keys {
                std::env::remove_var(key);
            }
            Self { saved }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in &self.saved {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    #[test]
    fn default_config_uses_the_shared_profile_resolver() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let _env = EnvGuard::clear();

        let config = RuntimeConfig::new().expect("resolve the profile default");
        let launch = chatspeed_runtime_client::resolve_launch_config().expect("shared resolver");
        assert_eq!(config.runtime_dir(), launch.runtime_dir());
        assert_eq!(config.db_path(), launch.db_path());
        assert_eq!(config.app_data_dir(), launch.app_data_dir());

        // Development and production roots are isolated, and neither is a
        // `.`-relative fallback.
        assert!(config.app_data_dir().is_absolute());
        match chatspeed_runtime_client::build_profile() {
            chatspeed_runtime_client::BuildProfile::Debug => {
                assert!(config.app_data_dir().ends_with("dev_data"));
            }
            chatspeed_runtime_client::BuildProfile::Release => {
                assert!(config.app_data_dir().ends_with("ai.aidyou.chatspeed"));
            }
        }
    }

    #[test]
    fn from_env_honors_explicit_overrides() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let _env = EnvGuard::clear();
        std::env::set_var("CHATSPEED_RUNTIME_DIR", "/sandbox/rt");
        std::env::set_var("CHATSPEED_RUNTIME_DB", "/sandbox/db/chatspeed.db");
        std::env::set_var("CHATSPEED_RUNTIME_APP_DATA_DIR", "/sandbox/app");

        let config = RuntimeConfig::from_env().expect("from env");
        assert_eq!(config.runtime_dir(), Path::new("/sandbox/rt"));
        assert_eq!(config.db_path(), Path::new("/sandbox/db/chatspeed.db"));
        assert_eq!(config.app_data_dir(), Path::new("/sandbox/app"));
    }

    #[test]
    fn explicit_runtime_dir_isolates_db_and_app_data() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = RuntimeConfig::with_runtime_dir(dir.path());
        assert_eq!(config.runtime_dir(), dir.path());
        assert_eq!(config.db_path(), dir.path().join(DB_FILE_NAME));
        assert_eq!(config.app_data_dir(), dir.path());
    }

    // -- database authority -------------------------------------------------

    #[test]
    fn database_lock_path_normalizes_directory_aliases() {
        let dir = tempfile::tempdir().expect("tempdir");
        let canonical =
            database_lock_path(&dir.path().join(DB_FILE_NAME)).expect("canonical lock path");
        let aliased = database_lock_path(&dir.path().join(".").join(DB_FILE_NAME))
            .expect("aliased lock path");
        assert_eq!(canonical, aliased);
        assert!(canonical.starts_with(dir.path().canonicalize().expect("canonicalize")));
        // Taking the lock path never creates or touches the database.
        assert!(!dir.path().join(DB_FILE_NAME).exists());
    }

    #[cfg(unix)]
    #[test]
    fn database_lock_path_resolves_a_symlinked_database_to_one_target() {
        let real_dir = tempfile::tempdir().expect("real dir");
        let real_db = real_dir.path().join(DB_FILE_NAME);
        // An empty placeholder stands in for the database; only its metadata and
        // canonical path are ever consulted, never its content.
        fs::write(&real_db, b"").expect("placeholder database file");

        let alias_dir = tempfile::tempdir().expect("alias dir");
        let alias_db = alias_dir.path().join("alias.db");
        std::os::unix::fs::symlink(&real_db, &alias_db).expect("symlink alias");

        let real_lock = database_lock_path(&real_db).expect("real lock path");
        let alias_lock = database_lock_path(&alias_db).expect("alias lock path");
        // One database target, one authority lock, whatever the alias is called.
        assert_eq!(real_lock, alias_lock);

        // The kernel lock actually conflicts across the two aliases ...
        let first = RuntimeDbLock::acquire(&real_db).expect("acquire via the real path");
        let second = RuntimeDbLock::acquire(&alias_db);
        assert!(matches!(second, Err(RuntimeError::AlreadyRunning { .. })));

        // ... and is released on drop, so the alias can acquire it again.
        drop(first);
        let third = RuntimeDbLock::acquire(&alias_db).expect("re-acquire via the alias");
        drop(third);

        // Only the sidecar lock was created; the placeholder is untouched.
        assert!(real_db.exists());
    }

    #[cfg(unix)]
    #[test]
    fn database_lock_path_rejects_a_hard_linked_database() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dir.path().join(DB_FILE_NAME);
        fs::write(&db, b"").expect("placeholder database file");
        fs::hard_link(&db, dir.path().join("alias.db")).expect("hard link alias");

        // Two hard links to one inode cannot be unified by canonical path, so the
        // resolver fails closed instead of handing out two different locks.
        let result = database_lock_path(&db);
        assert!(matches!(result, Err(RuntimeError::Config(_))));
    }

    #[test]
    fn database_lock_refuses_a_second_holder_and_keeps_the_inode() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dir.path().join(DB_FILE_NAME);

        let lock = RuntimeDbLock::acquire(&db).expect("first acquire");
        let path = lock.path().to_path_buf();
        assert!(path.exists());

        let second = RuntimeDbLock::acquire(&db);
        assert!(matches!(second, Err(RuntimeError::AlreadyRunning { .. })));
        assert!(path.exists(), "a refused instance must not remove the lock");

        drop(lock);
        assert!(path.exists(), "the stable lock inode must not be removed");
        let third = RuntimeDbLock::acquire(&db).expect("re-acquire");
        drop(third);
        assert!(path.exists());
        // Only the sidecar lock exists; the database content was never created.
        assert!(!db.exists());
    }

    #[tokio::test]
    async fn a_shared_database_is_refused_across_runtime_directories() {
        let shared = tempfile::tempdir().expect("shared tempdir");
        let db = shared.path().join(DB_FILE_NAME);
        let first_dir = tempfile::tempdir().expect("first dir");
        let second_dir = tempfile::tempdir().expect("second dir");

        let handle = start_runtime(
            RuntimeConfig::with_runtime_dir(first_dir.path())
                .with_db_path(&db)
                .with_grace(Duration::from_secs(30))
                .with_lease_ttl(Duration::from_secs(30)),
        )
        .await
        .expect("first runtime owns the database");

        // A different runtime directory with the same database is refused before
        // it can open the database or publish discovery.
        let second = start_runtime(
            RuntimeConfig::with_runtime_dir(second_dir.path())
                .with_db_path(&db)
                .with_grace(Duration::from_secs(30))
                .with_lease_ttl(Duration::from_secs(30)),
        )
        .await;
        assert!(matches!(second, Err(RuntimeError::AlreadyRunning { .. })));
        assert!(!discovery::discovery_path_in(second_dir.path()).exists());

        handle.shutdown();
        tokio::time::timeout(Duration::from_secs(5), handle.wait())
            .await
            .expect("runtime stops");

        // Once the owner has drained, the same database can be acquired again.
        let restart = start_runtime(
            RuntimeConfig::with_runtime_dir(second_dir.path())
                .with_db_path(&db)
                .with_grace(Duration::from_secs(30))
                .with_lease_ttl(Duration::from_secs(30)),
        )
        .await
        .expect("database authority released on drop");
        restart.shutdown();
        tokio::time::timeout(Duration::from_secs(5), restart.wait())
            .await
            .expect("restart stops");
    }

    // -- lease proofs -------------------------------------------------------

    #[test]
    fn validate_resolves_only_a_matching_live_lease() {
        let registry = LeaseRegistry::new(Duration::from_secs(60));
        let response = registry
            .register(&lease_request("tauri-main"))
            .expect("register");

        let lease = registry
            .validate("tauri-main", &response.lease_id)
            .expect("matching proof resolves");
        assert_eq!(lease.client_id, "tauri-main");
        assert_eq!(lease.lease_id, response.lease_id);
        assert_eq!(lease.client_kind, "tauri");

        assert!(matches!(
            registry.validate("tauri-main", "not-the-lease"),
            Err(LeaseError::NotFound(_))
        ));
        assert!(matches!(
            registry.validate("unknown-client", &response.lease_id),
            Err(LeaseError::NotFound(_))
        ));

        // An expired lease no longer resolves.
        let expiring = LeaseRegistry::new(Duration::from_millis(1));
        let response = expiring
            .register(&lease_request("tauri-main"))
            .expect("register");
        std::thread::sleep(Duration::from_millis(10));
        assert!(matches!(
            expiring.validate("tauri-main", &response.lease_id),
            Err(LeaseError::NotFound(_))
        ));
    }

    // -- lock ---------------------------------------------------------------

    #[test]
    fn lock_rejects_duplicate_and_releases_on_owner_drop() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = instance_lock_path(dir.path());

        let lock = RuntimeDirLock::acquire(dir.path()).expect("first acquire");
        assert!(path.exists());
        assert!(!lock.instance_id().is_empty());

        let second = RuntimeDirLock::acquire(dir.path());
        assert!(matches!(second, Err(RuntimeError::AlreadyRunning { .. })));
        assert!(path.exists(), "a refused instance must not remove the lock");

        drop(lock);
        assert!(path.exists(), "the stable lock inode must not be removed");

        let third = RuntimeDirLock::acquire(dir.path()).expect("re-acquire");
        drop(third);
        assert!(path.exists());
    }

    #[test]
    fn stale_lock_with_impossible_pid_is_taken_over() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = instance_lock_path(dir.path());
        let ghost = LockRecord {
            instance_id: "ghost".to_string(),
            pid: u32::MAX,
            started_at: "unix-0".to_string(),
        };
        let body = serde_json::to_vec_pretty(&ghost).expect("serialize ghost");
        fs::write(&path, body).expect("write ghost lock");

        let lock = RuntimeDirLock::acquire(dir.path()).expect("take over stale lock");
        assert_ne!(lock.instance_id(), "ghost");
        let record = read_lock_record(&path).expect("read taken-over lock");
        assert_eq!(record.instance_id, lock.instance_id());
        assert_eq!(record.pid, std::process::id());

        drop(lock);
        assert!(path.exists());
        let _next = RuntimeDirLock::acquire(dir.path()).expect("stale takeover releases owner");
    }

    #[test]
    fn malformed_lock_fails_closed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = instance_lock_path(dir.path());
        fs::write(&path, b"not-json").expect("write malformed lock");

        let result = RuntimeDirLock::acquire(dir.path());
        assert!(matches!(result, Err(RuntimeError::LockMalformed { .. })));
        assert!(path.exists(), "unverifiable lock must be left untouched");
    }

    #[cfg(unix)]
    #[test]
    fn lock_and_runtime_dir_are_current_user_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let _lock = RuntimeDirLock::acquire(dir.path()).expect("acquire lock");

        let dir_mode = fs::metadata(dir.path())
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700);
        let lock_mode = fs::metadata(instance_lock_path(dir.path()))
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(lock_mode, 0o600);
    }

    // -- database / fail-closed --------------------------------------------

    #[tokio::test]
    async fn in_memory_database_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let result =
            start_runtime(RuntimeConfig::with_runtime_dir(dir.path()).with_db_path(":memory:"))
                .await;
        assert!(matches!(result, Err(RuntimeError::Config(_))));
        assert!(
            !discovery::discovery_path_in(dir.path()).exists(),
            "a refused configuration must not publish discovery"
        );
    }

    #[tokio::test]
    async fn a_database_failure_exits_without_publishing_discovery() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Point the database at a directory so `Connection::open` fails; the
        // runtime must fail closed instead of falling back to :memory:.
        let blocked = dir.path().join("db-is-a-directory");
        fs::create_dir(&blocked).expect("create blocking directory");
        let result =
            start_runtime(RuntimeConfig::with_runtime_dir(dir.path()).with_db_path(&blocked)).await;
        assert!(matches!(result, Err(RuntimeError::Assembly(_))));
        assert!(!discovery::discovery_path_in(dir.path()).exists());
        // The lock was released, so the same directory can be re-acquired.
        let _next = RuntimeDirLock::acquire(dir.path()).expect("owner lock was released");
    }

    // -- control plane over real HTTP --------------------------------------

    struct Http {
        client: reqwest::Client,
        port: u16,
        token: String,
    }

    impl Http {
        fn url(&self, path: &str) -> String {
            format!("http://127.0.0.1:{}{}", self.port, path)
        }

        async fn get(&self, path: &str) -> reqwest::Response {
            self.client
                .get(self.url(path))
                .header("Authorization", format!("Bearer {}", self.token))
                .send()
                .await
                .expect("get request")
        }

        async fn post(&self, path: &str, body: &str) -> reqwest::Response {
            self.client
                .post(self.url(path))
                .header("Authorization", format!("Bearer {}", self.token))
                .header("Content-Type", "application/json")
                .body(body.to_string())
                .send()
                .await
                .expect("post request")
        }

        /// Sends a Web MCP provider control request with the header-only proof
        /// the production contract requires (never in the URL, query or body).
        async fn web_mcp(
            &self,
            path: &str,
            body: &str,
            client_id: &str,
            lease_id: &str,
        ) -> reqwest::Response {
            self.client
                .post(self.url(path))
                .header("Authorization", format!("Bearer {}", self.token))
                .header("Content-Type", "application/json")
                .header(WEB_MCP_PROOF_HEADER, "proof-a")
                .header(WEB_MCP_CLIENT_HEADER, client_id)
                .header(WEB_MCP_LEASE_HEADER, lease_id)
                .header(WEB_MCP_INSTANCE_HEADER, "desktop-instance-a")
                .body(body.to_string())
                .send()
                .await
                .expect("web-mcp request")
        }

        /// Reads the runtime-owned `get_available_tools` data command.
        async fn available_tools(&self) -> Value {
            let response = self
                .post("/control/v1/data-commands/get_available_tools", "{}")
                .await;
            assert_eq!(response.status(), reqwest::StatusCode::OK);
            response.json().await.expect("available tools json")
        }

        /// Sends a coordinated-shutdown request with the optional proof headers
        /// and idempotency key, so a test can drive every refusal path.
        async fn shutdown(
            &self,
            body: &str,
            client_id: Option<&str>,
            lease_id: Option<&str>,
            key: Option<&str>,
        ) -> reqwest::Response {
            let mut request = self
                .client
                .post(self.url(RUNTIME_SHUTDOWN_PATH))
                .header("Authorization", format!("Bearer {}", self.token))
                .header("Content-Type", "application/json");
            if let Some(client_id) = client_id {
                request = request.header(RUNTIME_CLIENT_HEADER, client_id);
            }
            if let Some(lease_id) = lease_id {
                request = request.header(RUNTIME_LEASE_HEADER, lease_id);
            }
            if let Some(key) = key {
                request = request.header("Idempotency-Key", key);
            }
            request
                .body(body.to_string())
                .send()
                .await
                .expect("shutdown request")
        }
    }

    #[tokio::test]
    async fn standalone_runtime_serves_canonical_meta_and_lease_routes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let handle = start_runtime(short_config(
            dir.path(),
            Duration::from_secs(30),
            Duration::from_secs(30),
        ))
        .await
        .expect("start runtime");

        // Discovery is published with the same instance id the lock claimed.
        let document = discovery::read_discovery_in(dir.path()).expect("discovery published");
        assert_eq!(document.server_instance_id, handle.instance_id());
        assert_eq!(document.port, handle.port());
        assert!(instance_lock_path(dir.path()).exists());

        // The runtime owns a real persistent database, not :memory:.
        assert_eq!(handle.db_path(), dir.path().join(DB_FILE_NAME));
        assert!(
            handle.db_path().is_file(),
            "the runtime database file exists"
        );

        let http = Http {
            client: reqwest::Client::new(),
            port: handle.port(),
            token: document.token.clone(),
        };

        // `/meta` identifies the standalone runtime and the locked instance.
        let response = http.get("/control/v1/meta").await;
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let meta: Value = response.json().await.expect("meta json");
        assert_eq!(meta["service"], SERVICE_NAME);
        assert_eq!(meta["server_instance_id"], handle.instance_id());
        assert_eq!(meta["pid"], std::process::id());

        // The same server serves the canonical workflow and automation routes
        // the clients rely on, not only `/meta` and the lease lifecycle.
        let response = http.get("/control/v1/agents").await;
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let agents: Value = response.json().await.expect("agents json");
        assert!(agents.is_array(), "agents is a JSON list");

        let response = http.get("/control/v1/automations").await;
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let automations: Value = response.json().await.expect("automations json");
        assert!(automations.is_array(), "automations is a JSON list");

        // Lease lifecycle over the same server.
        let response = http
            .post(
                "/control/v1/clients/register",
                r#"{"client_id":"client-a","client_kind":"tauri"}"#,
            )
            .await;
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let registered: Value = response.json().await.expect("lease json");
        let lease_id = registered["lease_id"]
            .as_str()
            .expect("lease id")
            .to_string();
        assert!(!lease_id.is_empty());
        assert_eq!(handle.leases().active_count(), 1);

        let response = http.post("/control/v1/clients/client-a/renew", "").await;
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let renewed: Value = response.json().await.expect("renew json");
        assert_eq!(renewed["lease_id"], lease_id);

        let response = http.post("/control/v1/clients/client-a/release", "").await;
        assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);
        assert_eq!(handle.leases().active_count(), 0);

        let response = http.post("/control/v1/clients/client-a/renew", "").await;
        assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);

        handle.shutdown();
        tokio::time::timeout(Duration::from_secs(5), handle.wait())
            .await
            .expect("runtime stops");
        assert!(!discovery::discovery_path_in(dir.path()).exists());
        let _next = RuntimeDirLock::acquire(dir.path()).expect("owner lock was released");
    }

    #[tokio::test]
    async fn standalone_runtime_mounts_the_owner_chat_and_model_routes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let handle = start_runtime(short_config(
            dir.path(),
            Duration::from_secs(30),
            Duration::from_secs(30),
        ))
        .await
        .expect("start runtime");
        let document = discovery::read_discovery_in(dir.path()).expect("discovery published");
        let http = Http {
            client: reqwest::Client::new(),
            port: handle.port(),
            token: document.token.clone(),
        };

        // The chat/model surface is mounted on the one owner: an unreachable
        // loopback endpoint reaches the canonical executor and maps its failure,
        // so the route is never reported as `runtime_unavailable`.
        let response = http
            .post(
                "/control/v1/models/list",
                r#"{"api_protocol":"openai","api_url":"http://127.0.0.1:1/v1","api_key":"k"}"#,
            )
            .await;
        assert_eq!(response.status(), reqwest::StatusCode::BAD_GATEWAY);
        let envelope: Value = response.json().await.expect("error envelope");
        assert_eq!(envelope["error"]["code"], "model_list_failed");

        // Stopping an unknown chat is a typed no-op on the owner, not an
        // unavailable route.
        let response = http
            .post("/control/v1/chats/chat-x/stop", r#"{"chat_id":"chat-x"}"#)
            .await;
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let stopped: Value = response.json().await.expect("stop json");
        assert_eq!(stopped["chat_id"], "chat-x");

        handle.shutdown();
        tokio::time::timeout(Duration::from_secs(5), handle.wait())
            .await
            .expect("runtime stops");
    }

    // -- desktop Web MCP provider end to end --------------------------------

    /// A loopback MCP provider that speaks the real streamable-HTTP transport
    /// and exposes exactly the two reserved web tool aliases. It is the runtime's
    /// remote peer in the handshake, not a fake installer, so the test covers the
    /// wire path instead of a shortened seam.
    #[derive(Clone)]
    struct LoopbackWebProvider;

    impl rmcp::ServerHandler for LoopbackWebProvider {
        fn get_info(&self) -> rmcp::model::ServerConfig {
            let mut info = rmcp::model::ServerConfig::default();
            info.capabilities = rmcp::model::ServerCapabilities::builder()
                .enable_tools()
                .build();
            info.server_info = rmcp::model::Implementation::new("loopback-web-provider", "1.0.0");
            info
        }

        async fn list_tools(
            &self,
            _request: Option<rmcp::model::PaginatedRequestParams>,
            _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
        ) -> Result<rmcp::model::ListToolsResult, rmcp::ErrorData> {
            Ok(rmcp::model::ListToolsResult::with_all_items(vec![
                rmcp::model::Tool::new(
                    "web_fetch",
                    "fetch a URL through the desktop provider",
                    Arc::new(serde_json::Map::new()),
                ),
                rmcp::model::Tool::new(
                    "web_search",
                    "search the web through the desktop provider",
                    Arc::new(serde_json::Map::new()),
                ),
            ]))
        }
    }

    /// Binds the provider fixture to an ephemeral loopback port.
    async fn spawn_loopback_web_provider() -> (u16, tokio::task::JoinHandle<()>) {
        let factory = move || Ok(LoopbackWebProvider);
        let service = rmcp::transport::streamable_http_server::StreamableHttpService::new(
            factory,
            Arc::new(
                rmcp::transport::streamable_http_server::session::local::LocalSessionManager::default(
                ),
            ),
            rmcp::transport::streamable_http_server::StreamableHttpServerConfig::default(),
        );
        let router = axum::Router::new().nest_service("/mcp", service);
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind the loopback Web MCP provider");
        let port = listener.local_addr().expect("provider address").port();
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        (port, task)
    }

    /// Canonical ids of the web tools the runtime currently advertises.
    fn web_provider_tool_ids(tools: &Value) -> Vec<String> {
        let prefix = format!("{WEB_MCP_SERVER_NAME}__MCP__");
        tools
            .as_array()
            .expect("tool list")
            .iter()
            .filter_map(|tool| tool["id"].as_str())
            .filter(|id| id.starts_with(&prefix))
            .map(str::to_string)
            .collect()
    }

    /// The reported regression: after the runtime migration the desktop Web MCP
    /// provider must still register onto the standalone runtime, its two fixed
    /// tools must reach `get_available_tools` as canonical MCP entries, and
    /// unregistering must clear both the slot and the tools.
    ///
    /// This runs the real path end to end: an isolated `start_runtime`, a real
    /// `tauri` lease, a real loopback streamable-HTTP MCP server, and the
    /// canonical control-plane routes. It never bypasses the production
    /// `ToolManager::register_web_mcp_provider` handshake.
    #[tokio::test]
    async fn standalone_runtime_registers_discovers_and_removes_the_web_mcp_provider() {
        let dir = tempfile::tempdir().expect("tempdir");
        let handle = start_runtime(short_config(
            dir.path(),
            Duration::from_secs(30),
            Duration::from_secs(30),
        ))
        .await
        .expect("start runtime");
        let document = discovery::read_discovery_in(dir.path()).expect("discovery published");
        let http = Http {
            client: reqwest::Client::new(),
            port: handle.port(),
            token: document.token.clone(),
        };

        // A live `tauri` lease is the only identity allowed to register the
        // provider onto the runtime.
        let response = http
            .post(
                "/control/v1/clients/register",
                r#"{"client_id":"desktop-a","client_kind":"tauri"}"#,
            )
            .await;
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let registered: Value = response.json().await.expect("lease json");
        let lease_id = registered["lease_id"]
            .as_str()
            .expect("lease id")
            .to_string();

        // No provider yet: the status route answers a typed `unavailable`, never
        // a generic 404 route miss.
        let response = http.get("/control/v1/web-mcp/status").await;
        assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
        let envelope: Value = response.json().await.expect("status error envelope");
        assert_eq!(envelope["error"]["code"], "unavailable");

        // A fresh runtime advertises no web tool before the provider exists.
        assert!(
            web_provider_tool_ids(&http.available_tools().await).is_empty(),
            "a provider-less runtime must not advertise the web tools"
        );

        let (provider_port, provider_task) = spawn_loopback_web_provider().await;

        // Registering performs the real MCP handshake and the fail-closed first
        // tool discovery before it returns, so a 200 already proves discovery.
        let body = format!(r#"{{"port":{provider_port}}}"#);
        let response = http
            .web_mcp(WEB_MCP_REGISTER_PATH, &body, "desktop-a", &lease_id)
            .await;
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let registered: Value = response.json().await.expect("register json");
        assert_eq!(registered["server_name"], WEB_MCP_SERVER_NAME);
        assert_eq!(registered["generation"], 1);

        // `get_available_tools` reaches the canonical ToolManager and reports
        // exactly the two reserved aliases with their canonical MCP ids.
        let tools = http.available_tools().await;
        let mcp: Vec<&Value> = tools
            .as_array()
            .expect("tool list")
            .iter()
            .filter(|tool| tool["category"] == "MCP")
            .collect();
        assert_eq!(mcp.len(), 2, "unexpected MCP tool set: {mcp:?}");
        let mut ids = mcp
            .iter()
            .map(|tool| tool["id"].as_str().expect("tool id").to_string())
            .collect::<Vec<_>>();
        ids.sort();
        assert_eq!(
            ids,
            vec![
                format!("{WEB_MCP_SERVER_NAME}__MCP__web_fetch"),
                format!("{WEB_MCP_SERVER_NAME}__MCP__web_search"),
            ]
        );
        let mut names = mcp
            .iter()
            .map(|tool| tool["name"].as_str().expect("tool name").to_string())
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(
            names,
            vec!["web_fetch".to_string(), "web_search".to_string()]
        );
        assert!(mcp.iter().all(|tool| tool["scope"] == "both"));

        // The redacted status view reports the installed slot and no secret.
        let response = http.get("/control/v1/web-mcp/status").await;
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let status_text = response.text().await.expect("status text");
        let status: Value = serde_json::from_str(&status_text).expect("status json");
        assert_eq!(status["server_name"], WEB_MCP_SERVER_NAME);
        assert_eq!(status["client_id"], "desktop-a");
        assert_eq!(status["port"], provider_port);
        assert!(
            !status_text.contains("proof-a"),
            "the status view leaked the provider proof token: {status_text}"
        );

        // Unregistering removes both the slot and the discovered tools.
        let response = http
            .web_mcp(WEB_MCP_UNREGISTER_PATH, "{}", "desktop-a", &lease_id)
            .await;
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert!(
            web_provider_tool_ids(&http.available_tools().await).is_empty(),
            "unregister must clear the discovered web tools"
        );
        let response = http.get("/control/v1/web-mcp/status").await;
        assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
        let envelope: Value = response.json().await.expect("status error envelope");
        assert_eq!(envelope["error"]["code"], "unavailable");

        provider_task.abort();
        handle.shutdown();
        tokio::time::timeout(Duration::from_secs(5), handle.wait())
            .await
            .expect("runtime stops");
        assert!(!discovery::discovery_path_in(dir.path()).exists());
        let _next = RuntimeDirLock::acquire(dir.path()).expect("owner lock was released");
    }

    #[tokio::test]
    async fn lease_routes_enforce_bearer_origin_and_url_token_rules() {
        let dir = tempfile::tempdir().expect("tempdir");
        let handle = start_runtime(short_config(
            dir.path(),
            Duration::from_secs(30),
            Duration::from_secs(30),
        ))
        .await
        .expect("start runtime");
        let document = discovery::read_discovery_in(dir.path()).expect("discovery published");
        let base = format!("http://127.0.0.1:{}", handle.port());
        let client = reqwest::Client::new();

        // No bearer token.
        let response = client
            .get(format!("{base}/control/v1/meta"))
            .send()
            .await
            .expect("request");
        assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);

        // Browser Origin is rejected before the token is even compared.
        let response = client
            .get(format!("{base}/control/v1/meta"))
            .header("Authorization", format!("Bearer {}", document.token))
            .header("Origin", "http://evil.example")
            .send()
            .await
            .expect("request");
        assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);

        // Credentials in the URL are rejected.
        let response = client
            .get(format!("{base}/control/v1/meta?token={}", document.token))
            .header("Authorization", format!("Bearer {}", document.token))
            .send()
            .await
            .expect("request");
        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);

        handle.shutdown();
        tokio::time::timeout(Duration::from_secs(5), handle.wait())
            .await
            .expect("runtime stops");
    }

    #[tokio::test]
    async fn start_refuses_second_instance_without_publishing_discovery() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = short_config(dir.path(), Duration::from_secs(30), Duration::from_secs(30));
        let handle = start_runtime(config.clone()).await.expect("start runtime");

        // Remove our own discovery so the assertion below is meaningful.
        fs::remove_file(discovery::discovery_path_in(dir.path())).expect("remove discovery");

        let second = start_runtime(config).await;
        assert!(matches!(second, Err(RuntimeError::AlreadyRunning { .. })));
        assert!(!discovery::discovery_path_in(dir.path()).exists());

        handle.shutdown();
        tokio::time::timeout(Duration::from_secs(5), handle.wait())
            .await
            .expect("runtime stops");
        assert!(instance_lock_path(dir.path()).exists());
        let _next = RuntimeDirLock::acquire(dir.path()).expect("owner lock was released");
    }

    // -- lifecycle ---------------------------------------------------------

    #[tokio::test]
    async fn idle_grace_shuts_down_after_last_release() {
        let dir = tempfile::tempdir().expect("tempdir");
        let handle = start_runtime(short_config(
            dir.path(),
            Duration::from_millis(400),
            Duration::from_secs(5),
        ))
        .await
        .expect("start runtime");

        handle
            .leases()
            .register(&lease_request("client-a"))
            .expect("register");
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(
            !handle.is_finished(),
            "an active lease keeps the runtime up"
        );
        assert!(discovery::discovery_path_in(dir.path()).exists());

        handle.leases().release("client-a").expect("release");
        tokio::time::timeout(Duration::from_secs(5), handle.wait())
            .await
            .expect("grace expires and runtime stops");
        assert!(!discovery::discovery_path_in(dir.path()).exists());
    }

    #[tokio::test]
    async fn desktop_exit_release_shuts_down_idle_runtime_before_grace() {
        let dir = tempfile::tempdir().expect("tempdir");
        let handle = start_runtime(short_config(
            dir.path(),
            Duration::from_secs(30),
            Duration::from_secs(5),
        ))
        .await
        .expect("start runtime");
        let document = discovery::read_discovery_in(dir.path()).expect("discovery");
        let http = Http {
            client: reqwest::Client::new(),
            port: handle.port(),
            token: document.token,
        };
        let response = http
            .post(
                "/control/v1/clients/register",
                r#"{"client_id":"tauri-main","client_kind":"tauri"}"#,
            )
            .await;
        assert_eq!(response.status(), reqwest::StatusCode::OK);

        let response = http
            .post(
                "/control/v1/clients/tauri-main/release?shutdown_if_idle=true",
                "",
            )
            .await;
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let result: Value = response.json().await.expect("release result");
        assert_eq!(result["idle_shutdown"], true);
        tokio::time::timeout(Duration::from_secs(10), handle.wait())
            .await
            .expect("idle runtime exits before its 30-second grace");
    }

    #[tokio::test]
    async fn desktop_exit_release_keeps_runtime_with_another_lease() {
        let dir = tempfile::tempdir().expect("tempdir");
        let handle = start_runtime(short_config(
            dir.path(),
            Duration::from_secs(30),
            Duration::from_secs(5),
        ))
        .await
        .expect("start runtime");
        let document = discovery::read_discovery_in(dir.path()).expect("discovery");
        let http = Http {
            client: reqwest::Client::new(),
            port: handle.port(),
            token: document.token,
        };
        for client_id in ["tauri-main", "other-client"] {
            let response = http
                .post(
                    "/control/v1/clients/register",
                    &format!(r#"{{"client_id":"{client_id}","client_kind":"tauri"}}"#),
                )
                .await;
            assert_eq!(response.status(), reqwest::StatusCode::OK);
        }

        let response = http
            .post(
                "/control/v1/clients/tauri-main/release?shutdown_if_idle=true",
                "",
            )
            .await;
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let result: Value = response.json().await.expect("release result");
        assert_eq!(result["idle_shutdown"], false);
        let status: Value = http
            .get("/control/v1/runtime/status")
            .await
            .json()
            .await
            .expect("status");
        assert_eq!(status["phase"], "running");
        assert_eq!(status["active_leases"], 1);

        handle.shutdown();
        tokio::time::timeout(Duration::from_secs(10), handle.wait())
            .await
            .expect("runtime stops");
    }

    #[tokio::test]
    async fn non_desktop_release_cannot_request_immediate_idle_shutdown() {
        let dir = tempfile::tempdir().expect("tempdir");
        let handle = start_runtime(short_config(
            dir.path(),
            Duration::from_secs(30),
            Duration::from_secs(5),
        ))
        .await
        .expect("start runtime");
        let document = discovery::read_discovery_in(dir.path()).expect("discovery");
        let http = Http {
            client: reqwest::Client::new(),
            port: handle.port(),
            token: document.token,
        };
        let response = http
            .post(
                "/control/v1/clients/register",
                r#"{"client_id":"cscli-1","client_kind":"cli"}"#,
            )
            .await;
        assert_eq!(response.status(), reqwest::StatusCode::OK);

        let response = http
            .post(
                "/control/v1/clients/cscli-1/release?shutdown_if_idle=true",
                "",
            )
            .await;
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let result: Value = response.json().await.expect("release result");
        assert_eq!(result["idle_shutdown"], false);
        assert!(!handle.is_finished());

        handle.shutdown();
        tokio::time::timeout(Duration::from_secs(10), handle.wait())
            .await
            .expect("runtime stops");
    }

    #[tokio::test]
    async fn new_register_within_grace_cancels_shutdown() {
        let dir = tempfile::tempdir().expect("tempdir");
        let handle = start_runtime(short_config(
            dir.path(),
            Duration::from_millis(400),
            Duration::from_secs(5),
        ))
        .await
        .expect("start runtime");

        handle
            .leases()
            .register(&lease_request("client-a"))
            .expect("register");
        handle.leases().release("client-a").expect("release");

        // Register again inside the grace window; the pending shutdown must be
        // cancelled for the full grace after the new registration.
        tokio::time::sleep(Duration::from_millis(80)).await;
        handle
            .leases()
            .register(&lease_request("client-b"))
            .expect("re-register");
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(
            !handle.is_finished(),
            "a registration inside grace must cancel shutdown"
        );
        assert!(discovery::discovery_path_in(dir.path()).exists());

        handle.leases().release("client-b").expect("release");
        tokio::time::timeout(Duration::from_secs(5), handle.wait())
            .await
            .expect("runtime stops after release");
        assert!(!discovery::discovery_path_in(dir.path()).exists());
    }

    /// A held admission is in-flight work even with no client lease, so the idle
    /// supervisor must keep the runtime alive until it drains.
    #[tokio::test]
    async fn an_active_admission_keeps_the_runtime_alive_without_a_lease() {
        let dir = tempfile::tempdir().expect("tempdir");
        let handle = start_runtime(short_config(
            dir.path(),
            Duration::from_millis(300),
            Duration::from_secs(5),
        ))
        .await
        .expect("start runtime");

        // Simulate a canonical work item in flight: a chat turn holds a ticket.
        let ticket = handle
            .gate()
            .admit(WorkKind::ChatTurn)
            .expect("admit while running");

        // Past the grace with no lease, the in-flight work still holds it open.
        tokio::time::sleep(Duration::from_millis(900)).await;
        assert!(
            !handle.is_finished(),
            "an in-flight admission keeps the runtime alive"
        );
        assert!(discovery::discovery_path_in(dir.path()).exists());

        drop(ticket);
        tokio::time::timeout(Duration::from_secs(5), handle.wait())
            .await
            .expect("the runtime stops after the work drains");
    }

    /// A short register/release inside the grace window is activity: it must
    /// re-arm the whole grace instead of letting the supervisor exit early.
    #[tokio::test]
    async fn a_short_register_release_rearms_the_full_grace() {
        let dir = tempfile::tempdir().expect("tempdir");
        let handle = start_runtime(short_config(
            dir.path(),
            Duration::from_millis(500),
            Duration::from_secs(5),
        ))
        .await
        .expect("start runtime");

        // The initial grace started at t0. At t300 a client registers and releases
        // immediately: the pending grace must be cancelled and re-armed in full,
        // so the runtime is still up at t600 (the old break-then-final-check path
        // would have stopped it near t300).
        tokio::time::sleep(Duration::from_millis(300)).await;
        handle
            .leases()
            .register(&lease_request("client-a"))
            .expect("register");
        handle.leases().release("client-a").expect("release");

        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            !handle.is_finished(),
            "a short register/release must re-arm the full grace"
        );

        tokio::time::timeout(Duration::from_secs(5), handle.wait())
            .await
            .expect("the runtime stops after the re-armed grace");
    }

    /// A lease registration is fenced at the lease registry itself, so a direct,
    /// non-HTTP caller is refused on the gate while the runtime is quiescing.
    #[tokio::test]
    async fn a_direct_lease_registration_is_refused_once_quiesced() {
        let dir = tempfile::tempdir().expect("tempdir");
        let handle = start_runtime(short_config(
            dir.path(),
            Duration::from_secs(30),
            Duration::from_secs(30),
        ))
        .await
        .expect("start runtime");

        assert!(handle.gate().begin_quiesce());
        let error = handle
            .leases()
            .register(&lease_request("client-late"))
            .expect_err("a quiescing runtime refuses a direct registration");
        assert!(
            matches!(error, LeaseError::Stopping(_)),
            "refused with the stable stopping error"
        );

        handle.shutdown();
        tokio::time::timeout(Duration::from_secs(5), handle.wait())
            .await
            .expect("runtime stops");
    }

    #[tokio::test]
    async fn lease_ttl_expiry_is_swept_and_triggers_grace() {
        let dir = tempfile::tempdir().expect("tempdir");
        let handle = start_runtime(short_config(
            dir.path(),
            Duration::from_millis(200),
            Duration::from_millis(120),
        ))
        .await
        .expect("start runtime");

        handle
            .leases()
            .register(&lease_request("client-a"))
            .expect("register");

        // Without renewing, the sweeper must expire the lease and the grace
        // must then stop the instance; this deliberately never touches the
        // registry from the test so the sweeper is the only actor.
        tokio::time::timeout(Duration::from_secs(5), handle.wait())
            .await
            .expect("ttl expiry plus grace stops the runtime");
        assert!(!discovery::discovery_path_in(dir.path()).exists());
        assert!(instance_lock_path(dir.path()).exists());
        let _next = RuntimeDirLock::acquire(dir.path()).expect("owner lock was released");
    }

    // -- runtime background ownership --------------------------------------

    fn assemble_owner(dir: &Path) -> RuntimeOwner {
        RuntimeOwner::assemble(RuntimeOwnerConfig {
            instance_id: "test-instance".to_string(),
            db_path: dir.join(DB_FILE_NAME),
            app_data_dir: dir.to_path_buf(),
        })
        .expect("assemble the runtime owner")
    }

    /// The runtime, not the desktop, must populate the agents table: the
    /// `tauri::setup` hook that used to call `sync_builtin_agents_if_needed` is
    /// gone, so assembly owns it before the runtime is discoverable.
    #[tokio::test]
    async fn runtime_owner_synchronizes_bundled_builtin_agents() {
        let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
        assert!(
            assets.join("agents").is_dir(),
            "bundled built-in agent assets must exist at {:?}",
            assets
        );

        let dir = tempfile::tempdir().expect("tempdir");
        // Resource resolution is process-global; point it at the bundled assets
        // for the duration of assembly and restore it immediately afterwards.
        // No other test in this binary touches `RESOURCE_DIR`, so the window is
        // race-free.
        let original = chatspeed_runtime_backend::constants::RESOURCE_DIR
            .read()
            .clone();
        *chatspeed_runtime_backend::constants::RESOURCE_DIR.write() = assets;
        let owner = assemble_owner(dir.path());
        *chatspeed_runtime_backend::constants::RESOURCE_DIR.write() = original;

        let agents = owner.main_store().get_all_agents().expect("load agents");
        assert!(
            !agents.is_empty(),
            "the bundled built-in agents were written to the store during assembly"
        );
        assert!(
            agents.iter().all(|agent| agent.id.starts_with("builtin:")),
            "every synchronized agent keeps its builtin: id"
        );
        assert!(
            agents.iter().any(|agent| agent.is_system == Some(true)),
            "built-in agents are marked as system agents"
        );
    }

    #[tokio::test]
    async fn runtime_core_tool_surface_is_desktop_free_and_requires_web_provider() {
        let dir = tempfile::tempdir().expect("tempdir");
        let owner = assemble_owner(dir.path());

        // A fresh database has no crash-interrupted capability operation.
        assert!(owner.recover_capability_state().is_empty());

        owner
            .register_core_tools()
            .await
            .expect("register core tools");
        let tool_manager = owner.chat_state().tool_manager.clone();

        assert!(tool_manager.has_tool("read_file").await);
        assert!(tool_manager.has_tool("write_file").await);
        assert!(tool_manager.has_tool("grep").await);

        // The desktop-only Web MCP provider is registered separately after it
        // has started its loopback rmcp server. A headless runtime must not
        // advertise web tools before that provider is live.
        assert!(!tool_manager.has_tool("web_search").await);
        assert!(!tool_manager.has_tool("web_fetch").await);
    }

    /// The tool surface and the pre-workflow automation shells must share exactly
    /// one owned-process registry. A hold begun through the chat-state handle
    /// (the one the engine's shell tool registers every child on) must be awaited
    /// by the canonical automation cancel, which proves they are the same signal
    /// rather than two parallel registries.
    #[tokio::test]
    async fn runtime_owner_shares_one_owned_process_registry_with_the_tool_surface() {
        let dir = tempfile::tempdir().expect("tempdir");
        let owner = assemble_owner(dir.path());

        let tool_signal = owner
            .chat_state()
            .owned_process_cancellation()
            .expect("the owner installs the owned-process signal on the chat state");
        assert_eq!(tool_signal.in_flight(), 0);

        // A hold taken through the tool-side handle ...
        let ticket = tool_signal.begin();
        assert_eq!(tool_signal.in_flight(), 1);

        // ... must block the canonical automation cancel: the same registry.
        let cancelling = owner.service().cancel_owned_automation_shells();
        tokio::pin!(cancelling);
        assert!(
            tokio::time::timeout(Duration::from_millis(150), &mut cancelling)
                .await
                .is_err(),
            "the automation cancel must await a tool-side hold"
        );
        drop(ticket);
        let in_flight = tokio::time::timeout(Duration::from_secs(5), cancelling)
            .await
            .expect("the cancel resolves once the hold is released");
        assert_eq!(in_flight, 1);
        assert_eq!(tool_signal.in_flight(), 0);
    }

    #[tokio::test]
    async fn runtime_background_tasks_start_and_stop_with_their_owner() {
        let dir = tempfile::tempdir().expect("tempdir");
        let owner = assemble_owner(dir.path());
        owner
            .register_core_tools()
            .await
            .expect("register core tools");

        let background = RuntimeBackground::start(
            &owner,
            "test".to_string(),
            Arc::new(RuntimeLifecycleGate::new()),
        )
        .await;

        // The automation tick, the reconcile/MCP tasks and the ccproxy listener
        // must all stop promptly once the runtime asks them to, rather than
        // outliving the owner they were started for.
        tokio::time::timeout(Duration::from_secs(15), background.shutdown())
            .await
            .expect("background shuts down");
    }

    // -- runtime lifecycle: status and gate (AC-3/AC-7) ---------------------

    /// The status route reports the authoritative phase and lease count, and a
    /// closed gate refuses new work with `runtime_stopping` while the runtime
    /// keeps serving. This proves the gate also fences the canonical work start
    /// (not just the lease lifecycle) and that a quiescing runtime stays
    /// observable.
    #[tokio::test]
    async fn runtime_status_and_gate_refuse_work_while_quiescing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let handle = start_runtime(short_config(
            dir.path(),
            Duration::from_secs(30),
            Duration::from_secs(30),
        ))
        .await
        .expect("start runtime");
        let document = discovery::read_discovery_in(dir.path()).expect("discovery published");
        let http = Http {
            client: reqwest::Client::new(),
            port: handle.port(),
            token: document.token.clone(),
        };

        let response = http
            .post(
                "/control/v1/clients/register",
                r#"{"client_id":"client-a","client_kind":"tauri"}"#,
            )
            .await;
        assert_eq!(response.status(), reqwest::StatusCode::OK);

        // Running: the status route reports the live instance and the lease.
        let response = http.get("/control/v1/runtime/status").await;
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let status: Value = response.json().await.expect("status json");
        assert_eq!(status["phase"], "running");
        assert_eq!(status["instance_id"], handle.instance_id());
        assert_eq!(status["quiesced"], false);
        assert_eq!(status["active_leases"], 1);
        assert_eq!(status["automation"]["keep_alive"], false);

        // Close the gate without starting the coordinated exit, so the runtime
        // keeps serving and every refusal the gate produces is observable.
        assert!(handle.gate().begin_quiesce());

        let response = http.get("/control/v1/runtime/status").await;
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let status: Value = response.json().await.expect("status json");
        assert_eq!(status["phase"], "quiescing");
        assert_eq!(status["quiesced"], true);
        assert_eq!(status["epoch"], 1);

        // A new lease is refused with the stable quiescing code.
        let response = http
            .post(
                "/control/v1/clients/register",
                r#"{"client_id":"client-b","client_kind":"tauri"}"#,
            )
            .await;
        assert_eq!(response.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
        let envelope: Value = response.json().await.expect("error envelope");
        assert_eq!(envelope["error"]["code"], "runtime_stopping");

        // A canonical workflow start is refused on the same gate.
        let response = http
            .client
            .post(http.url("/control/v1/workflows/session-x/start"))
            .header("Authorization", format!("Bearer {}", http.token))
            .header("Content-Type", "application/json")
            .header("Idempotency-Key", "gate-1")
            .body("{}".to_string())
            .send()
            .await
            .expect("workflow start request");
        assert_eq!(response.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
        let envelope: Value = response.json().await.expect("error envelope");
        assert_eq!(envelope["error"]["code"], "runtime_stopping");

        handle.shutdown();
        tokio::time::timeout(Duration::from_secs(5), handle.wait())
            .await
            .expect("runtime stops");
        assert!(!discovery::discovery_path_in(dir.path()).exists());
        let _next = RuntimeDirLock::acquire(dir.path()).expect("owner lock was released");
    }

    // -- runtime lifecycle: coordinated shutdown ----------------------------

    /// The shutdown route enforces the bearer middleware, the lease proof, the
    /// expected instance id, an explicit confirmation and a mandatory
    /// Idempotency-Key; it answers `202 Accepted` (never a completion). The
    /// proof is validated before the idempotency cache, so every call — including
    /// a retry — is re-checked against the currently valid instance and lease.
    #[tokio::test]
    async fn runtime_shutdown_enforces_proof_instance_confirm_and_idempotency() {
        let dir = tempfile::tempdir().expect("tempdir");
        let handle = start_runtime(short_config(
            dir.path(),
            Duration::from_secs(30),
            Duration::from_secs(30),
        ))
        .await
        .expect("start runtime");
        let document = discovery::read_discovery_in(dir.path()).expect("discovery published");
        let http = Http {
            client: reqwest::Client::new(),
            port: handle.port(),
            token: document.token.clone(),
        };
        let instance = handle.instance_id().to_string();

        let response = http
            .post(
                "/control/v1/clients/register",
                r#"{"client_id":"client-a","client_kind":"tauri"}"#,
            )
            .await;
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let lease_id = response.json::<Value>().await.expect("lease json")["lease_id"]
            .as_str()
            .expect("lease id")
            .to_string();

        let wrong_instance =
            r#"{"expected_instance_id":"other-instance","confirm":true,"force":true}"#;
        let valid =
            format!(r#"{{"expected_instance_id":"{instance}","confirm":true,"force":true}}"#);

        // A missing Idempotency-Key is refused before anything else.
        let response = http
            .shutdown(&valid, Some("client-a"), Some(&lease_id), None)
            .await;
        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
        let envelope: Value = response.json().await.expect("error envelope");
        assert_eq!(envelope["error"]["code"], "missing_idempotency_key");

        // A missing lease proof is refused.
        let response = http.shutdown(&valid, None, None, Some("k-proof")).await;
        assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
        let envelope: Value = response.json().await.expect("error envelope");
        assert_eq!(envelope["error"]["code"], "lease_invalid");

        // A stale expected instance id is refused. The proof is validated before
        // the idempotency cache, so the refusal is re-derived on every call and
        // the same key/body yields the same refusal instead of a cached conflict.
        let response = http
            .shutdown(
                &wrong_instance,
                Some("client-a"),
                Some(&lease_id),
                Some("k-inst"),
            )
            .await;
        assert_eq!(response.status(), reqwest::StatusCode::CONFLICT);
        let stale_body = response.text().await.expect("stale body");
        assert!(stale_body.contains("instance_mismatch"), "{stale_body}");

        let response = http
            .shutdown(
                &wrong_instance,
                Some("client-a"),
                Some(&lease_id),
                Some("k-inst"),
            )
            .await;
        assert_eq!(response.status(), reqwest::StatusCode::CONFLICT);
        assert_eq!(response.text().await.expect("replayed body"), stale_body);

        // An unconfirmed request against the live instance is refused.
        let unconfirmed = format!(r#"{{"expected_instance_id":"{instance}","confirm":false}}"#);
        let response = http
            .shutdown(
                &unconfirmed,
                Some("client-a"),
                Some(&lease_id),
                Some("k-conf"),
            )
            .await;
        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
        let envelope: Value = response.json().await.expect("error envelope");
        assert_eq!(envelope["error"]["code"], "not_confirmed");

        // The confirmed request is accepted, not completed.
        let response = http
            .shutdown(&valid, Some("client-a"), Some(&lease_id), Some("k-run"))
            .await;
        assert_eq!(response.status(), reqwest::StatusCode::ACCEPTED);
        let accepted: Value = response.json().await.expect("accepted json");
        assert_eq!(accepted["accepted"], true);
        assert_eq!(accepted["force"], true);
        assert_eq!(accepted["instance_id"], instance);
        assert!(!accepted["operation_id"]
            .as_str()
            .expect("operation")
            .is_empty());

        // The coordinated exit drains and stops without waiting on its own
        // server; the lock is released for a successor.
        tokio::time::timeout(Duration::from_secs(10), handle.wait())
            .await
            .expect("runtime stops");
        assert!(!discovery::discovery_path_in(dir.path()).exists());
        let _next = RuntimeDirLock::acquire(dir.path()).expect("owner lock was released");
    }

    // -- runtime lifecycle: automation keep-alive ---------------------------

    /// Seeds one enabled, far-future `once` automation so the runtime reports an
    /// effective schedule and a keep-alive reason that is not a lease.
    fn seed_enabled_once_automation(db_path: &Path) {
        use chatspeed_runtime_backend::db::automation::WorkflowAutomationUpsert;
        use chatspeed_runtime_backend::db::Agent;
        let store = chatspeed_runtime_backend::db::MainStore::new(db_path).expect("seed store");
        // The automation's `agent_id` is a foreign key, so the owning agent row
        // must exist first.
        let agent = Agent::new(
            "agent-1".to_string(),
            "Seeder".to_string(),
            None,
            Some("primary".to_string()),
            None,
            String::new(),
            None,
            None,
            Some("[]".to_string()),
            Some("[]".to_string()),
            None,
            Some("[]".to_string()),
            Some("[]".to_string()),
            Some(false),
            Some("default".to_string()),
            Some(false),
            Some("[]".to_string()),
            Some("plan".to_string()),
            Some(false),
            Some(false),
            None,
        );
        store.add_agent(&agent).expect("seed agent");
        store
            .create_workflow_automation(&WorkflowAutomationUpsert {
                id: "automation-1".to_string(),
                title: "Keep alive".to_string(),
                prompt: Some("noop".to_string()),
                prompt_file_path: None,
                agent_id: "agent-1".to_string(),
                agent_config: None,
                allowed_paths: "[]".to_string(),
                shell_config: None,
                schedule_kind: "once".to_string(),
                schedule_config: r#"{"run_at":"2099-01-01 00:00:00"}"#.to_string(),
                continuous_context: false,
                current_workflow_session_id: None,
                self_review: false,
                enabled: true,
                next_run_at: Some("2099-01-01 00:00:00".to_string()),
            })
            .expect("seed automation");
    }

    /// An effective automation schedule keeps the runtime alive with no client
    /// lease, so the idle supervisor never mistakes a scheduled owner for an idle
    /// process. The status route surfaces the same reason.
    #[tokio::test]
    async fn automation_lifecycle_schedule_keeps_the_runtime_alive_without_leases() {
        let dir = tempfile::tempdir().expect("tempdir");
        seed_enabled_once_automation(&dir.path().join(DB_FILE_NAME));

        let handle = start_runtime(short_config(
            dir.path(),
            Duration::from_millis(300),
            Duration::from_secs(30),
        ))
        .await
        .expect("start runtime");

        // No lease is ever registered; the effective schedule must hold the
        // runtime open well past the grace window.
        tokio::time::sleep(Duration::from_millis(900)).await;
        assert!(
            !handle.is_finished(),
            "an effective automation schedule keeps the runtime alive without a lease"
        );
        assert!(discovery::discovery_path_in(dir.path()).exists());

        let document = discovery::read_discovery_in(dir.path()).expect("discovery published");
        let http = Http {
            client: reqwest::Client::new(),
            port: handle.port(),
            token: document.token.clone(),
        };
        let status: Value = http
            .get("/control/v1/runtime/status")
            .await
            .json()
            .await
            .expect("status json");
        assert_eq!(status["automation"]["effective_schedules"], 1);
        assert_eq!(status["automation"]["keep_alive"], true);
        assert_eq!(status["active_leases"], 0);

        handle.shutdown();
        tokio::time::timeout(Duration::from_secs(10), handle.wait())
            .await
            .expect("runtime stops on request");
    }

    // -- runtime lifecycle: U-5 budgets, forced fallback and termination hook --

    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Records the last-resort termination fallback without exiting the test
    /// process.
    #[derive(Debug)]
    struct RecordingHook {
        fired: Arc<AtomicUsize>,
    }

    impl TerminationHook for RecordingHook {
        fn force_exit(&self, _instance_id: &str, _pid: u32) {
            self.fired.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// The graceful and forced budgets, plus the termination hook, are injectable
    /// so a test (or the desktop supervisor) can drive a short shutdown without a
    /// production configuration knob.
    #[test]
    fn shutdown_budgets_and_the_termination_hook_are_injectable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = RuntimeConfig::with_runtime_dir(dir.path())
            .with_shutdown_budget(Duration::from_millis(1234))
            .with_force_budget(Duration::from_millis(567));
        assert_eq!(config.shutdown_budget(), Duration::from_millis(1234));
        assert_eq!(config.force_budget(), Duration::from_millis(567));
        // The default hook exists and is the safe library hook.
        let _ = config.termination_hook();

        let defaults = RuntimeConfig::with_runtime_dir(dir.path());
        assert_eq!(defaults.shutdown_budget(), DEFAULT_SHUTDOWN_BUDGET);
        assert_eq!(defaults.force_budget(), DEFAULT_FORCE_BUDGET);
    }

    /// An idle-grace shutdown is never forced, so it never spends the forced
    /// budget nor invokes the termination fallback.
    #[tokio::test]
    async fn an_idle_shutdown_never_forces_and_never_fires_the_termination_hook() {
        let dir = tempfile::tempdir().expect("tempdir");
        let fired = Arc::new(AtomicUsize::new(0));
        let hook = Arc::new(RecordingHook {
            fired: fired.clone(),
        });
        let handle = start_runtime(
            short_config(
                dir.path(),
                Duration::from_millis(300),
                Duration::from_secs(5),
            )
            .with_force_budget(Duration::from_millis(200))
            .with_termination_hook(hook),
        )
        .await
        .expect("start runtime");

        tokio::time::timeout(Duration::from_secs(5), handle.wait())
            .await
            .expect("the idle grace stops the runtime");
        assert_eq!(
            fired.load(Ordering::SeqCst),
            0,
            "an idle shutdown is never forced"
        );
    }

    /// A clean shutdown must still record `Graceful`: the outcome-preservation
    /// path added for an unproven owned teardown must never turn a healthy stop
    /// into a failure.
    #[tokio::test]
    async fn a_clean_shutdown_records_a_graceful_outcome() {
        let dir = tempfile::tempdir().expect("tempdir");
        let handle = start_runtime(short_config(
            dir.path(),
            Duration::from_millis(50),
            Duration::from_secs(5),
        ))
        .await
        .expect("start runtime");

        let (operation, handoff) = handle.shutdown.request(false);
        drop(handoff);
        assert!(!operation.force, "the graceful request is not forced");

        tokio::time::timeout(Duration::from_secs(5), handle.wait())
            .await
            .expect("the idle grace stops the runtime");

        let outcome = handle.shutdown.operation_dto().and_then(|dto| dto.outcome);
        assert_eq!(
            outcome,
            Some(RuntimeShutdownOutcome::Graceful),
            "a clean shutdown with no unproven owned work records a graceful outcome"
        );
    }

    /// A forced shutdown that cannot drain (a held admission) must never falsely
    /// report `stopped`; it spends the forced budget and then fires the
    /// last-resort termination fallback, which the test records instead of exiting.
    #[tokio::test]
    async fn a_forced_shutdown_that_cannot_drain_fires_the_termination_hook_without_claiming_stopped(
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let fired = Arc::new(AtomicUsize::new(0));
        let hook = Arc::new(RecordingHook {
            fired: fired.clone(),
        });
        let handle = start_runtime(
            short_config(dir.path(), Duration::from_secs(30), Duration::from_secs(30))
                .with_shutdown_budget(Duration::from_millis(300))
                .with_force_budget(Duration::from_millis(200))
                .with_termination_hook(hook),
        )
        .await
        .expect("start runtime");

        // Hold one admission so the drain can never complete.
        let ticket = handle
            .gate()
            .admit(WorkKind::ChatTurn)
            .expect("admit while running");

        // An explicit forced request (the private authority the route and the
        // supervisor funnel into); dropping the hand-off starts the worker at once.
        let (_operation, handoff) = handle.shutdown.request(true);
        drop(handoff);

        let mut waited = Duration::ZERO;
        while fired.load(Ordering::SeqCst) == 0 && waited < Duration::from_secs(5) {
            tokio::time::sleep(Duration::from_millis(25)).await;
            waited += Duration::from_millis(25);
        }
        assert!(
            fired.load(Ordering::SeqCst) >= 1,
            "the forced fallback fires once the graceful and forced budgets elapse"
        );
        assert!(
            !handle.is_finished(),
            "a held admission forbids the `stopped` claim"
        );

        drop(ticket);
    }
}
