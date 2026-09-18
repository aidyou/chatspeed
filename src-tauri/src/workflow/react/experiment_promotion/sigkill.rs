//! Phase 2I real-process SIGKILL / restart matrix.
//!
//! The matrix runs a **real headless instance in a real child process** (the
//! test binary re-executes itself in child mode and boots
//! `headless::bootstrap::start`, control plane, schedulers and all), and the
//! parent `SIGKILL`s it at the effect boundaries the plan names:
//!
//! - `checkpoint_intent` — killed while the checkpoint intent is durable and the
//!   effect may or may not have happened; a restart must converge with the
//!   checkpoint created **at most once**;
//! - `canary` — killed **mid-canary** (the arm containers are up and the arms
//!   are running); a restart must reclaim the dead attempt's arms and still
//!   advance the branch exactly once, leaving no orphan container/worktree;
//! - `branch intent/update` — deterministic pre-states (CAS already applied /
//!   CAS not applied / branch moved to a third value) reconciled by a real
//!   child: roll-forward, retry-once and park respectively.
//!
//! The child is killed with a real `SIGKILL`: no graceful shutdown, no cleanup
//! handlers, exactly like an OOM kill or a power loss.

use super::smoke::{
    smoke_campaign, supervisor, write_bundle, write_profile, write_target, BASE_MEAN, BRANCH,
    IMPROVED_MEAN, LEASE_MS, T0, TARGET_REF,
};
use crate::db::experiment_promotion::ExperimentPromotionStore;
use crate::db::experiment_schedule::ExperimentScheduleStore;
use crate::db::MainStore;
use crate::workflow::react::experiment_promotion::types::{PromotionRequestV1, PromotionState};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::tempdir;

/// The child-mode test the parent re-executes. Harmless (and instant) when the
/// child env is absent, so plain `cargo test` runs are unaffected.
#[test]
fn promotion_sigkill_child_entry() {
    let Some(domain) = std::env::var("PROMOTION_SIGKILL_DOMAIN").ok() else {
        return;
    };
    let repo = std::env::var("PROMOTION_SIGKILL_REPO").expect("child repo env");
    let stop_file = std::env::var("PROMOTION_SIGKILL_STOP").expect("child stop file env");
    let runtime = tokio::runtime::Runtime::new().expect("child runtime");
    runtime.block_on(async move {
        let options = crate::headless::bootstrap::HeadlessOptions::new(&domain)
            .with_base_repo(PathBuf::from(repo));
        let runtime_handle = crate::headless::bootstrap::start(options)
            .await
            .expect("child headless start");
        std::fs::write(
            Path::new(&domain).join("child-ready"),
            format!("pid={}\n", std::process::id()),
        )
        .expect("write ready marker");
        // Wait until the parent asks for a clean stop; a SIGKILL ends us first.
        let deadline = Instant::now() + Duration::from_secs(300);
        while !Path::new(&stop_file).exists() {
            if Instant::now() > deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        runtime_handle.shutdown().await;
    });
}

/// Marks the domain so a real headless child may adopt the database the parent
/// prepared. The marker's id is derived exactly the way
/// `ExperimentDomain::open` derives it: from the canonical data directory.
fn mark_domain(store: &Arc<MainStore>, domain: &Path) {
    use crate::headless::domain::DOMAIN_ID_DOMAIN;
    use crate::workflow::react::campaign::domain_hash;
    let canonical = std::fs::canonicalize(domain)
        .expect("canonicalise domain")
        .to_string_lossy()
        .to_string();
    let domain_id = format!(
        "domain-{}",
        &domain_hash(DOMAIN_ID_DOMAIN, canonical.as_bytes())[..32]
    );
    store
        .db_runtime()
        .expect("runtime")
        .write_blocking(move |conn| {
            conn.execute(
                "INSERT OR IGNORE INTO experiment_domain (
                    domain_id, domain_kind, marker_schema_version, singleton, created_at_ms
                 ) VALUES (?1, 'experiment.v1', 'experiment_domain_marker.v1', 1, 0)",
                rusqlite::params![domain_id],
            )?;
            Ok(())
        })
        .expect("mark domain");
}

