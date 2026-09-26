//! Experiment data-domain layout, marker and singleton lease (Phase 2H).
//!
//! A `chatspeed-headless` instance is only ever allowed to own an explicit,
//! physically separate data directory. This module is the fail-closed gate in
//! front of everything else (AC-1, INV-1, INV-9):
//!
//! 1. The data directory must be usable and must not be a symlink, and every
//!    fixed subdirectory is created with owner-only permissions.
//! 2. The database is adopted only when it does not yet exist, or when it
//!    already carries the `experiment.v1` marker row. An existing database
//!    without that marker — above all a desktop database — is refused before
//!    any migration or runtime startup. There is no `:memory:` fallback.
//! 3. A singleton lease row makes a second concurrent instance fail closed
//!    instead of racing the first, and it is fenced by generation so a stale
//!    instance can never release or renew a newer instance's lease.

use crate::db::experiment_schedule::persistence_error;
use crate::db::MainStore;
use crate::workflow::react::campaign::domain_hash;
use crate::workflow::react::experiment_schedule::types::{
    ExperimentDomainMarkerV1, ScheduleError, ScheduleErrorCode, DOMAIN_KIND_EXPERIMENT_V1,
};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Hash domain for the backend-minted domain id.
pub const DOMAIN_ID_DOMAIN: &str = "cs-experiment-domain:id";

/// The fixed directory layout of an experiment domain. Everything a run writes
/// lives under these directories and nowhere else.
///
/// `promotion-targets` was added by Phase 2I. The layout is created with
/// `create_dir_all`, so an experiment domain provisioned by an earlier binary
/// simply gains the empty directory on its next open; nothing else changes.
pub const DOMAIN_DIRECTORIES: &[&str] = &[
    "runtime",
    "worktrees",
    "bundles",
    "artifacts",
    "journals",
    "promotion-targets",
];

/// Default domain-lease length. The headless heartbeat renews it well inside
/// this window; an expired lease is a recoverable crash, not a live owner.
pub const DEFAULT_DOMAIN_LEASE_MS: u64 = 30_000;

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// RFC3339 timestamp used for human-facing markers.
fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn domain_error(code: ScheduleErrorCode, message: impl Into<String>) -> ScheduleError {
    ScheduleError::new(code, message)
}

/// The fixed paths of one experiment domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExperimentDomainPaths {
    pub root: PathBuf,
    pub database: PathBuf,
    pub runtime: PathBuf,
    pub worktrees: PathBuf,
    pub bundles: PathBuf,
    pub artifacts: PathBuf,
    pub journals: PathBuf,
}

impl ExperimentDomainPaths {
    /// Resolves the layout under a data directory without touching the disk.
    pub fn resolve(data_dir: impl AsRef<Path>) -> Self {
        let root = data_dir.as_ref().to_path_buf();
        Self {
            database: root.join("chatspeed.db"),
            runtime: root.join("runtime"),
            worktrees: root.join("worktrees"),
            bundles: root.join("bundles"),
            artifacts: root.join("artifacts"),
            journals: root.join("journals"),
            root,
        }
    }

    /// The runtime discovery file of this domain. It is deliberately *not*
    /// `${CHATSPEED_HOME}/runtime/...`: a headless domain publishes its own
    /// discovery document so a desktop instance is never confused with it.
    pub fn discovery_file(&self) -> PathBuf {
        self.runtime.join("control-plane-v1.json")
    }
}

/// An opened, marked experiment domain holding the singleton lease.
///
/// Deliberately not `Debug`: it owns the live `MainStore`, and there is no
/// useful debug representation of a database handle.
pub struct ExperimentDomain {
    paths: ExperimentDomainPaths,
    domain_id: String,
    store: Arc<MainStore>,
    lease: ExperimentDomainLease,
}

