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

use chatspeed_contracts::{ClientLease, ClientLeaseRequest, ClientLeaseResponse};
use chatspeed_runtime_backend::background::RuntimeBackground;
use chatspeed_runtime_backend::owner::{RuntimeOwner, RuntimeOwnerConfig};
use chatspeed_runtime_backend::workflow::react::client::http::server::{
    self, RuntimeControlPlane, RuntimeControlPlaneOptions, RuntimeLeaseError, RuntimeWebMcpPlane,
};
use chatspeed_runtime_backend::web_provider::{
    ToolManagerProviderInstaller, WebProviderRegistry,
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
}

impl From<LeaseError> for RuntimeLeaseError {
    fn from(error: LeaseError) -> Self {
        match error {
            LeaseError::InvalidInput(message) => RuntimeLeaseError::InvalidInput(message),
            LeaseError::NotFound(client_id) => RuntimeLeaseError::NotFound(client_id),
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
}

impl LeaseRegistry {
    /// Creates an empty registry applying `ttl` to every lease.
    pub fn new(ttl: Duration) -> Self {
        let (count_tx, _count_rx) = watch::channel(0);
        Self {
            ttl,
            inner: std::sync::Mutex::new(HashMap::new()),
            count_tx,
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
        let now = Instant::now();
        let (removed, count) = {
            let mut leases = self.lock();
            leases.retain(|_, entry| entry.expires_at > now);
            let removed = leases.remove(client_id).is_some();
            (removed, leases.len())
        };
        self.publish_count(count);
        if removed {
            log::debug!("[Runtime] released lease for client");
            Ok(())
        } else {
            Err(LeaseError::NotFound(client_id.to_string()))
        }
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

/// Exposes the runtime lease lifecycle to the canonical control plane, so the
/// lease routes and `/meta` service identity live on the same server as the
/// workflow/capability/automation routes (AC-6/AC-7).
impl RuntimeControlPlane for LeaseRegistry {
    fn service_name(&self) -> &str {
        SERVICE_NAME
    }

    fn register_lease(
        &self,
        request: &ClientLeaseRequest,
    ) -> Result<ClientLeaseResponse, RuntimeLeaseError> {
        self.register(request).map_err(RuntimeLeaseError::from)
    }

    fn renew_lease(&self, client_id: &str) -> Result<ClientLeaseResponse, RuntimeLeaseError> {
        self.renew(client_id).map_err(RuntimeLeaseError::from)
    }

    fn validate_lease(
        &self,
        client_id: &str,
        lease_id: &str,
    ) -> Result<ClientLease, RuntimeLeaseError> {
        self.validate(client_id, lease_id)
            .map_err(RuntimeLeaseError::from)
    }

    fn release_lease(&self, client_id: &str) -> Result<(), RuntimeLeaseError> {
        self.release(client_id).map_err(RuntimeLeaseError::from)
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
    shutdown_tx: watch::Sender<bool>,
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

    /// Whether the instance has finished shutting down.
    pub fn is_finished(&self) -> bool {
        *self.finished_rx.borrow()
    }

    /// Requests graceful shutdown.
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send_replace(true);
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
        std::env::var_os("CHATSPEED_RUNTIME_RESOURCE_DIR").map(PathBuf::from),
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
    let owner = RuntimeOwner::assemble(RuntimeOwnerConfig {
        instance_id: instance_id.clone(),
        db_path: config.db_path().to_path_buf(),
        app_data_dir: config.app_data_dir().to_path_buf(),
    })
    .map_err(|error| RuntimeError::Assembly(error.to_string()))?;

    // Fail-closed ordering: classify crash-interrupted capability operations and
    // load the desktop-free core tool surface *before* discovery is published or
    // a lease is accepted, so a client can never observe a half-initialized
    // owner. Recovery failures are logged, not fatal, because the durable
    // journal keeps the operation repairable on the next start.
    owner.recover_capability_state();
    if let Err(error) = owner.register_core_tools().await {
        log::error!("[Runtime] failed to register core tools: {error}");
    }

    let leases = Arc::new(LeaseRegistry::new(config.lease_ttl()));

    // The runtime owns its own background work: capability reconcile, the
    // configured MCP servers, the automation tick and the chat-completion proxy.
    // It starts before the control plane so a connecting client can rely on the
    // same paths the desktop wires in `tauri::setup`.
    let background = RuntimeBackground::start(&owner, env!("CARGO_PKG_VERSION").to_string()).await;

    let control = match server::start_runtime_control_plane(
        owner.service().clone(),
        RuntimeControlPlaneOptions {
            discovery_dir: config.runtime_dir().to_path_buf(),
            leases: leases.clone(),
            chat: Some(owner.chat_plane()),
            terminal: Some(owner.terminal_plane()),
            // The desktop registers its loopback Web MCP provider onto this
            // single lease-bound slot; the runtime reaches it as an ordinary MCP
            // server, so the fixed web tools stay on the canonical tool path.
            web_provider: Some(
                Arc::new(WebProviderRegistry::new(Arc::new(
                    ToolManagerProviderInstaller::new(owner.chat_state().clone()),
                ))) as Arc<dyn RuntimeWebMcpPlane>,
            ),
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

    let port = control.port;
    let runtime_dir = config.runtime_dir().to_path_buf();
    let db_path = config.db_path().to_path_buf();

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (finished_tx, finished_rx) = watch::channel(false);

    // The serve task owns the owner, the background tasks, the lock and the
    // discovery cleanup: the lock is only released after the canonical HTTP
    // server has fully stopped and no runtime-owned task is left running.
    let mut serve_shutdown = shutdown_rx.clone();
    let serve_instance = instance_id.clone();
    tokio::spawn(async move {
        let _ = serve_shutdown.changed().await;
        control.shutdown();
        control.wait().await;
        // Stop the automation tick, the reconcile/MCP tasks and the ccproxy
        // listener only after the control plane has drained, so an in-flight
        // request is never cut off by background shutdown.
        background.shutdown().await;
        drop(owner);
        // Release the database authority only after the owner (and the control
        // plane that serves it) has fully drained, so a successor never opens
        // the database while this instance is still using it.
        drop(db_lock);
        drop(lock);
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

    // Idle supervisor: grace-based shutdown after the last lease is gone.
    tokio::spawn(supervise_idle(
        leases.subscribe(),
        shutdown_rx.clone(),
        config.grace(),
        shutdown_tx.clone(),
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
        shutdown_tx,
        finished_rx,
    })
}

/// Shuts the instance down once the lease count has been zero for `grace`.
///
/// A registration during the grace window wins: the count change aborts the
/// pending shutdown and the loop starts over.
async fn supervise_idle(
    mut count_rx: watch::Receiver<usize>,
    mut shutdown_rx: watch::Receiver<bool>,
    grace: Duration,
    shutdown_tx: watch::Sender<bool>,
) {
    loop {
        if *shutdown_rx.borrow() {
            return;
        }
        if *count_rx.borrow() > 0 {
            tokio::select! {
                changed = count_rx.changed() => {
                    if changed.is_err() {
                        return;
                    }
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        return;
                    }
                }
            }
            continue;
        }

        let sleep = tokio::time::sleep(grace);
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                _ = &mut sleep => {
                    if *count_rx.borrow() == 0 {
                        log::info!(
                            "[Runtime] idle grace of {:?} elapsed with no active leases; shutting down",
                            grace
                        );
                        let _ = shutdown_tx.send_replace(true);
                        return;
                    }
                    break;
                }
                changed = count_rx.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    if *count_rx.borrow() > 0 {
                        log::debug!("[Runtime] active lease cancelled pending idle shutdown");
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use chatspeed_runtime_backend::workflow::react::client::http::discovery;
    use serde_json::Value;

    /// Serializes tests that mutate the process environment.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

    #[tokio::test]
    async fn runtime_background_tasks_start_and_stop_with_their_owner() {
        let dir = tempfile::tempdir().expect("tempdir");
        let owner = assemble_owner(dir.path());
        owner
            .register_core_tools()
            .await
            .expect("register core tools");

        let background = RuntimeBackground::start(&owner, "test".to_string()).await;

        // The automation tick, the reconcile/MCP tasks and the ccproxy listener
        // must all stop promptly once the runtime asks them to, rather than
        // outliving the owner they were started for.
        tokio::time::timeout(Duration::from_secs(15), background.shutdown())
            .await
            .expect("background shuts down");
    }
}