/// One SIGKILL scenario result: everything the parent verified about the Git
/// effect and the durable row.
struct MatrixOutcome {
    state: String,
    branch_head: String,
    checkpoint_ref_target: Option<String>,
    journal_intent_count: usize,
    journal_created_count: usize,
    journal_advanced_count: usize,
    orphan_containers: Vec<String>,
    orphan_worktrees: Vec<String>,
    journal_entries:
        Vec<crate::workflow::react::experiment_promotion::types::PromotionJournalEntryV1>,
}

/// A real headless child process running this crate's bootstrap.
struct HeadlessChild {
    child: Child,
    stop_file: PathBuf,
}

impl HeadlessChild {
    fn spawn(domain: &Path, repo: &Path) -> Self {
        let stop_file = domain.join("child-stop");
        let _ = std::fs::remove_file(&stop_file);
        let _ = std::fs::remove_file(domain.join("child-ready"));
        let child = Command::new(std::env::current_exe().expect("current exe"))
            .args([
                "--exact",
                "workflow::react::experiment_promotion::sigkill::promotion_sigkill_child_entry",
                "--nocapture",
            ])
            .env("PROMOTION_SIGKILL_DOMAIN", domain)
            .env("PROMOTION_SIGKILL_REPO", repo)
            .env("PROMOTION_SIGKILL_STOP", &stop_file)
            .env("CHATSPEED_PROMOTION_LEASE_MS", "3000")
            .stdout(Stdio::from(
                std::fs::File::create(domain.join("child-stdout.log")).expect("child stdout"),
            ))
            .stderr(Stdio::from(
                std::fs::File::create(domain.join("child-stderr.log")).expect("child stderr"),
            ))
            .spawn()
            .expect("spawn the headless child");
        let mut child = Self { child, stop_file };
        // Fail fast (with the child's own log) when the child cannot boot, so a
        // broken environment is reported instead of a slow timeout.
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            if domain.join("child-ready").exists() {
                return child;
            }
            if let Ok(Some(status)) = child.child.try_wait() {
                let log =
                    std::fs::read_to_string(domain.join("child-stderr.log")).unwrap_or_default();
                panic!("the headless child exited before it was ready ({status}): {log}");
            }
            assert!(
                Instant::now() < deadline,
                "the headless child never became ready; see {}/child-stderr.log",
                domain.display()
            );
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// Real SIGKILL (the `kill` syscall), not a graceful shutdown.
    fn sigkill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// Graceful stop, used when the scenario is complete.
    fn stop(&mut self) {
        std::fs::write(&self.stop_file, "stop\n").expect("write stop file");
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for HeadlessChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn git(cwd: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "cs-smoke")
        .env("GIT_AUTHOR_EMAIL", "cs-smoke@example.invalid")
        .env("GIT_COMMITTER_NAME", "cs-smoke")
        .env("GIT_COMMITTER_EMAIL", "cs-smoke@example.invalid")
        .output()
        .expect("run git")
}

fn git_ok(cwd: &Path, args: &[&str]) -> String {
    let output = git(cwd, args);
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Reads the durable promotion row through a read-only connection, so the parent
/// never contends with the live child for the write lock.
fn read_state(domain: &Path, promotion_id: &str) -> Option<(String, Option<String>)> {
    let connection = rusqlite::Connection::open_with_flags(
        domain.join("chatspeed.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .ok()?;
    connection.busy_timeout(Duration::from_secs(10)).ok()?;
    connection
        .query_row(
            "SELECT state, error_code FROM experiment_promotions WHERE promotion_id = ?1",
            rusqlite::params![promotion_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .ok()
}

/// Waits until the lease the killed instance held has expired, so a restart may
/// adopt the domain (a live lease is a recoverable crash, not a live owner).
fn wait_for_lease_expiry(domain: &Path) {
    let connection = rusqlite::Connection::open_with_flags(
        domain.join("chatspeed.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("open read-only");
    connection
        .busy_timeout(Duration::from_secs(10))
        .expect("busy timeout");
    let expires: i64 = connection
        .query_row(
            "SELECT COALESCE(MAX(lease_expires_at_ms), 0) FROM experiment_domain_lease",
            [],
            |row| row.get(0),
        )
        .expect("read lease");
    let now = crate::headless::domain::now_ms() as i64;
    if expires > now {
        std::thread::sleep(Duration::from_millis((expires - now) as u64 + 500));
    }
}

fn wait_for_state(domain: &Path, promotion_id: &str, wanted: &str, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        let Some((state, _)) = read_state(domain, promotion_id) else {
            // The child may still be booting; the row appears once it claims.
            if Instant::now() >= deadline {
                panic!("promotion never appeared in the durable store");
            }
            std::thread::sleep(Duration::from_millis(250));
            continue;
        };
        if state == wanted {
            return;
        }
        if Instant::now() >= deadline {
            let stderr =
                std::fs::read_to_string(domain.join("child-stderr.log")).unwrap_or_default();
            let tail: String = stderr.lines().rev().take(40).collect::<Vec<_>>().join("\n");
            let detail = rusqlite::Connection::open_with_flags(
                domain.join("chatspeed.db"),
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .and_then(|connection| {
                connection
                    .query_row(
                        "SELECT state, lease_expires_at_ms, attempt, owner_id
                           FROM experiment_promotions WHERE promotion_id = ?1",
                        rusqlite::params![promotion_id],
                        |row| {
                            Ok(format!(
                                "state={} lease_expires={:?} attempt={} owner={:?}",
                                row.get::<_, String>(0)?,
                                row.get::<_, Option<i64>>(1)?,
                                row.get::<_, i64>(2)?,
                                row.get::<_, Option<String>>(3)?
                            ))
                        },
                    )
                    .map_err(rusqlite::Error::from)
            })
            .unwrap_or_else(|error| format!("row unreadable: {error}"));
            panic!("promotion never reached {wanted}; last state {state}\n{detail}\nchild log tail:\n{tail}");
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn promote_via_store(
    store: &Arc<MainStore>,
    domain: &Path,
    request: &PromotionRequestV1,
    until: PromotionState,
) {
    let promotions = ExperimentPromotionStore::new(store.clone());
    promotions
        .submit(
            request,
            &request.promotion_id(),
            &request.promotion_id(),
            T0,
        )
        .expect("submit");
    let supervisor = supervisor(store, domain, &domain.join("base"));
    for attempt in 0..12u32 {
        if promotions
            .get(&request.promotion_id())
            .expect("record")
            .state
            == until
        {
            return;
        }
        let now = T0 + u64::from(attempt) * (LEASE_MS + 1);
        match supervisor.tick(now) {
            Ok(_) => {}
            Err(error) => panic!("supervisor tick failed: {error}"),
        }
    }
    panic!("supervisor never reached {until:?}");
}

/// Forces the durable row into `advancing` with a recorded branch intent, the
/// exact state a worker is in after the CAS intent was written and before the
/// outcome was recorded.
fn force_advancing(domain: &Path, promotion_id: &str) {
    let connection = rusqlite::Connection::open(domain.join("chatspeed.db")).expect("open");
    connection
        .execute(
            "UPDATE experiment_promotions
                SET state = 'advancing', branch_intent = 'intent_recorded'
              WHERE promotion_id = ?1",
            rusqlite::params![promotion_id],
        )
        .expect("force advancing");
}

fn collect_outcome(
    store: &Arc<MainStore>,
    repo: &Path,
    domain: &Path,
    promotion_id: &str,
) -> MatrixOutcome {
    let promotions = ExperimentPromotionStore::new(store.clone());
    let record = promotions.get(promotion_id).expect("record");
    let branch_head = git_ok(repo, &["rev-parse", BRANCH]);
    let checkpoint_ref =
        crate::workflow::react::experiment_promotion::types::checkpoint_ref_for(promotion_id);
    let checkpoint_ref_target = {
        let output = git(repo, &["rev-parse", "--verify", "--quiet", &checkpoint_ref]);
        if output.status.success() {
            Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            None
        }
    };
    let journal = promotions.journal(promotion_id).expect("journal");
    let count = |stage: &str| journal.iter().filter(|e| e.stage == stage).count();
    let listing = Command::new("docker")
        .args([
            "ps",
            "-a",
            "--filter",
            &format!("name=cs-run-{promotion_id}"),
            "--format",
            "{{.Names}} {{.State}} {{.Label \"cs.generation\"}}",
        ])
        .output()
        .expect("docker ps");
    let orphan_containers: Vec<String> = String::from_utf8_lossy(&listing.stdout)
        .lines()
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect();
    let worktrees = domain.join("worktrees");
    let orphan_worktrees: Vec<String> = if worktrees.is_dir() {
        std::fs::read_dir(&worktrees)
            .expect("read worktrees")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.starts_with(promotion_id))
            .collect()
    } else {
        Vec::new()
    };
    MatrixOutcome {
        state: record.state.as_str().to_string(),
        branch_head,
        checkpoint_ref_target,
        journal_intent_count: count("checkpoint_intent"),
        journal_created_count: count("checkpoint_created"),
        journal_advanced_count: count("branch_advanced"),
        orphan_containers,
        orphan_worktrees,
        journal_entries: journal,
    }
}

/// A real SIGKILL while the checkpoint intent is durable: the restart must
/// converge with the checkpoint created at most once and the branch advanced
/// exactly once.
#[test]
fn sigkill_during_the_checkpoint_boundary_converges() {
    let directory = tempdir().expect("tempdir");
    let domain = directory.path().to_path_buf();
    let (repo, base_head) = super::smoke::repository(&domain);
    let Some(image) = super::smoke::image_or_skip() else {
        eprintln!("skipping: no local digest-pinned image available");
        return;
    };
    write_profile(&domain, &image);
    write_target(&domain);
    write_bundle(&domain);
    let store = Arc::new(MainStore::new(domain.join("chatspeed.db")).expect("store"));
    mark_domain(&store, &domain);
    let schedule = ExperimentScheduleStore::new(store.clone());
    let improve =
        b"--- /dev/null\n+++ b/improvement.txt\n@@ -0,0 +1 @@\n+better-scenario-a\n".to_vec();
    let campaign = smoke_campaign(
        &store,
        &schedule,
        &domain,
        &repo,
        &[("prompt-a", Some(improve))],
    );
    let request = PromotionRequestV1 {
        schema_version: crate::workflow::react::experiment_promotion::types::PROMOTION_REQUEST_V1
            .to_string(),
        campaign_id: campaign.campaign_id.clone(),
        candidate_key: "prompt-a".to_string(),
        target_ref: TARGET_REF.to_string(),
        evidence: super::smoke::evidence(&campaign, "prompt-a", BASE_MEAN, IMPROVED_MEAN),
    };
    let promotion_id = request.promotion_id();
    ExperimentPromotionStore::new(store.clone())
        .submit(&request, &promotion_id, &promotion_id, T0)
        .expect("submit");

    // Child #1: a real headless instance drives the promotion into the
    // checkpoint boundary; the parent SIGKILLs it there.
    let mut child = HeadlessChild::spawn(&domain, &repo);
    wait_for_state(
        &domain,
        &promotion_id,
        "checkpointing",
        Duration::from_secs(60),
    );
    child.sigkill();
    drop(child);

    // Whatever the kill caught, the checkpoint ref either exists or provably
    // does not; both must converge below.
    wait_for_lease_expiry(&domain);

    // Child #2: a fresh headless instance adopts the crashed attempt.
    let mut child = HeadlessChild::spawn(&domain, &repo);
    wait_for_state(&domain, &promotion_id, "promoted", Duration::from_secs(180));
    let outcome = collect_outcome(&store, &repo, &domain, &promotion_id);
    child.stop();
    drop(child);

    assert_eq!(outcome.state, "promoted");
    assert_eq!(
        outcome.checkpoint_ref_target.as_deref(),
        Some(outcome.branch_head.as_str())
    );
    assert_eq!(
        git_ok(
            &repo,
            &[
                "rev-list",
                "--count",
                &format!("{base_head}..{}", outcome.branch_head)
            ]
        ),
        "1",
        "the checkpoint commit must exist exactly once"
    );
    assert_eq!(outcome.journal_intent_count, 1, "one intent, one effect");
    assert_eq!(outcome.journal_created_count, 1);
    assert_eq!(outcome.journal_advanced_count, 1);
    assert!(
        outcome.orphan_containers.is_empty() && outcome.orphan_worktrees.is_empty(),
        "orphan arm resources survived: containers={:?} worktrees={:?}\njournal: {:?}",
        outcome.orphan_containers,
        outcome.orphan_worktrees,
        outcome.journal_entries
    );
    assert!(
        git(&repo, &["config", "--get-regexp", "^remote\\."])
            .stdout
            .is_empty(),
        "the smoke repository must have no remote configured"
    );
}

/// A real SIGKILL **mid-canary** (both arm containers running): the restart must
/// reclaim the dead attempt's arms, re-run the canary once and advance the
/// branch exactly once, leaving no orphan resources.
#[test]
fn sigkill_mid_canary_reclaims_arms_and_converges() {
    let directory = tempdir().expect("tempdir");
    let domain = directory.path().to_path_buf();
    let (repo, base_head) = super::smoke::repository(&domain);
    let Some(image) = super::smoke::image_or_skip() else {
        eprintln!("skipping: no local digest-pinned image available");
        return;
    };
    write_profile(&domain, &image);
    write_target(&domain);
    // A slow canary programme gives the parent a wide, deterministic window in
    // which the arms are demonstrably running.
    super::smoke::write_bundle_with_program(&domain, &super::smoke::canary_program(Some(8)));
    let store = Arc::new(MainStore::new(domain.join("chatspeed.db")).expect("store"));
    mark_domain(&store, &domain);
    let schedule = ExperimentScheduleStore::new(store.clone());
    let improve =
        b"--- /dev/null\n+++ b/improvement.txt\n@@ -0,0 +1 @@\n+better-scenario-b\n".to_vec();
    let campaign = smoke_campaign(
        &store,
        &schedule,
        &domain,
        &repo,
        &[("prompt-a", Some(improve))],
    );
    let request = PromotionRequestV1 {
        schema_version: crate::workflow::react::experiment_promotion::types::PROMOTION_REQUEST_V1
            .to_string(),
        campaign_id: campaign.campaign_id.clone(),
        candidate_key: "prompt-a".to_string(),
        target_ref: TARGET_REF.to_string(),
        evidence: super::smoke::evidence(&campaign, "prompt-a", BASE_MEAN, IMPROVED_MEAN),
    };
    let promotion_id = request.promotion_id();
    ExperimentPromotionStore::new(store.clone())
        .submit(&request, &promotion_id, &promotion_id, T0)
        .expect("submit");

    let mut child = HeadlessChild::spawn(&domain, &repo);
    wait_for_state(
        &domain,
        &promotion_id,
        "canary_running",
        Duration::from_secs(120),
    );
    // Give the arms time to actually come up, then kill mid-effect.
    std::thread::sleep(Duration::from_secs(3));
    child.sigkill();
    drop(child);
    wait_for_lease_expiry(&domain);

    let mut child = HeadlessChild::spawn(&domain, &repo);
    wait_for_state(&domain, &promotion_id, "promoted", Duration::from_secs(240));
    let outcome = collect_outcome(&store, &repo, &domain, &promotion_id);
    child.stop();
    drop(child);

    assert_eq!(outcome.state, "promoted");
    assert_eq!(
        outcome.checkpoint_ref_target.as_deref(),
        Some(outcome.branch_head.as_str())
    );
    assert_eq!(
        git_ok(
            &repo,
            &[
                "rev-list",
                "--count",
                &format!("{base_head}..{}", outcome.branch_head)
            ]
        ),
        "1",
        "the restart must not duplicate the checkpoint commit"
    );
    assert_eq!(outcome.journal_intent_count, 1);
    assert_eq!(outcome.journal_created_count, 1);
    assert_eq!(outcome.journal_advanced_count, 1);
    assert!(
        outcome.orphan_containers.is_empty() && outcome.orphan_worktrees.is_empty(),
        "the restart must reclaim the killed attempt's arms"
    );
}

/// The branch boundary, reconciled by a real headless child process:
/// - CAS already applied + crash → roll-forward, branch unchanged;
/// - CAS not applied → exactly one CAS, branch moves once;
/// - branch moved to a third value → parked, never overwritten.
#[test]
fn sigkill_at_the_branch_boundary_rolls_forward_retries_or_parks() {
    // ---- roll-forward ------------------------------------------------------
    let directory = tempdir().expect("tempdir");
    let domain = directory.path().to_path_buf();
    let (repo, _base_head) = super::smoke::repository(&domain);
    let Some(image) = super::smoke::image_or_skip() else {
        eprintln!("skipping: no local digest-pinned image available");
        return;
    };
    write_profile(&domain, &image);
    write_target(&domain);
    write_bundle(&domain);
    let store = Arc::new(MainStore::new(domain.join("chatspeed.db")).expect("store"));
    mark_domain(&store, &domain);
    let schedule = ExperimentScheduleStore::new(store.clone());
    let improve =
        b"--- /dev/null\n+++ b/improvement.txt\n@@ -0,0 +1 @@\n+better-scenario-c1\n".to_vec();
    let campaign = smoke_campaign(
        &store,
        &schedule,
        &domain,
        &repo,
        &[("prompt-a", Some(improve))],
    );
    let request = PromotionRequestV1 {
        schema_version: crate::workflow::react::experiment_promotion::types::PROMOTION_REQUEST_V1
            .to_string(),
        campaign_id: campaign.campaign_id.clone(),
        candidate_key: "prompt-a".to_string(),
        target_ref: TARGET_REF.to_string(),
        evidence: super::smoke::evidence(&campaign, "prompt-a", BASE_MEAN, IMPROVED_MEAN),
    };
    let promotion_id = request.promotion_id();

    // Drive the real checkpoint to existence with the in-process supervisor.
    promote_via_store(&store, &domain, &request, PromotionState::Checkpointed);
    let checkpoint_commit = git_ok(
        &repo,
        &[
            "rev-parse",
            &format!("refs/chatspeed/checkpoints/{promotion_id}"),
        ],
    );
    // Simulate the crashed worker: the CAS was applied, the outcome was never
    // recorded.
    force_advancing(&domain, &promotion_id);
    git_ok(&repo, &["update-ref", BRANCH, &checkpoint_commit]);

    let mut child = HeadlessChild::spawn(&domain, &repo);
    wait_for_state(&domain, &promotion_id, "promoted", Duration::from_secs(120));
    let outcome = collect_outcome(&store, &repo, &domain, &promotion_id);
    child.stop();
    drop(child);

    assert_eq!(
        outcome.branch_head, checkpoint_commit,
        "roll-forward must not move the branch again"
    );
    assert_eq!(outcome.journal_advanced_count, 1);
    assert!(outcome.orphan_containers.is_empty() && outcome.orphan_worktrees.is_empty());

    // ---- retry-once --------------------------------------------------------
    let directory = tempdir().expect("tempdir");
    let domain = directory.path().to_path_buf();
    let (repo, base_head) = super::smoke::repository(&domain);
    let Some(image) = super::smoke::image_or_skip() else {
        eprintln!("skipping: no local digest-pinned image available");
        return;
    };
    write_profile(&domain, &image);
    write_target(&domain);
    write_bundle(&domain);
    let store = Arc::new(MainStore::new(domain.join("chatspeed.db")).expect("store"));
    mark_domain(&store, &domain);
    let schedule = ExperimentScheduleStore::new(store.clone());
    let improve =
        b"--- /dev/null\n+++ b/improvement.txt\n@@ -0,0 +1 @@\n+better-scenario-c2\n".to_vec();
    let campaign = smoke_campaign(
        &store,
        &schedule,
        &domain,
        &repo,
        &[("prompt-a", Some(improve))],
    );
    let request = PromotionRequestV1 {
        schema_version: crate::workflow::react::experiment_promotion::types::PROMOTION_REQUEST_V1
            .to_string(),
        campaign_id: campaign.campaign_id.clone(),
        candidate_key: "prompt-a".to_string(),
        target_ref: TARGET_REF.to_string(),
        evidence: super::smoke::evidence(&campaign, "prompt-a", BASE_MEAN, IMPROVED_MEAN),
    };
    let promotion_id = request.promotion_id();
    promote_via_store(&store, &domain, &request, PromotionState::Checkpointed);
    let checkpoint_commit = git_ok(
        &repo,
        &[
            "rev-parse",
            &format!("refs/chatspeed/checkpoints/{promotion_id}"),
        ],
    );
    force_advancing(&domain, &promotion_id);
    // The CAS never landed: the branch is still on the old head.

    let mut child = HeadlessChild::spawn(&domain, &repo);
    wait_for_state(&domain, &promotion_id, "promoted", Duration::from_secs(120));
    let outcome = collect_outcome(&store, &repo, &domain, &promotion_id);
    child.stop();
    drop(child);

    assert_eq!(
        outcome.branch_head, checkpoint_commit,
        "the retry must apply the CAS once"
    );
    assert_eq!(
        git_ok(
            &repo,
            &[
                "rev-list",
                "--count",
                &format!("{base_head}..{}", outcome.branch_head)
            ]
        ),
        "1"
    );
    assert_eq!(outcome.journal_advanced_count, 1);

    // ---- third value is parked, never overwritten --------------------------
    let directory = tempdir().expect("tempdir");
    let domain = directory.path().to_path_buf();
    let (repo, base_head) = super::smoke::repository(&domain);
    let Some(image) = super::smoke::image_or_skip() else {
        eprintln!("skipping: no local digest-pinned image available");
        return;
    };
    write_profile(&domain, &image);
    write_target(&domain);
    write_bundle(&domain);
    let store = Arc::new(MainStore::new(domain.join("chatspeed.db")).expect("store"));
    mark_domain(&store, &domain);
    let schedule = ExperimentScheduleStore::new(store.clone());
    let improve =
        b"--- /dev/null\n+++ b/improvement.txt\n@@ -0,0 +1 @@\n+better-scenario-c3\n".to_vec();
    let campaign = smoke_campaign(
        &store,
        &schedule,
        &domain,
        &repo,
        &[("prompt-a", Some(improve))],
    );
    let request = PromotionRequestV1 {
        schema_version: crate::workflow::react::experiment_promotion::types::PROMOTION_REQUEST_V1
            .to_string(),
        campaign_id: campaign.campaign_id.clone(),
        candidate_key: "prompt-a".to_string(),
        target_ref: TARGET_REF.to_string(),
        evidence: super::smoke::evidence(&campaign, "prompt-a", BASE_MEAN, IMPROVED_MEAN),
    };
    let promotion_id = request.promotion_id();
    promote_via_store(&store, &domain, &request, PromotionState::Checkpointed);
    force_advancing(&domain, &promotion_id);
    let checkpoint_commit_hint = git_ok(
        &repo,
        &[
            "rev-parse",
            &format!("refs/chatspeed/checkpoints/{promotion_id}"),
        ],
    );
    // An external actor moved the branch somewhere that is neither the expected
    // old head nor the checkpoint: a fresh orphan commit object.
    let tree = git_ok(&repo, &["rev-parse", &format!("{base_head}^{{tree}}")]);
    let third_value = git_ok(
        &repo,
        &["commit-tree", &tree, "-m", "third-value (external actor)"],
    );
    assert_ne!(third_value, base_head);
    assert_ne!(third_value, checkpoint_commit_hint);
    git_ok(&repo, &["update-ref", BRANCH, &third_value]);

    let mut child = HeadlessChild::spawn(&domain, &repo);
    wait_for_state(
        &domain,
        &promotion_id,
        "unknown_manual",
        Duration::from_secs(120),
    );
    let outcome = collect_outcome(&store, &repo, &domain, &promotion_id);
    child.stop();
    drop(child);

    assert_eq!(outcome.state, "unknown_manual");
    assert_eq!(
        git_ok(&repo, &["rev-parse", BRANCH]),
        third_value,
        "a third value must never be overwritten"
    );
    assert_eq!(outcome.journal_advanced_count, 0);
    // The checkpoint evidence is retained.
    assert!(outcome.checkpoint_ref_target.is_some());
}