impl ExperimentDomain {
    /// Opens or initializes the domain at `data_dir`, runs migrations, ensures
    /// the marker, and takes the singleton lease.
    ///
    /// Every failure is a pre-runtime, fail-closed error: no window, no
    /// runtime, no scheduler and no provider is started before this succeeds.
    pub fn open(data_dir: impl AsRef<Path>, owner_id: &str) -> Result<Self, ScheduleError> {
        let data_dir = data_dir.as_ref();
        if data_dir.as_os_str().is_empty() {
            return Err(domain_error(
                ScheduleErrorCode::DomainLayoutUnsafe,
                "headless requires an explicit non-empty --data-dir",
            ));
        }
        let paths = ExperimentDomainPaths::resolve(data_dir);
        prepare_directories(&paths)?;

        let domain_id = domain_id_for(&paths.root)?;
        ensure_marker_admissible(&paths.database)?;

        let store = Arc::new(MainStore::new(&paths.database).map_err(|error| {
            domain_error(
                ScheduleErrorCode::PersistenceFailure,
                format!("failed to open the experiment database: {error}"),
            )
        })?);
        write_marker(&store, &domain_id)?;
        verify_marker(&store, &domain_id)?;

        let lease = ExperimentDomainLease::acquire(&store, owner_id, now_ms())?;
        Ok(Self {
            paths,
            domain_id,
            store,
            lease,
        })
    }

    pub fn paths(&self) -> &ExperimentDomainPaths {
        &self.paths
    }

    pub fn domain_id(&self) -> &str {
        &self.domain_id
    }

    /// The domain's single main store. Crate-internal: callers get what they
    /// need through the runtime/domain APIs rather than reaching into the
    /// domain's persistence handle.
    pub(crate) fn store(&self) -> &Arc<MainStore> {
        &self.store
    }

    pub fn lease(&self) -> &ExperimentDomainLease {
        &self.lease
    }

    /// Renews the domain lease. A stale generation fails closed, which is what
    /// tells a superseded instance to stop touching the domain.
    pub fn renew_lease(&self, lease_ms: u64) -> Result<(), ScheduleError> {
        self.lease.renew(&self.store, now_ms(), lease_ms)
    }

    /// The shared main store. Ownership must stay with exactly one runtime per
    /// domain (INV-1).
    pub fn into_parts(self) -> (Arc<MainStore>, ExperimentDomainPaths) {
        (self.store, self.paths)
    }
}

impl std::fmt::Debug for ExperimentDomain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The live database handle has no useful representation, so only the
        // domain identity and layout are rendered.
        f.debug_struct("ExperimentDomain")
            .field("paths", &self.paths)
            .field("domain_id", &self.domain_id)
            .field("lease", &self.lease)
            .finish_non_exhaustive()
    }
}

/// Derives the backend-minted domain id from the canonical data directory.
fn domain_id_for(root: &Path) -> Result<String, ScheduleError> {
    let canonical = std::fs::canonicalize(root).map_err(|error| {
        domain_error(
            ScheduleErrorCode::DomainLayoutUnsafe,
            format!("data directory '{}' is not usable: {error}", root.display()),
        )
    })?;
    let canonical = canonical.to_string_lossy().to_string();
    Ok(format!(
        "domain-{}",
        &domain_hash(DOMAIN_ID_DOMAIN, canonical.as_bytes())[..32]
    ))
}

/// Creates the fixed layout with owner-only permissions and rejects anything
/// that is not a plain directory.
fn prepare_directories(paths: &ExperimentDomainPaths) -> Result<(), ScheduleError> {
    if let Ok(metadata) = std::fs::symlink_metadata(&paths.root) {
        if metadata.file_type().is_symlink() {
            return Err(domain_error(
                ScheduleErrorCode::DomainLayoutUnsafe,
                format!(
                    "data directory '{}' is a symlink; refusing to own it",
                    paths.root.display()
                ),
            ));
        }
    }
    std::fs::create_dir_all(&paths.root).map_err(|error| {
        domain_error(
            ScheduleErrorCode::DomainLayoutUnsafe,
            format!(
                "failed to create data directory '{}': {error}",
                paths.root.display()
            ),
        )
    })?;
    restrict_to_owner(&paths.root)?;
    for name in DOMAIN_DIRECTORIES {
        let directory = paths.root.join(name);
        std::fs::create_dir_all(&directory).map_err(|error| {
            domain_error(
                ScheduleErrorCode::DomainLayoutUnsafe,
                format!("failed to create '{}': {error}", directory.display()),
            )
        })?;
        let metadata = std::fs::symlink_metadata(&directory).map_err(|error| {
            domain_error(
                ScheduleErrorCode::DomainLayoutUnsafe,
                format!("failed to inspect '{}': {error}", directory.display()),
            )
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(domain_error(
                ScheduleErrorCode::DomainLayoutUnsafe,
                format!(
                    "'{}' must be a real directory (not a symlink or file)",
                    directory.display()
                ),
            ));
        }
        restrict_to_owner(&directory)?;
    }
    Ok(())
}

#[cfg(unix)]
fn restrict_to_owner(path: &Path) -> Result<(), ScheduleError> {
    use std::os::unix::fs::PermissionsExt;
    let permissions = std::fs::Permissions::from_mode(0o700);
    std::fs::set_permissions(path, permissions).map_err(|error| {
        domain_error(
            ScheduleErrorCode::DomainLayoutUnsafe,
            format!("failed to restrict '{}': {error}", path.display()),
        )
    })
}

#[cfg(not(unix))]
fn restrict_to_owner(_path: &Path) -> Result<(), ScheduleError> {
    // Windows has no POSIX mode bits; the ACL is inherited from the data
    // directory the operator created.
    Ok(())
}

/// Refuses an existing database that does not already carry the experiment
/// marker. This is the check that keeps a desktop database safe (INV-9).
fn ensure_marker_admissible(database: &Path) -> Result<(), ScheduleError> {
    let metadata = match std::fs::metadata(database) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(domain_error(
                ScheduleErrorCode::DomainLayoutUnsafe,
                format!("failed to inspect '{}': {error}", database.display()),
            ))
        }
    };
    if metadata.is_dir() {
        return Err(domain_error(
            ScheduleErrorCode::DomainLayoutUnsafe,
            format!("'{}' is a directory, not a database", database.display()),
        ));
    }
    // A zero-length file is a brand-new database that SQLite has not written
    // its header to yet; it carries no data to protect.
    if metadata.len() == 0 {
        return Ok(());
    }

    let connection = Connection::open_with_flags(
        database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| {
        domain_error(
            ScheduleErrorCode::DomainLayoutUnsafe,
            format!("failed to open '{}' read-only: {error}", database.display()),
        )
    })?;
    let table_present: bool = connection
        .query_row(
            "SELECT COUNT(1) FROM sqlite_master WHERE type = 'table' AND name = 'experiment_domain'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count > 0)
        .map_err(|error| {
            domain_error(
                ScheduleErrorCode::DomainLayoutUnsafe,
                format!(
                    "'{}' is not a readable ChatSpeed database: {error}",
                    database.display()
                ),
            )
        })?;
    if !table_present {
        return Err(domain_error(
            ScheduleErrorCode::DomainUnmarked,
            format!(
                "'{}' already exists but has no experiment-domain marker; \
                 refusing to take over an existing database",
                database.display()
            ),
        ));
    }
    let kind: Option<String> = connection
        .query_row(
            "SELECT domain_kind FROM experiment_domain LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| {
            domain_error(
                ScheduleErrorCode::DomainUnmarked,
                format!(
                    "failed to read the domain marker of '{}': {error}",
                    database.display()
                ),
            )
        })?;
    match kind.as_deref() {
        Some(DOMAIN_KIND_EXPERIMENT_V1) => Ok(()),
        Some(other) => Err(domain_error(
            ScheduleErrorCode::DomainUnmarked,
            format!(
                "'{}' is marked as '{other}', not an experiment domain",
                database.display()
            ),
        )),
        None => Err(domain_error(
            ScheduleErrorCode::DomainUnmarked,
            format!(
                "'{}' exists without a domain marker row; refusing to take over",
                database.display()
            ),
        )),
    }
}

/// Writes the marker row for a freshly initialized domain. An existing marker
/// is left untouched so a restart never rewrites domain identity.
fn write_marker(store: &Arc<MainStore>, domain_id: &str) -> Result<(), ScheduleError> {
    let runtime = store.db_runtime().map_err(persistence_error)?;
    let domain_id = domain_id.to_string();
    runtime
        .write_blocking(move |conn| {
            let existing: Option<String> = conn
                .query_row(
                    "SELECT domain_id FROM experiment_domain LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .optional()
                .map_err(crate::db::StoreError::from)?;
            if existing.is_some() {
                return Ok(());
            }
            let marker = ExperimentDomainMarkerV1::new(&domain_id, now_rfc3339());
            conn.execute(
                "INSERT INTO experiment_domain (
                    domain_id, domain_kind, marker_schema_version, singleton, created_at_ms
                 ) VALUES (?1, ?2, ?3, 1, ?4)",
                params![
                    marker.domain_id,
                    marker.domain_kind,
                    marker.schema_version,
                    now_ms() as i64,
                ],
            )?;
            Ok(())
        })
        .map_err(persistence_error)
}

/// Confirms the stored marker really identifies this domain.
fn verify_marker(store: &Arc<MainStore>, domain_id: &str) -> Result<(), ScheduleError> {
    let runtime = store.db_runtime().map_err(persistence_error)?;
    let domain_id = domain_id.to_string();
    runtime
        .read_blocking(move |conn| {
            let row: Option<(String, String, String)> = conn
                .query_row(
                    "SELECT domain_id, domain_kind, marker_schema_version
                       FROM experiment_domain LIMIT 1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            let (stored_id, kind, schema_version) = row.ok_or_else(|| {
                crate::db::StoreError::InvalidData(
                    "experiment domain has no marker row".to_string(),
                )
            })?;
            if kind != DOMAIN_KIND_EXPERIMENT_V1 {
                return Err(crate::db::StoreError::InvalidData(format!(
                    "experiment domain marker kind is '{kind}'"
                )));
            }
            let marker = ExperimentDomainMarkerV1 {
                schema_version,
                domain_kind: kind,
                domain_id: stored_id,
                created_at: String::new(),
            };
            if !marker.is_experiment_v1() {
                return Err(crate::db::StoreError::InvalidData(
                    "experiment domain marker is not a versioned experiment marker".to_string(),
                ));
            }
            if marker.domain_id != domain_id {
                return Err(crate::db::StoreError::InvalidData(format!(
                    "experiment domain id mismatch: '{}' is marked '{}'",
                    domain_id, marker.domain_id
                )));
            }
            Ok(())
        })
        .map_err(persistence_error)
}

/// The singleton, generation-fenced domain lease.
///
/// Cloneable so a heartbeat task can renew the exact `(owner_id, generation)`
/// pair the process was granted; it can never renew a generation it does not
/// own.
#[derive(Debug, Clone)]
pub struct ExperimentDomainLease {
    owner_id: String,
    generation: i64,
}

impl ExperimentDomainLease {
    /// Takes or renews the singleton lease. A live foreign lease is a
    /// fail-closed `experiment_domain_locked`; an expired one is a crash the
    /// new instance may take over with a bumped generation.
    pub fn acquire(
        store: &Arc<MainStore>,
        owner_id: &str,
        now_ms: u64,
    ) -> Result<Self, ScheduleError> {
        if owner_id.trim().is_empty() {
            return Err(domain_error(
                ScheduleErrorCode::DomainLayoutUnsafe,
                "domain owner id must not be empty",
            ));
        }
        let runtime = store.db_runtime().map_err(persistence_error)?;
        let owner = owner_id.to_string();
        let expires = now_ms.saturating_add(DEFAULT_DOMAIN_LEASE_MS) as i64;
        runtime
            .write_blocking(move |conn| {
                let tx = conn.transaction()?;
                let current: Option<(String, i64, i64)> = tx
                    .query_row(
                        "SELECT owner_id, lease_generation, lease_expires_at_ms
                           FROM experiment_domain_lease
                          WHERE singleton = 1",
                        [],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .optional()?;
                let generation = match current {
                    Some((current_owner, generation, lease_expires_at_ms)) => {
                        if lease_expires_at_ms > now_ms as i64 {
                            return Err(crate::db::StoreError::InvalidData(format!(
                                "experiment domain is locked by live owner '{current_owner}' \
                                 (generation {generation})"
                            )));
                        }
                        generation + 1
                    }
                    None => 1,
                };
                tx.execute(
                    "INSERT OR REPLACE INTO experiment_domain_lease (
                        singleton, owner_id, lease_generation, pid,
                        lease_expires_at_ms, heartbeat_at_ms, started_at_ms
                     ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?5)",
                    params![
                        owner,
                        generation,
                        std::process::id() as i64,
                        expires,
                        now_ms as i64,
                    ],
                )?;
                tx.commit()?;
                Ok(generation)
            })
            .map(|generation| Self {
                owner_id: owner_id.to_string(),
                generation,
            })
            .map_err(|error| {
                let rendered = error.to_string();
                if rendered.contains("experiment domain is locked") {
                    domain_error(ScheduleErrorCode::DomainLocked, rendered)
                } else {
                    persistence_error(error)
                }
            })
    }

    pub fn owner_id(&self) -> &str {
        &self.owner_id
    }

    pub fn generation(&self) -> i64 {
        self.generation
    }

    /// Renews the lease. Losing the generation means another instance took the
    /// domain over, and this instance must stop.
    pub fn renew(
        &self,
        store: &Arc<MainStore>,
        now_ms: u64,
        lease_ms: u64,
    ) -> Result<(), ScheduleError> {
        let runtime = store.db_runtime().map_err(persistence_error)?;
        let owner = self.owner_id.clone();
        let generation = self.generation;
        let expires = now_ms.saturating_add(lease_ms) as i64;
        runtime
            .write_blocking(move |conn| {
                let changed = conn.execute(
                    "UPDATE experiment_domain_lease
                        SET lease_expires_at_ms = ?3, heartbeat_at_ms = ?4
                      WHERE singleton = 1 AND owner_id = ?1 AND lease_generation = ?2",
                    params![owner, generation, expires, now_ms as i64],
                )?;
                if changed != 1 {
                    return Err(crate::db::StoreError::InvalidData(format!(
                        "experiment domain lease generation {generation} is no longer current"
                    )));
                }
                Ok(())
            })
            .map_err(|error| {
                let rendered = error.to_string();
                if rendered.contains("no longer current") {
                    domain_error(ScheduleErrorCode::DomainLocked, rendered)
                } else {
                    persistence_error(error)
                }
            })
    }

    /// Releases the lease on a graceful shutdown. The release is fenced, so a
    /// superseded instance can never free a newer instance's lease.
    ///
    /// Releasing an already-absent lease is idempotent; releasing a lease that
    /// now belongs to another owner or generation fails closed.
    pub fn release(&self, store: &Arc<MainStore>) -> Result<(), ScheduleError> {
        let runtime = store.db_runtime().map_err(persistence_error)?;
        let owner = self.owner_id.clone();
        let generation = self.generation;
        runtime
            .write_blocking(move |conn| {
                let deleted = conn.execute(
                    "DELETE FROM experiment_domain_lease
                      WHERE singleton = 1 AND owner_id = ?1 AND lease_generation = ?2",
                    params![owner, generation],
                )?;
                if deleted == 1 {
                    return Ok(());
                }
                let present: Option<i64> = conn
                    .query_row(
                        "SELECT COUNT(1) FROM experiment_domain_lease WHERE singleton = 1",
                        [],
                        |row| row.get(0),
                    )
                    .optional()?;
                let present = present.unwrap_or(0);
                if present > 0 {
                    return Err(crate::db::StoreError::InvalidData(format!(
                        "experiment domain lease generation {generation} is no longer current"
                    )));
                }
                Ok(())
            })
            .map_err(|error| {
                let rendered = error.to_string();
                if rendered.contains("no longer current") {
                    domain_error(ScheduleErrorCode::DomainLocked, rendered)
                } else {
                    persistence_error(error)
                }
            })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn open(data_dir: &Path, owner: &str) -> Result<ExperimentDomain, ScheduleError> {
        ExperimentDomain::open(data_dir, owner)
    }

    #[test]
    fn fresh_domain_is_created_marked_and_layout_is_fixed() {
        let directory = tempdir().expect("tempdir");
        let domain = open(directory.path(), "owner-a").expect("opens");
        assert_eq!(domain.lease().owner_id(), "owner-a");
        assert_eq!(domain.lease().generation(), 1);
        assert!(domain.domain_id().starts_with("domain-"));
        assert!(domain.paths().database.exists());
        for name in DOMAIN_DIRECTORIES {
            assert!(
                directory.path().join(name).is_dir(),
                "missing {name} directory"
            );
        }
        assert_eq!(
            domain.paths().discovery_file(),
            directory
                .path()
                .join("runtime")
                .join("control-plane-v1.json")
        );
    }

    #[test]
    fn reopening_the_same_domain_keeps_identity() {
        let directory = tempdir().expect("tempdir");
        let first = open(directory.path(), "owner-a").expect("opens");
        let domain_id = first.domain_id().to_string();
        first
            .lease()
            .release(first.store())
            .expect("release on shutdown");
        drop(first);

        let second = open(directory.path(), "owner-b").expect("reopens");
        assert_eq!(second.domain_id(), domain_id);
        assert_eq!(second.lease().owner_id(), "owner-b");
        // A graceful shutdown frees the lease, so a fresh takeover starts at
        // generation one again; only a crashed owner bumps it (asserted by
        // `an_expired_lease_is_recoverable_and_fenced`).
        assert_eq!(second.lease().generation(), 1);
    }

    #[test]
    fn a_live_lease_blocks_a_second_instance() {
        let directory = tempdir().expect("tempdir");
        let first = open(directory.path(), "owner-a").expect("opens");
        let error = open(directory.path(), "owner-b").expect_err("must be locked");
        assert_eq!(error.code, ScheduleErrorCode::DomainLocked);
        // Releasing the first instance unblocks the second.
        first.lease().release(first.store()).expect("release");
        drop(first);
        open(directory.path(), "owner-b").expect("now free");
    }

    #[test]
    fn an_expired_lease_is_recoverable_and_fenced() {
        let directory = tempdir().expect("tempdir");
        let first = open(directory.path(), "owner-a").expect("opens");
        // Simulate a crashed owner by expiring the lease row directly.
        {
            let runtime = first.store().db_runtime().expect("runtime");
            runtime
                .write_blocking(|conn| {
                    conn.execute(
                        "UPDATE experiment_domain_lease SET lease_expires_at_ms = 0 WHERE singleton = 1",
                        [],
                    )?;
                    Ok(())
                })
                .expect("expire lease");
        }
        let second = open(directory.path(), "owner-b").expect("adopts expired lease");
        assert_eq!(second.lease().generation(), 2);
        // The superseded instance can neither renew nor release.
        let error = first
            .renew_lease(DEFAULT_DOMAIN_LEASE_MS)
            .expect_err("stale lease");
        assert_eq!(error.code, ScheduleErrorCode::DomainLocked);
        let error = first
            .lease()
            .release(first.store())
            .expect_err("stale release");
        assert_eq!(error.code, ScheduleErrorCode::DomainLocked);
        // The new owner's lease is still intact.
        second.renew_lease(DEFAULT_DOMAIN_LEASE_MS).expect("renew");
    }

    #[test]
    fn an_existing_unmarked_database_is_refused() {
        let directory = tempdir().expect("tempdir");
        // A database with a table but no experiment marker stands in for a
        // desktop database.
        let path = directory.path().join("chatspeed.db");
        {
            let connection = Connection::open(&path).expect("open");
            connection
                .execute("CREATE TABLE agents (id INTEGER PRIMARY KEY)", [])
                .expect("create");
        }
        let error = open(directory.path(), "owner-a").expect_err("must refuse");
        assert_eq!(error.code, ScheduleErrorCode::DomainUnmarked);
    }

    #[test]
    fn an_empty_database_file_is_treated_as_fresh() {
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("chatspeed.db");
        std::fs::write(&path, b"").expect("touch empty file");
        open(directory.path(), "owner-a").expect("fresh domain");
    }

    #[test]
    fn a_symlinked_data_directory_is_refused() {
        let directory = tempdir().expect("tempdir");
        let real = directory.path().join("real");
        std::fs::create_dir_all(&real).expect("create real dir");
        let link = directory.path().join("link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &link).expect("symlink");
        #[cfg(not(unix))]
        {
            // Without symlink support there is nothing to assert here.
            return;
        }
        let error = open(&link, "owner-a").expect_err("must refuse");
        assert_eq!(error.code, ScheduleErrorCode::DomainLayoutUnsafe);
    }

    #[test]
    fn an_empty_data_dir_argument_is_refused() {
        let error = ExperimentDomain::open("", "owner-a").expect_err("must refuse");
        assert_eq!(error.code, ScheduleErrorCode::DomainLayoutUnsafe);
    }

    #[test]
    fn domain_id_is_stable_and_path_derived() {
        let directory = tempdir().expect("tempdir");
        let first = domain_id_for(directory.path()).expect("id");
        let second = domain_id_for(directory.path()).expect("id");
        assert_eq!(first, second);
        assert!(first.starts_with("domain-"));
        assert_eq!(first.len(), "domain-".len() + 32);
    }

    #[test]
    fn a_database_without_a_marker_row_is_refused() {
        let directory = tempdir().expect("tempdir");
        open(directory.path(), "owner-a").expect("init");
        {
            let path = directory.path().join("chatspeed.db");
            let connection = Connection::open(&path).expect("open");
            connection
                .execute("DELETE FROM experiment_domain", [])
                .expect("drop marker row");
        }
        let error = open(directory.path(), "owner-b").expect_err("must refuse");
        assert_eq!(error.code, ScheduleErrorCode::DomainUnmarked);
    }
}
