//! Durable experiment-domain and campaign-schedule store (Phase 2G+2H).
//!
//! Every operation runs inside one `DbRuntime` writer transaction, so a
//! schedule submission, a lease claim, a fenced state transition and its
//! journal entry either all commit or all roll back. There is no second data
//! authority: the scheduler, the owner saga and the control plane all go
//! through this store, which itself goes through [`MainStore`].
//!
//! Ownership and effect rules encoded here (INV-3/INV-5/INV-8/INV-9):
//!
//! - A claim is a compare-and-swap on `(job_id, state, lease_generation)`; the
//!   generation is bumped by the winner, so a stale worker's later writes match
//!   zero rows and are rejected with `lease_lost`.
//! - A claim never picks up a job whose dispatch marker is not
//!   `not_dispatched`. Intent-recorded work is only ever parked for a human.
//! - The stored plan/fixture/bundle payload contains refs, digests and typed
//!   state only — never an instruction, prompt, response, token or environment.

use crate::db::{MainStore, StoreError};
use crate::workflow::react::campaign::{campaign_id_for_plan, CampaignPlanV1};
use crate::workflow::react::experiment_schedule::fixture::{resolve_task_ref, FixtureTaskRefV1};
use crate::workflow::react::experiment_schedule::types::{
    classify_restart_recovery, is_sha256_hex, job_id_for, validate_dispatch_invariant,
    validate_transition, CampaignJobV1, CampaignScheduleAcceptedV1, CampaignScheduleRequestV1,
    DispatchMarker, JobSagaStage, JobState, OwnerFence, RecoveryDecision, ScheduleError,
    ScheduleErrorCode, CAMPAIGN_JOB_V1, SCHEDULE_CONCURRENCY,
};
use rusqlite::{params, OptionalExtension, Transaction};
use std::sync::Arc;

/// Lifecycle status of a durable campaign schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CampaignStatus {
    Active,
    Closed,
    Cancelled,
}

impl CampaignStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            CampaignStatus::Active => "active",
            CampaignStatus::Closed => "closed",
            CampaignStatus::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "active" => CampaignStatus::Active,
            "closed" => CampaignStatus::Closed,
            "cancelled" => CampaignStatus::Cancelled,
            _ => return None,
        })
    }

    pub fn accepts_new_work(&self) -> bool {
        matches!(self, CampaignStatus::Active)
    }
}

/// The durable schedule record, projected for the HTTP/CLI surface.
#[derive(Debug, Clone)]
pub struct CampaignRecord {
    pub campaign_id: String,
    pub campaign_key: String,
    pub plan_hash: String,
    pub schedule_hash: String,
    pub status: CampaignStatus,
    pub execution_profile_ref: String,
    pub plan: CampaignPlanV1,
    pub fixture_refs: Vec<FixtureTaskRefV1>,
    pub bundle_refs: Vec<String>,
    pub job_ids: Vec<String>,
}

/// A durable job record plus the store-only fields the scheduler needs.
#[derive(Debug, Clone)]
pub struct JobRecord {
    pub job: CampaignJobV1,
    pub session_id: Option<String>,
    pub artifact_dir: Option<String>,
    pub owner_id: Option<String>,
    pub lease_expires_at_ms: Option<u64>,
    pub heartbeat_at_ms: Option<u64>,
}

/// A durable artifact row recorded for a job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobArtifactRecord {
    pub artifact_id: String,
    pub kind: String,
    pub relative_path: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub base_revision: Option<String>,
    pub run_id: Option<String>,
    pub session_id: Option<String>,
    pub candidate_key: Option<String>,
}

/// One classified non-terminal job from the restart sweep.
#[derive(Debug, Clone)]
pub struct RecoveryRecord {
    pub job_id: String,
    pub state: JobState,
    pub dispatch_marker: DispatchMarker,
    pub run_id: Option<String>,
    pub decision: RecoveryDecision,
}

/// The outcome of a schedule submission: `created` is false when the same
/// idempotency key replayed the same body (no new rows were written).
#[derive(Debug, Clone)]
pub struct ScheduleOutcome {
    pub accepted: CampaignScheduleAcceptedV1,
    pub created: bool,
}

/// The outcome of a claim attempt.
#[derive(Debug, Clone)]
pub enum ClaimOutcome {
    /// A job was claimed or adopted with a fresh lease generation.
    Claimed(Box<JobRecord>),
    /// No claimable work (queue empty, campaigns closed, or leases live).
    Idle,
}

/// Builds a store error carrying a stable machine code.
pub fn store_error(code: ScheduleErrorCode, message: impl Into<String>) -> ScheduleError {
    ScheduleError::new(code, message)
}

/// Maps a durable-store or rusqlite failure onto the schedule error surface.
pub fn persistence_error(error: impl std::fmt::Display) -> ScheduleError {
    ScheduleError::new(
        ScheduleErrorCode::PersistenceFailure,
        format!("durable schedule store failure: {error}"),
    )
}

/// Extracts the typed [`ScheduleError`] a writer closure produced, or maps the
/// outer store failure. Mirrors the flattening helper used by the 2B ledger.
fn flatten<T>(result: Result<Result<T, ScheduleError>, StoreError>) -> Result<T, ScheduleError> {
    match result {
        Ok(inner) => inner,
        Err(error) => Err(persistence_error(error)),
    }
}

/// The durable experiment schedule store.
#[derive(Clone)]
pub struct ExperimentScheduleStore {
    store: Arc<MainStore>,
}

impl ExperimentScheduleStore {
    pub fn new(store: Arc<MainStore>) -> Self {
        Self { store }
    }

    /// The underlying store, used by in-crate tests that must inspect the raw
    /// durable bytes. Production code goes through the store's own API.
    #[cfg(test)]
    pub(crate) fn main_store(&self) -> &Arc<MainStore> {
        &self.store
    }

    /// Whether this database carries the experiment-domain marker.
    ///
    /// The durable schedule surface is only valid inside an experiment domain;
    /// a desktop database answers `false` and therefore refuses to enqueue work
    /// (AC-1/AC-2). Nothing is written by this check.
    pub fn domain_is_marked(&self) -> Result<bool, ScheduleError> {
        let runtime = self
            .store
            .db_runtime()
            .map_err(|error| persistence_error(error))?;
        flatten(runtime.read_blocking(|conn| {
            let inner = (|| -> Result<bool, ScheduleError> {
                let count: i64 = conn
                    .query_row("SELECT COUNT(1) FROM experiment_domain", [], |row| {
                        row.get(0)
                    })
                    .map_err(persistence_error)?;
                Ok(count > 0)
            })();
            Ok(inner)
        }))
    }

    /// Whether a campaign exists (used to distinguish "unknown" from "empty").
    pub fn campaign_exists(&self, campaign_id: &str) -> Result<bool, ScheduleError> {
        match self.get_campaign(campaign_id) {
            Ok(_) => Ok(true),
            Err(error) if error.code == ScheduleErrorCode::UnknownCampaign => Ok(false),
            Err(error) => Err(error),
        }
    }

    // -----------------------------------------------------------------------
    // Campaign schedule
    // -----------------------------------------------------------------------

    /// Persists one validated schedule request, creating the ordered job list
    /// in the same transaction. Replaying the same idempotency key with the
    /// same body returns the original acceptance; a different body under the
    /// same key, or a different plan under the same campaign key, is rejected.
    pub fn schedule_campaign(
        &self,
        request: &CampaignScheduleRequestV1,
        idempotency_key: &str,
        now_ms: u64,
    ) -> Result<ScheduleOutcome, ScheduleError> {
        let plan = request.plan.clone();
        let plan_hash = plan.plan_hash();
        let campaign_id = campaign_id_for_plan(&plan_hash);
        let schedule_hash =
            crate::workflow::react::experiment_schedule::types::schedule_request_hash(request);
        let request = request.clone();
        let idempotency_key = idempotency_key.to_string();

        let runtime = self
            .store
            .db_runtime()
            .map_err(|error| persistence_error(error))?;
        let campaign_id_for_closure = campaign_id.clone();
        let outcome = flatten(runtime.write_blocking(move |conn| {
            let inner = (|| -> Result<ScheduleOutcome, ScheduleError> {
                let tx = conn.transaction().map_err(persistence_error)?;

                // Idempotent replay: same key and same schedule hash.
                let existing: Option<(String, String, String)> = tx
                    .query_row(
                        "SELECT campaign_id, schedule_hash, status
                           FROM experiment_campaign_schedules
                          WHERE idempotency_key = ?1",
                        params![idempotency_key],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .optional()
                    .map_err(persistence_error)?;
                if let Some((existing_id, existing_hash, _status)) = existing {
                    if existing_hash != schedule_hash {
                        return Err(store_error(
                            ScheduleErrorCode::IdempotencyConflict,
                            "the idempotency key was already used for a different schedule body",
                        ));
                    }
                    let record = load_campaign(&tx, &existing_id)?;
                    tx.commit().map_err(persistence_error)?;
                    return Ok(ScheduleOutcome {
                        accepted: accepted_projection(&record),
                        created: false,
                    });
                }

                // A campaign key is unique: the same key with a different plan
                // is a caller bug, not a new campaign.
                let conflicting: Option<String> = tx
                    .query_row(
                        "SELECT campaign_id FROM experiment_campaign_schedules WHERE campaign_key = ?1",
                        params![plan.campaign_key],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(persistence_error)?;
                if let Some(existing_id) = conflicting {
                    if existing_id != campaign_id_for_closure {
                        return Err(store_error(
                            ScheduleErrorCode::IdempotencyConflict,
                            "the campaign key is already bound to a different plan",
                        ));
                    }
                }

                let plan_json = serde_json::to_string(&plan).map_err(|error| {
                    store_error(
                        ScheduleErrorCode::PersistenceFailure,
                        format!("plan is not serializable: {error}"),
                    )
                })?;
                let fixture_json = serde_json::to_string(&request.fixture_refs).map_err(|error| {
                    store_error(
                        ScheduleErrorCode::PersistenceFailure,
                        format!("fixture refs are not serializable: {error}"),
                    )
                })?;
                let bundle_json = serde_json::to_string(&request.bundle_refs).map_err(|error| {
                    store_error(
                        ScheduleErrorCode::PersistenceFailure,
                        format!("bundle refs are not serializable: {error}"),
                    )
                })?;

                tx.execute(
                    "INSERT INTO experiment_campaign_schedules (
                        campaign_id, campaign_key, plan_hash, schedule_hash, plan_json,
                        fixture_refs_json, execution_profile_ref, bundle_refs_json,
                        concurrency, status, idempotency_key, created_at_ms, updated_at_ms
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'active', ?10, ?11, ?11)",
                    params![
                        campaign_id_for_closure,
                        plan.campaign_key,
                        plan_hash,
                        schedule_hash,
                        plan_json,
                        fixture_json,
                        request.execution_profile_ref,
                        bundle_json,
                        SCHEDULE_CONCURRENCY,
                        idempotency_key,
                        now_ms as i64,
                    ],
                )
                .map_err(persistence_error)?;

                // Ordered jobs: one per declared candidate, in plan order. The
                // candidate list is validated by the 2F plan parser already.
                let mut job_ids = Vec::with_capacity(plan.candidates.len());
                for (ordinal, candidate) in plan.candidates.iter().enumerate() {
                    let task_id = plan.task.clone();
                    let job_id = job_id_for(
                        &campaign_id_for_closure,
                        &candidate.candidate_key,
                        &task_id,
                        ordinal as u32,
                    );
                    tx.execute(
                        "INSERT INTO experiment_campaign_jobs (
                            job_id, campaign_id, ordinal, candidate_key, task_id, suite,
                            dataset_id, dataset_version, split, manifest_digest, task_digest,
                            instruction_hash, execution_profile_ref, state, dispatch_marker,
                            attempt, lease_generation, created_at_ms, updated_at_ms
                         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                                   'queued', 'not_dispatched', 0, 0, ?14, ?14)",
                        params![
                            job_id,
                            campaign_id_for_closure,
                            ordinal as i64,
                            candidate.candidate_key,
                            task_id,
                            fixture_suite(&request)?,
                            fixture_dataset_id(&request)?,
                            fixture_dataset_version(&request)?,
                            fixture_split(&request)?,
                            fixture_manifest_digest(&request)?,
                            fixture_task_digest(&request)?,
                            fixture_instruction_hash(&request)?,
                            request.execution_profile_ref,
                            now_ms as i64,
                        ],
                    )
                    .map_err(persistence_error)?;
                    job_ids.push(job_id);
                }

                let record = load_campaign(&tx, &campaign_id_for_closure)?;
                tx.commit().map_err(persistence_error)?;
                Ok(ScheduleOutcome {
                    accepted: accepted_projection(&record),
                    created: true,
                })
            })();
            Ok(inner)
        }))?;

        if outcome.accepted.job_ids.is_empty() {
            return Err(store_error(
                ScheduleErrorCode::InvalidJobState,
                "the schedule produced no durable jobs",
            ));
        }
        Ok(outcome)
    }

    /// Loads one campaign by id.
    pub fn get_campaign(&self, campaign_id: &str) -> Result<CampaignRecord, ScheduleError> {
        let runtime = self
            .store
            .db_runtime()
            .map_err(|error| persistence_error(error))?;
        let campaign_id = campaign_id.to_string();
        flatten(runtime.read_blocking(move |conn| Ok(load_campaign(conn, &campaign_id))))
    }

    /// Loads the durable job list of a campaign, ordered by ordinal.
    pub fn list_jobs(&self, campaign_id: &str) -> Result<Vec<JobRecord>, ScheduleError> {
        let runtime = self
            .store
            .db_runtime()
            .map_err(|error| persistence_error(error))?;
        let campaign_id = campaign_id.to_string();
        flatten(runtime.read_blocking(move |conn| {
            let inner = (|| -> Result<Vec<JobRecord>, ScheduleError> {
                // A job list exists only for a campaign that exists: an unknown
                // id fails closed instead of looking like an empty queue.
                load_campaign(conn, &campaign_id)?;
                let mut statement = conn
                    .prepare(
                        "SELECT job_id, campaign_id, ordinal, candidate_key, task_id, suite,
                            dataset_id, dataset_version, split, manifest_digest, task_digest,
                            instruction_hash, execution_profile_ref, state, dispatch_marker,
                            run_id, session_id, attempt, owner_id, lease_generation,
                            lease_expires_at_ms, heartbeat_at_ms, last_stage, error_code,
                            artifact_dir
                       FROM experiment_campaign_jobs
                      WHERE campaign_id = ?1
                      ORDER BY ordinal ASC",
                    )
                    .map_err(persistence_error)?;
                let rows = statement
                    .query_map(params![campaign_id], map_job_row)
                    .map_err(persistence_error)?;
                let mut jobs = Vec::new();
                for row in rows {
                    jobs.push(row.map_err(persistence_error)?);
                }
                Ok(jobs)
            })();
            Ok(inner)
        }))
    }

    /// Loads one job by id.
    pub fn get_job(&self, job_id: &str) -> Result<JobRecord, ScheduleError> {
        let runtime = self
            .store
            .db_runtime()
            .map_err(|error| persistence_error(error))?;
        let job_id = job_id.to_string();
        flatten(runtime.read_blocking(move |conn| Ok(load_job(conn, &job_id))))
    }

    /// The durable artifact rows recorded for one job, oldest first.
    ///
    /// The Phase 2I promotion binding reads this instead of trusting a caller's
    /// projection: the digest the CLI advertises for the candidate patch must
    /// equal the digest the scheduler itself recorded when it published the
    /// artifact (AC-2/INV-2).
    pub fn job_artifacts(&self, job_id: &str) -> Result<Vec<JobArtifactRecord>, ScheduleError> {
        let runtime = self
            .store
            .db_runtime()
            .map_err(|error| persistence_error(error))?;
        let job_id = job_id.to_string();
        flatten(runtime.read_blocking(move |conn| {
            let inner = (|| -> Result<Vec<JobArtifactRecord>, ScheduleError> {
                let mut statement = conn
                    .prepare(
                        "SELECT artifact_id, kind, relative_path, sha256, size_bytes,
                                base_revision, run_id, session_id, candidate_key
                           FROM experiment_job_artifacts
                          WHERE job_id = ?1
                          ORDER BY created_at_ms ASC, artifact_id ASC",
                    )
                    .map_err(persistence_error)?;
                let rows = statement
                    .query_map(params![job_id], |row| {
                        Ok(JobArtifactRecord {
                            artifact_id: row.get(0)?,
                            kind: row.get(1)?,
                            relative_path: row.get(2)?,
                            sha256: row.get(3)?,
                            size_bytes: row.get::<_, i64>(4)? as u64,
                            base_revision: row.get(5)?,
                            run_id: row.get(6)?,
                            session_id: row.get(7)?,
                            candidate_key: row.get(8)?,
                        })
                    })
                    .map_err(persistence_error)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(persistence_error)?;
                Ok(rows)
            })();
            Ok(inner)
        }))
    }

    /// Closes or cancels a campaign. Closing never rewrites an existing job
    /// outcome; it only stops new claims.
    pub fn set_campaign_status(
        &self,
        campaign_id: &str,
        status: CampaignStatus,
        now_ms: u64,
    ) -> Result<CampaignRecord, ScheduleError> {
        let runtime = self
            .store
            .db_runtime()
            .map_err(|error| persistence_error(error))?;
        let campaign_id = campaign_id.to_string();
        flatten(runtime.write_blocking(move |conn| {
            let inner = (|| -> Result<CampaignRecord, ScheduleError> {
                let tx = conn.transaction().map_err(persistence_error)?;
                let current = load_campaign(&tx, &campaign_id)?;
                if !current.status.accepts_new_work() {
                    // Idempotent: re-closing an already closed campaign is fine,
                    // re-opening one is not part of the contract.
                    if current.status == status {
                        return Ok(current);
                    }
                    return Err(store_error(
                        ScheduleErrorCode::CampaignNotActive,
                        format!(
                            "campaign is already {} and cannot become {}",
                            current.status.as_str(),
                            status.as_str()
                        ),
                    ));
                }
                tx.execute(
                    "UPDATE experiment_campaign_schedules
                        SET status = ?2, updated_at_ms = ?3
                      WHERE campaign_id = ?1",
                    params![campaign_id, status.as_str(), now_ms as i64],
                )
                .map_err(persistence_error)?;
                let record = load_campaign(&tx, &campaign_id)?;
                tx.commit().map_err(persistence_error)?;
                Ok(record)
            })();
            Ok(inner)
        }))
    }

    /// Cancels every job of a campaign that has provably not been dispatched.
    /// A dispatched job is never cancelled here: the FSM forbids it and the
    /// caller must stop the run through the run kernel instead.
    pub fn cancel_pre_dispatch_jobs(
        &self,
        campaign_id: &str,
        now_ms: u64,
    ) -> Result<Vec<String>, ScheduleError> {
        let runtime = self
            .store
            .db_runtime()
            .map_err(|error| persistence_error(error))?;
        let campaign_id = campaign_id.to_string();
        flatten(runtime.write_blocking(move |conn| {
            let inner = (|| -> Result<Vec<String>, ScheduleError> {
                let tx = conn.transaction().map_err(persistence_error)?;
                let mut statement = tx
                    .prepare(
                        "SELECT job_id, state, dispatch_marker
                           FROM experiment_campaign_jobs
                          WHERE campaign_id = ?1",
                    )
                    .map_err(persistence_error)?;
                let rows: Vec<(String, String, String)> = statement
                    .query_map(params![campaign_id], |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                    })
                    .map_err(persistence_error)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(persistence_error)?;
                drop(statement);

                let mut cancelled = Vec::new();
                for (job_id, state, marker) in rows {
                    let state = JobState::parse(&state).ok_or_else(|| {
                        store_error(
                            ScheduleErrorCode::InvalidJobState,
                            "durable job has an unrecognized state",
                        )
                    })?;
                    let marker = DispatchMarker::parse(&marker).ok_or_else(|| {
                        store_error(
                            ScheduleErrorCode::InvalidJobState,
                            "durable job has an unrecognized dispatch marker",
                        )
                    })?;
                    if state.is_terminal() {
                        continue;
                    }
                    // Only a provably-undispatched job may be cancelled here.
                    if marker != DispatchMarker::NotDispatched {
                        continue;
                    }
                    let from_state = state.as_str();
                    let changed = tx
                        .execute(
                            "UPDATE experiment_campaign_jobs
                                SET state = 'cancelled', owner_id = NULL,
                                    lease_expires_at_ms = NULL, heartbeat_at_ms = NULL,
                                    updated_at_ms = ?3
                              WHERE job_id = ?1
                                AND state = ?2
                                AND dispatch_marker = 'not_dispatched'",
                            params![job_id, from_state, now_ms as i64],
                        )
                        .map_err(persistence_error)?;
                    if changed == 1 {
                        let generation: i64 = tx
                            .query_row(
                                "SELECT lease_generation FROM experiment_campaign_jobs WHERE job_id = ?1",
                                params![job_id],
                                |row| row.get(0),
                            )
                            .map_err(persistence_error)?;
                        insert_journal(
                            &tx,
                            &job_id,
                            JobSagaStage::CleanupDone,
                            None,
                            generation,
                            Some("cancelled"),
                            now_ms,
                        )?;
                        cancelled.push(job_id);
                    }
                }
                tx.commit().map_err(persistence_error)?;
                Ok(cancelled)
            })();
            Ok(inner)
        }))
    }

    // -----------------------------------------------------------------------
    // Lease claim / heartbeat / fenced transition
    // -----------------------------------------------------------------------

    /// Claims the next claimable job with a fresh lease generation.
    ///
    /// Claimable means: the campaign is active, the dispatch marker is
    /// `not_dispatched` (so no dispatch intent was ever written), and the job
    /// is either `queued` or a `preparing`/`prepared` job whose lease provably
    /// expired. A `dispatching`/`running`/`collecting` job is never claimable
    /// here: recovery handles those through [`Self::adopt_recoverable_job`].
    pub fn claim_next_job(
        &self,
        owner_id: &str,
        now_ms: u64,
        lease_ms: u64,
    ) -> Result<ClaimOutcome, ScheduleError> {
        let runtime = self
            .store
            .db_runtime()
            .map_err(|error| persistence_error(error))?;
        let owner_id = owner_id.to_string();
        let lease_until = now_ms.saturating_add(lease_ms) as i64;
        flatten(runtime.write_blocking(move |conn| {
            let inner = (|| -> Result<ClaimOutcome, ScheduleError> {
                let tx = conn.transaction().map_err(persistence_error)?;
                let candidate: Option<(String, String, i64)> = tx
                    .query_row(
                        "SELECT j.job_id, j.state, j.lease_generation
                           FROM experiment_campaign_jobs j
                           JOIN experiment_campaign_schedules s
                             ON s.campaign_id = j.campaign_id
                          WHERE s.status = 'active'
                            AND j.dispatch_marker = 'not_dispatched'
                            AND (
                                j.state = 'queued'
                                OR (
                                    j.state IN ('preparing','prepared')
                                    AND (j.lease_expires_at_ms IS NULL
                                         OR j.lease_expires_at_ms <= ?1)
                                )
                            )
                          ORDER BY j.campaign_id ASC, j.ordinal ASC
                          LIMIT 1",
                        params![now_ms as i64],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .optional()
                    .map_err(persistence_error)?;
                let Some((job_id, previous_state, generation)) = candidate else {
                    tx.commit().map_err(persistence_error)?;
                    return Ok(ClaimOutcome::Idle);
                };
                let adopted = previous_state != "queued";
                let changed = tx
                    .execute(
                        "UPDATE experiment_campaign_jobs
                            SET state = 'preparing',
                                owner_id = ?2,
                                lease_generation = lease_generation + 1,
                                lease_expires_at_ms = ?3,
                                heartbeat_at_ms = ?4,
                                attempt = attempt + 1,
                                updated_at_ms = ?4
                          WHERE job_id = ?1
                            AND lease_generation = ?5
                            AND state = ?6
                            AND dispatch_marker = 'not_dispatched'",
                        params![
                            job_id,
                            owner_id,
                            lease_until,
                            now_ms as i64,
                            generation,
                            previous_state,
                        ],
                    )
                    .map_err(persistence_error)?;
                if changed != 1 {
                    // Someone else won the race; roll back and let the caller
                    // simply poll again. No effect was recorded.
                    tx.rollback().map_err(persistence_error)?;
                    return Ok(ClaimOutcome::Idle);
                }
                let record = load_job(&tx, &job_id)?;
                if adopted {
                    insert_journal(
                        &tx,
                        &job_id,
                        JobSagaStage::WorkspaceAcquired,
                        Some(&owner_id),
                        record.job.lease_generation,
                        Some("lease_adopted"),
                        now_ms,
                    )?;
                }
                tx.commit().map_err(persistence_error)?;
                Ok(ClaimOutcome::Claimed(Box::new(record)))
            })();
            Ok(inner)
        }))
    }

    /// Adopts a post-dispatch job whose lease provably expired, with a new
    /// owner and a bumped generation. Only `preparing`/`prepared`/`running`/
    /// `collecting` jobs may be adopted; `dispatching` never is, because its
    /// effect cannot be proven absent (INV-5).
    pub fn adopt_recoverable_job(
        &self,
        job_id: &str,
        owner_id: &str,
        now_ms: u64,
        lease_ms: u64,
    ) -> Result<JobRecord, ScheduleError> {
        let runtime = self
            .store
            .db_runtime()
            .map_err(|error| persistence_error(error))?;
        let job_id = job_id.to_string();
        let owner_id = owner_id.to_string();
        let lease_until = now_ms.saturating_add(lease_ms) as i64;
        flatten(runtime.write_blocking(move |conn| {
            let inner = (|| -> Result<JobRecord, ScheduleError> {
                let tx = conn.transaction().map_err(persistence_error)?;
                let current = load_job(&tx, &job_id)?;
                if current.job.state.is_terminal() {
                    return Err(store_error(
                        ScheduleErrorCode::InvalidJobTransition,
                        format!(
                            "job '{}' is terminal ({}) and is never re-adopted",
                            job_id,
                            current.job.state.as_str()
                        ),
                    ));
                }
                if !matches!(
                    current.job.state,
                    JobState::Preparing
                        | JobState::Prepared
                        | JobState::Running
                        | JobState::Collecting
                ) {
                    return Err(store_error(
                        ScheduleErrorCode::DispatchUncertain,
                        format!(
                            "job '{}' in state '{}' is never automatically adopted",
                            job_id,
                            current.job.state.as_str()
                        ),
                    ));
                }
                let lease_expired = current
                    .lease_expires_at_ms
                    .map(|expires| expires <= now_ms)
                    .unwrap_or(true);
                if !lease_expired {
                    return Err(store_error(
                        ScheduleErrorCode::LeaseConflict,
                        format!("job '{job_id}' still holds a live lease"),
                    ));
                }
                let from_state = current.job.state.as_str();
                let changed = tx
                    .execute(
                        "UPDATE experiment_campaign_jobs
                            SET owner_id = ?2,
                                lease_generation = lease_generation + 1,
                                lease_expires_at_ms = ?3,
                                heartbeat_at_ms = ?4,
                                updated_at_ms = ?4
                          WHERE job_id = ?1
                            AND state = ?5
                            AND lease_generation = ?6",
                        params![
                            job_id,
                            owner_id,
                            lease_until,
                            now_ms as i64,
                            from_state,
                            current.job.lease_generation,
                        ],
                    )
                    .map_err(persistence_error)?;
                if changed != 1 {
                    return Err(store_error(
                        ScheduleErrorCode::LeaseLost,
                        format!("job '{job_id}' was adopted by another owner"),
                    ));
                }
                let record = load_job(&tx, &job_id)?;
                insert_journal(
                    &tx,
                    &job_id,
                    JobSagaStage::WorkspaceAcquired,
                    Some(&owner_id),
                    record.job.lease_generation,
                    Some("post_dispatch_adopted"),
                    now_ms,
                )?;
                tx.commit().map_err(persistence_error)?;
                Ok(record)
            })();
            Ok(inner)
        }))
    }

    /// Renews a lease. The generation must still be current, otherwise the
    /// worker learns it lost ownership before doing anything else (INV-8).
    pub fn heartbeat(
        &self,
        fence: &OwnerFence,
        job_id: &str,
        now_ms: u64,
        lease_ms: u64,
    ) -> Result<(), ScheduleError> {
        let runtime = self
            .store
            .db_runtime()
            .map_err(|error| persistence_error(error))?;
        let job_id = job_id.to_string();
        let owner_id = fence.owner_id.clone();
        let generation = fence.lease_generation;
        let lease_until = now_ms.saturating_add(lease_ms) as i64;
        flatten(runtime.write_blocking(move |conn| {
            let inner = (|| -> Result<(), ScheduleError> {
                let tx = conn.transaction().map_err(persistence_error)?;
                let changed = tx
                    .execute(
                        "UPDATE experiment_campaign_jobs
                            SET lease_expires_at_ms = ?3,
                                heartbeat_at_ms = ?4,
                                updated_at_ms = ?4
                          WHERE job_id = ?1
                            AND owner_id = ?2
                            AND lease_generation = ?5
                            AND state NOT IN (
                                'succeeded','failed_precondition','failed','cancelled','unknown_manual'
                            )",
                        params![job_id, owner_id, lease_until, now_ms as i64, generation],
                    )
                    .map_err(persistence_error)?;
                if changed != 1 {
                    tx.rollback().map_err(persistence_error)?;
                    return Err(store_error(
                        ScheduleErrorCode::LeaseLost,
                        format!("lease generation {generation} for job '{job_id}' is no longer current"),
                    ));
                }
                tx.commit().map_err(persistence_error)?;
                Ok(())
            })();
            Ok(inner)
        }))
    }

    /// Performs one fenced state transition plus its journal entry.
    ///
    /// The current row is re-read and validated inside the transaction, so a
    /// worker whose lease was superseded matches zero rows and gets
    /// `lease_lost` rather than corrupting a newer owner's state.
    pub fn transition(
        &self,
        fence: &OwnerFence,
        request: TransitionRequest<'_>,
    ) -> Result<JobRecord, ScheduleError> {
        validate_transition(request.from, request.to, request.marker, request.run_id)?;
        let runtime = self
            .store
            .db_runtime()
            .map_err(|error| persistence_error(error))?;
        let job_id = request.job_id.to_string();
        let owner_id = fence.owner_id.clone();
        let generation = fence.lease_generation;
        let owned = TransitionValues {
            from: request.from,
            to: request.to,
            marker: request.marker,
            run_id: request.run_id.map(str::to_string),
            session_id: request.session_id.map(str::to_string),
            artifact_dir: request.artifact_dir.map(str::to_string),
            error_code: request.error_code.map(str::to_string),
            journal: request.journal,
            journal_detail: request.journal_detail.map(str::to_string),
            now_ms: request.now_ms,
        };
        flatten(runtime.write_blocking(move |conn| {
            let inner = (|| -> Result<JobRecord, ScheduleError> {
                let tx = conn.transaction().map_err(persistence_error)?;
                let current = load_job(&tx, &job_id)?;
                if current.job.state != owned.from {
                    return Err(store_error(
                        ScheduleErrorCode::InvalidJobTransition,
                        format!(
                            "job '{job_id}' is '{}' but '{}' was expected",
                            current.job.state.as_str(),
                            owned.from.as_str()
                        ),
                    ));
                }
                // The stored marker/run-id pair must already be self-consistent
                // before a new transition is layered on top of it.
                validate_dispatch_invariant(
                    current.job.state,
                    current.job.dispatch_marker,
                    current.job.run_id.as_deref(),
                )?;
                // A terminal transition releases ownership; an intermediate one
                // keeps the owner and its live lease, because the same worker
                // is still responsible for the next stage (INV-8).
                let clears_lease = if owned.to.is_terminal() { 1_i64 } else { 0_i64 };
                let changed = tx
                    .execute(
                        "UPDATE experiment_campaign_jobs
                            SET state = ?3,
                                dispatch_marker = ?4,
                                run_id = ?5,
                                session_id = COALESCE(?6, session_id),
                                artifact_dir = COALESCE(?7, artifact_dir),
                                error_code = COALESCE(?8, error_code),
                                owner_id = CASE WHEN ?12 = 1 THEN NULL ELSE ?2 END,
                                lease_expires_at_ms = CASE
                                    WHEN ?12 = 1 THEN NULL ELSE lease_expires_at_ms END,
                                heartbeat_at_ms = CASE
                                    WHEN ?12 = 1 THEN NULL ELSE heartbeat_at_ms END,
                                updated_at_ms = ?9
                          WHERE job_id = ?1
                            AND owner_id = ?2
                            AND lease_generation = ?10
                            AND state = ?11",
                        params![
                            job_id,
                            owner_id,
                            owned.to.as_str(),
                            owned.marker.as_str(),
                            owned.run_id,
                            owned.session_id,
                            owned.artifact_dir,
                            owned.error_code,
                            owned.now_ms as i64,
                            generation,
                            owned.from.as_str(),
                            clears_lease,
                        ],
                    )
                    .map_err(persistence_error)?;
                if changed != 1 {
                    tx.rollback().map_err(persistence_error)?;
                    return Err(store_error(
                        ScheduleErrorCode::LeaseLost,
                        format!("job '{job_id}' is no longer owned by generation {generation}"),
                    ));
                }
                if let Some(stage) = owned.journal {
                    insert_journal(
                        &tx,
                        &job_id,
                        stage,
                        Some(&owner_id),
                        generation,
                        owned.journal_detail.as_deref(),
                        owned.now_ms,
                    )?;
                }
                let record = load_job(&tx, &job_id)?;
                tx.commit().map_err(persistence_error)?;
                Ok(record)
            })();
            Ok(inner)
        }))
    }

    /// Records a completed saga stage for the owning generation. The journal is
    /// append-only; `last_stage` is the cheap projection used by status reads.
    pub fn record_stage(
        &self,
        fence: &OwnerFence,
        job_id: &str,
        stage: JobSagaStage,
        detail: Option<&str>,
        now_ms: u64,
    ) -> Result<(), ScheduleError> {
        let runtime = self
            .store
            .db_runtime()
            .map_err(|error| persistence_error(error))?;
        let job_id = job_id.to_string();
        let owner_id = fence.owner_id.clone();
        let generation = fence.lease_generation;
        let detail = detail.map(str::to_string);
        flatten(runtime.write_blocking(move |conn| {
            let inner = (|| -> Result<(), ScheduleError> {
                let tx = conn.transaction().map_err(persistence_error)?;
                let changed = tx
                    .execute(
                        "UPDATE experiment_campaign_jobs
                            SET last_stage = ?3, updated_at_ms = ?4
                          WHERE job_id = ?1
                            AND owner_id = ?2
                            AND lease_generation = ?5",
                        params![job_id, owner_id, stage.as_str(), now_ms as i64, generation],
                    )
                    .map_err(persistence_error)?;
                if changed != 1 {
                    tx.rollback().map_err(persistence_error)?;
                    return Err(store_error(
                        ScheduleErrorCode::LeaseLost,
                        format!("job '{job_id}' is no longer owned by generation {generation}"),
                    ));
                }
                insert_journal(
                    &tx,
                    &job_id,
                    stage,
                    Some(&owner_id),
                    generation,
                    detail.as_deref(),
                    now_ms,
                )?;
                tx.commit().map_err(persistence_error)?;
                Ok(())
            })();
            Ok(inner)
        }))
    }

    /// Records the durable dispatch intent. It is the last write before the run
    /// kernel is called and the only artifact that makes an unknown outcome
    /// detectable (INV-5).
    pub fn mark_dispatch_intent(
        &self,
        fence: &OwnerFence,
        job_id: &str,
        now_ms: u64,
    ) -> Result<JobRecord, ScheduleError> {
        self.transition(
            fence,
            TransitionRequest {
                job_id,
                from: JobState::Prepared,
                to: JobState::Dispatching,
                marker: DispatchMarker::IntentRecorded,
                run_id: None,
                session_id: None,
                artifact_dir: None,
                error_code: None,
                journal: Some(JobSagaStage::DispatchIntent),
                journal_detail: None,
                now_ms,
            },
        )
    }

    /// Every non-terminal job in the domain, in claim order.
    pub fn non_terminal_jobs(&self) -> Result<Vec<JobRecord>, ScheduleError> {
        let runtime = self
            .store
            .db_runtime()
            .map_err(|error| persistence_error(error))?;
        flatten(runtime.read_blocking(move |conn| {
            let inner = (|| -> Result<Vec<JobRecord>, ScheduleError> {
                let mut statement = conn
                    .prepare(&format!(
                        "SELECT {JOB_COLUMNS} FROM experiment_campaign_jobs
                      WHERE state NOT IN (
                        'succeeded','failed_precondition','failed','cancelled','unknown_manual'
                      )
                      ORDER BY campaign_id ASC, ordinal ASC"
                    ))
                    .map_err(persistence_error)?;
                let rows = statement
                    .query_map([], map_job_row)
                    .map_err(persistence_error)?;
                let mut jobs = Vec::new();
                for row in rows {
                    jobs.push(row.map_err(persistence_error)?);
                }
                Ok(jobs)
            })();
            Ok(inner)
        }))
    }

    /// Parks a job whose effect cannot be proven absent into `unknown_manual`.
    ///
    /// This is the *only* automatic recovery write that touches a
    /// dispatched job, and it refuses to run unless the pure classifier agrees
    /// the outcome is genuinely unknown and the lease is not live. It never
    /// requeues and never calls the run kernel.
    pub fn park_unknown_manual(
        &self,
        job_id: &str,
        run_terminal: Option<bool>,
        reason: &str,
        now_ms: u64,
    ) -> Result<JobRecord, ScheduleError> {
        let runtime = self
            .store
            .db_runtime()
            .map_err(|error| persistence_error(error))?;
        let job_id = job_id.to_string();
        let reason = reason.to_string();
        flatten(runtime.write_blocking(move |conn| {
            let inner = (|| -> Result<JobRecord, ScheduleError> {
                let tx = conn.transaction().map_err(persistence_error)?;
                let current = load_job(&tx, &job_id)?;
                let decision = classify_restart_recovery(
                    current.job.state,
                    current.job.dispatch_marker,
                    current.job.run_id.as_deref(),
                    run_terminal,
                );
                if decision != RecoveryDecision::UnknownManual {
                    return Err(store_error(
                        ScheduleErrorCode::DispatchUncertain,
                        format!("job '{job_id}' classifies as {decision:?}, not unknown_manual"),
                    ));
                }
                let lease_live = current
                    .lease_expires_at_ms
                    .map(|expires| expires > now_ms)
                    .unwrap_or(false);
                if lease_live {
                    return Err(store_error(
                        ScheduleErrorCode::LeaseConflict,
                        format!("job '{job_id}' still holds a live lease"),
                    ));
                }
                let from_state = current.job.state.as_str();
                let changed = tx
                    .execute(
                        "UPDATE experiment_campaign_jobs
                            SET state = 'unknown_manual',
                                error_code = 'dispatch_uncertain',
                                owner_id = NULL,
                                lease_expires_at_ms = NULL,
                                heartbeat_at_ms = NULL,
                                updated_at_ms = ?3
                          WHERE job_id = ?1
                            AND state = ?2",
                        params![job_id, from_state, now_ms as i64],
                    )
                    .map_err(persistence_error)?;
                if changed != 1 {
                    return Err(store_error(
                        ScheduleErrorCode::LeaseLost,
                        format!("job '{job_id}' changed while it was being parked"),
                    ));
                }
                insert_journal(
                    &tx,
                    &job_id,
                    JobSagaStage::CleanupDone,
                    None,
                    current.job.lease_generation,
                    Some(&reason),
                    now_ms,
                )?;
                let record = load_job(&tx, &job_id)?;
                tx.commit().map_err(persistence_error)?;
                Ok(record)
            })();
            Ok(inner)
        }))
    }

    /// Classifies every non-terminal job for a restart. `run_terminal` is
    /// supplied by the caller for jobs that carry a run id, because only the
    /// workflow authority can observe terminality (INV-3). Nothing is written.
    pub fn classify_restart(
        &self,
        run_terminal: impl Fn(&str) -> Option<bool>,
    ) -> Result<Vec<RecoveryRecord>, ScheduleError> {
        let jobs = self.non_terminal_jobs()?;
        Ok(jobs
            .into_iter()
            .map(|record| {
                let terminal = record.job.run_id.as_deref().and_then(&run_terminal);
                let decision = classify_restart_recovery(
                    record.job.state,
                    record.job.dispatch_marker,
                    record.job.run_id.as_deref(),
                    terminal,
                );
                RecoveryRecord {
                    job_id: record.job.job_id.clone(),
                    state: record.job.state,
                    dispatch_marker: record.job.dispatch_marker,
                    run_id: record.job.run_id.clone(),
                    decision,
                }
            })
            .collect())
    }

    /// The append-only journal of one job, oldest first.
    pub fn journal(
        &self,
        job_id: &str,
    ) -> Result<Vec<(i64, String, Option<String>)>, ScheduleError> {
        let runtime = self
            .store
            .db_runtime()
            .map_err(|error| persistence_error(error))?;
        let job_id = job_id.to_string();
        flatten(runtime.read_blocking(move |conn| {
            let inner = (|| -> Result<Vec<(i64, String, Option<String>)>, ScheduleError> {
                let mut statement = conn
                    .prepare(
                        "SELECT journal_id, stage, detail_json
                       FROM experiment_job_journal
                      WHERE job_id = ?1
                      ORDER BY journal_id ASC",
                    )
                    .map_err(persistence_error)?;
                let rows = statement
                    .query_map(params![job_id], |row| {
                        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                    })
                    .map_err(persistence_error)?;
                let mut entries = Vec::new();
                for row in rows {
                    entries.push(row.map_err(persistence_error)?);
                }
                Ok(entries)
            })();
            Ok(inner)
        }))
    }
}

/// The values of one fenced transition, owned so they can cross into the
/// writer closure.
struct TransitionValues {
    from: JobState,
    to: JobState,
    marker: DispatchMarker,
    run_id: Option<String>,
    session_id: Option<String>,
    artifact_dir: Option<String>,
    error_code: Option<String>,
    journal: Option<JobSagaStage>,
    journal_detail: Option<String>,
    now_ms: u64,
}

/// A fenced state transition request.
pub struct TransitionRequest<'a> {
    pub job_id: &'a str,
    pub from: JobState,
    pub to: JobState,
    pub marker: DispatchMarker,
    pub run_id: Option<&'a str>,
    pub session_id: Option<&'a str>,
    pub artifact_dir: Option<&'a str>,
    pub error_code: Option<&'a str>,
    /// The saga stage this transition completes, if any.
    pub journal: Option<JobSagaStage>,
    pub journal_detail: Option<&'a str>,
    pub now_ms: u64,
}

// ---------------------------------------------------------------------------
// Row helpers
// ---------------------------------------------------------------------------

/// Serializes the single fixture ref the schedule bound. The store keeps the
/// values as columns rather than a JSON blob so a job row is self-describing.
fn fixture_suite(request: &CampaignScheduleRequestV1) -> Result<String, ScheduleError> {
    Ok(fixture_ref(request)?.suite.clone())
}

fn fixture_dataset_id(request: &CampaignScheduleRequestV1) -> Result<String, ScheduleError> {
    Ok(fixture_ref(request)?.dataset_id.clone())
}

fn fixture_dataset_version(request: &CampaignScheduleRequestV1) -> Result<i64, ScheduleError> {
    Ok(fixture_ref(request)?.dataset_version as i64)
}

fn fixture_split(request: &CampaignScheduleRequestV1) -> Result<String, ScheduleError> {
    Ok(fixture_ref(request)?.split.clone())
}

fn fixture_manifest_digest(request: &CampaignScheduleRequestV1) -> Result<String, ScheduleError> {
    Ok(fixture_ref(request)?.manifest_digest.clone())
}

fn fixture_task_digest(request: &CampaignScheduleRequestV1) -> Result<String, ScheduleError> {
    Ok(fixture_ref(request)?.task_digest.clone())
}

fn fixture_instruction_hash(request: &CampaignScheduleRequestV1) -> Result<String, ScheduleError> {
    Ok(fixture_ref(request)?.instruction_hash.clone())
}

fn fixture_ref(request: &CampaignScheduleRequestV1) -> Result<&FixtureTaskRefV1, ScheduleError> {
    request.fixture_refs.first().ok_or_else(|| {
        store_error(
            ScheduleErrorCode::InvalidFixture,
            "schedule request carries no fixture ref",
        )
    })
}

fn map_job_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<JobRecord> {
    let state_raw: String = row.get(13)?;
    let marker_raw: String = row.get(14)?;
    let state = JobState::parse(&state_raw).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            13,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown job state '{state_raw}'"),
            )),
        )
    })?;
    let dispatch_marker = DispatchMarker::parse(&marker_raw).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            14,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown dispatch marker '{marker_raw}'"),
            )),
        )
    })?;
    Ok(JobRecord {
        job: CampaignJobV1 {
            schema_version: CAMPAIGN_JOB_V1.to_string(),
            job_id: row.get(0)?,
            campaign_id: row.get(1)?,
            ordinal: row.get::<_, i64>(2)? as u32,
            candidate_key: row.get(3)?,
            task_id: row.get(4)?,
            suite: row.get(5)?,
            dataset_id: row.get(6)?,
            dataset_version: row.get::<_, i64>(7)? as u32,
            split: row.get(8)?,
            manifest_digest: row.get(9)?,
            task_digest: row.get(10)?,
            instruction_hash: row.get(11)?,
            execution_profile_ref: row.get(12)?,
            state,
            dispatch_marker,
            run_id: row.get(15)?,
            attempt: row.get::<_, i64>(17)? as u32,
            lease_generation: row.get(19)?,
            last_stage: row.get(22)?,
            error_code: row.get(23)?,
        },
        session_id: row.get(16)?,
        artifact_dir: row.get(24)?,
        owner_id: row.get(18)?,
        lease_expires_at_ms: row.get::<_, Option<i64>>(20)?.map(|value| value as u64),
        heartbeat_at_ms: row.get::<_, Option<i64>>(21)?.map(|value| value as u64),
    })
}

const JOB_COLUMNS: &str = "job_id, campaign_id, ordinal, candidate_key, task_id, suite,
        dataset_id, dataset_version, split, manifest_digest, task_digest,
        instruction_hash, execution_profile_ref, state, dispatch_marker,
        run_id, session_id, attempt, owner_id, lease_generation,
        lease_expires_at_ms, heartbeat_at_ms, last_stage, error_code, artifact_dir";

fn load_job(conn: &rusqlite::Connection, job_id: &str) -> Result<JobRecord, ScheduleError> {
    conn.query_row(
        &format!("SELECT {JOB_COLUMNS} FROM experiment_campaign_jobs WHERE job_id = ?1"),
        params![job_id],
        map_job_row,
    )
    .optional()
    .map_err(persistence_error)?
    .ok_or_else(|| {
        store_error(
            ScheduleErrorCode::UnknownJob,
            format!("no durable job '{job_id}' in this domain"),
        )
    })
}

fn load_campaign(
    conn: &rusqlite::Connection,
    campaign_id: &str,
) -> Result<CampaignRecord, ScheduleError> {
    let row = conn
        .query_row(
            "SELECT campaign_id, campaign_key, plan_hash, schedule_hash, status,
                    execution_profile_ref, plan_json, fixture_refs_json, bundle_refs_json
               FROM experiment_campaign_schedules
              WHERE campaign_id = ?1",
            params![campaign_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                ))
            },
        )
        .optional()
        .map_err(persistence_error)?
        .ok_or_else(|| {
            store_error(
                ScheduleErrorCode::UnknownCampaign,
                format!("no durable campaign '{campaign_id}' in this domain"),
            )
        })?;

    let status = CampaignStatus::parse(&row.4).ok_or_else(|| {
        store_error(
            ScheduleErrorCode::PersistenceFailure,
            "durable campaign has an unrecognized status",
        )
    })?;
    let plan: CampaignPlanV1 = serde_json::from_str(&row.6).map_err(|error| {
        store_error(
            ScheduleErrorCode::PersistenceFailure,
            format!("durable plan is not a valid campaign plan: {error}"),
        )
    })?;
    let fixture_refs: Vec<FixtureTaskRefV1> = serde_json::from_str(&row.7).map_err(|error| {
        store_error(
            ScheduleErrorCode::PersistenceFailure,
            format!("durable fixture refs are malformed: {error}"),
        )
    })?;
    let bundle_refs: Vec<String> = serde_json::from_str(&row.8).map_err(|error| {
        store_error(
            ScheduleErrorCode::PersistenceFailure,
            format!("durable bundle refs are malformed: {error}"),
        )
    })?;

    let mut statement = conn
        .prepare("SELECT job_id FROM experiment_campaign_jobs WHERE campaign_id = ?1 ORDER BY ordinal ASC")
        .map_err(persistence_error)?;
    let job_ids: Vec<String> = statement
        .query_map(params![campaign_id], |row| row.get::<_, String>(0))
        .map_err(persistence_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(persistence_error)?;

    Ok(CampaignRecord {
        campaign_id: row.0,
        campaign_key: row.1,
        plan_hash: row.2,
        schedule_hash: row.3,
        status,
        execution_profile_ref: row.5,
        plan,
        fixture_refs,
        bundle_refs,
        job_ids,
    })
}

fn accepted_projection(record: &CampaignRecord) -> CampaignScheduleAcceptedV1 {
    CampaignScheduleAcceptedV1::new(
        record.campaign_id.clone(),
        record.campaign_key.clone(),
        record.plan_hash.clone(),
        record.schedule_hash.clone(),
        record.execution_profile_ref.clone(),
        record.job_ids.clone(),
    )
}

fn insert_journal(
    tx: &Transaction<'_>,
    job_id: &str,
    stage: JobSagaStage,
    owner_id: Option<&str>,
    lease_generation: i64,
    detail: Option<&str>,
    now_ms: u64,
) -> Result<(), ScheduleError> {
    tx.execute(
        "INSERT INTO experiment_job_journal (
            job_id, stage, owner_id, lease_generation, detail_json, created_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            job_id,
            stage.as_str(),
            owner_id,
            lease_generation,
            detail,
            now_ms as i64
        ],
    )
    .map_err(persistence_error)?;
    Ok(())
}

/// Verifies that a durable fixture ref still resolves against the pinned
/// checked-in catalog. Called before dispatch so a drifted catalog fails as a
/// pre-dispatch precondition instead of running a different fixture.
pub fn verify_job_fixture(job: &CampaignJobV1) -> Result<(), ScheduleError> {
    let reference = FixtureTaskRefV1 {
        suite: job.suite.clone(),
        dataset_id: job.dataset_id.clone(),
        dataset_version: job.dataset_version,
        split: job.split.clone(),
        task_id: job.task_id.clone(),
        manifest_digest: job.manifest_digest.clone(),
        task_digest: job.task_digest.clone(),
        instruction_hash: job.instruction_hash.clone(),
    };
    resolve_task_ref(&reference).map(|_| ()).map_err(|error| {
        store_error(
            ScheduleErrorCode::FixtureDigestMismatch,
            format!("{}: {}", error.code, error.message),
        )
    })
}

/// Rejects a durable row whose stored digests are malformed. Cheap guard used
/// by the scheduler before it trusts a persisted job.
pub fn job_digests_are_well_formed(job: &CampaignJobV1) -> bool {
    is_sha256_hex(&job.manifest_digest)
        && is_sha256_hex(&job.task_digest)
        && is_sha256_hex(&job.instruction_hash)
}

/// Validates that a persisted job row satisfies the FSM invariants. A row that
/// does not is never executed.
pub fn validate_job_row(job: &CampaignJobV1) -> Result<(), ScheduleError> {
    validate_dispatch_invariant(job.state, job.dispatch_marker, job.run_id.as_deref())
}

/// Convenience wrapper around the transition table for callers that already
/// hold a from/to pair.
pub fn job_transition_allowed(from: JobState, to: JobState) -> bool {
    crate::workflow::react::experiment_schedule::types::transition_allowed(from, to)
}

/// Re-exported so the scheduler never re-implements the transition guard.
pub fn validate_job_transition(
    from: JobState,
    to: JobState,
    marker: DispatchMarker,
    run_id: Option<&str>,
) -> Result<(), ScheduleError> {
    validate_transition(from, to, marker, run_id)
}

/// Re-exported so recovery branches on exactly one classifier.
pub fn classify_recovery(
    state: JobState,
    marker: DispatchMarker,
    run_id: Option<&str>,
    run_terminal: Option<bool>,
) -> RecoveryDecision {
    classify_restart_recovery(state, marker, run_id, run_terminal)
}

/// A fenced owner reference used by the store's mutating operations.
pub fn owner_fence(owner_id: &str, lease_generation: i64) -> OwnerFence {
    OwnerFence::new(owner_id, lease_generation)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::react::experiment_schedule::types::{
        parse_and_validate_campaign_schedule_request, CAMPAIGN_SCHEDULE_V1,
    };
    use serde_json::Value;
    use tempfile::tempdir;

    const T0: u64 = 1_700_000_000_000;
    const LEASE_MS: u64 = 30_000;

    fn harness() -> (ExperimentScheduleStore, tempfile::TempDir) {
        let directory = tempdir().expect("tempdir");
        let store =
            Arc::new(MainStore::new(directory.path().join("schedule.db")).expect("main store"));
        // The store is only usable inside a marked domain; the migration gives
        // every database the tables, and a headless test marks the domain.
        store
            .db_runtime()
            .expect("runtime")
            .write_blocking(|conn| {
                conn.execute(
                    "INSERT INTO experiment_domain (
                        domain_id, domain_kind, marker_schema_version, singleton, created_at_ms
                     ) VALUES ('domain-test', 'experiment.v1', 'experiment_domain_marker.v1', 1, 0)",
                    [],
                )?;
                Ok(())
            })
            .expect("mark domain");
        (ExperimentScheduleStore::new(store), directory)
    }

    fn request_value(campaign_key: &str, candidates: Value) -> Value {
        let resolved = crate::workflow::react::experiment_schedule::fixture::resolve_task(
            "chatspeed-smoke",
            "smoke_reply_ok",
        )
        .expect("fixture");
        serde_json::json!({
            "schema_version": CAMPAIGN_SCHEDULE_V1,
            "plan": {
                "schema_version": "campaign_plan.v1",
                "campaign_key": campaign_key,
                "stage": "stage_0_manual",
                "agent_id": "agent-1",
                "suite": "chatspeed-smoke",
                "task": "smoke_reply_ok",
                "concurrency": 1,
                "budget": {
                    "money_mode": { "mode": "token_resource_only" },
                    "caps": { "input_tokens": 1024, "output_tokens": 1024 },
                    "required_dimensions": [],
                    "max_attempts": 1
                },
                "candidates": candidates
            },
            "fixture_refs": [serde_json::to_value(resolved.task_ref()).expect("ref")],
            "execution_profile_ref": "smoke-local",
            "bundle_refs": []
        })
    }

    fn two_candidate_plan() -> Value {
        serde_json::json!([
            { "candidate_key": "baseline", "kind": "baseline" },
            {
                "candidate_key": "cand-a",
                "kind": "candidate",
                "mutable_surface": ["agent_prompt_ref"],
                "agent_prompt_ref": "smoke-terse",
                "prompt_hash": "0".repeat(64)
            }
        ])
    }

    fn request(campaign_key: &str, candidates: Value) -> CampaignScheduleRequestV1 {
        parse_and_validate_campaign_schedule_request(&request_value(campaign_key, candidates))
            .expect("valid request")
    }

    fn schedule(store: &ExperimentScheduleStore, campaign_key: &str, key: &str) -> ScheduleOutcome {
        store
            .schedule_campaign(&request(campaign_key, two_candidate_plan()), key, T0)
            .expect("schedule")
    }

    #[test]
    fn scheduling_creates_the_ordered_jobs_in_one_transaction() {
        let (store, _dir) = harness();
        let outcome = schedule(&store, "p2gh-a", "idem-1");
        assert!(outcome.created);
        assert_eq!(outcome.accepted.concurrency, SCHEDULE_CONCURRENCY);
        assert_eq!(outcome.accepted.job_ids.len(), 2);

        let campaign = store
            .get_campaign(&outcome.accepted.campaign_id)
            .expect("campaign");
        assert_eq!(campaign.status, CampaignStatus::Active);
        assert_eq!(campaign.job_ids, outcome.accepted.job_ids);

        let jobs = store.list_jobs(&campaign.campaign_id).expect("jobs");
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].job.candidate_key, "baseline");
        assert_eq!(jobs[0].job.ordinal, 0);
        assert_eq!(jobs[1].job.candidate_key, "cand-a");
        assert_eq!(jobs[1].job.ordinal, 1);
        for job in &jobs {
            assert_eq!(job.job.state, JobState::Queued);
            assert_eq!(job.job.dispatch_marker, DispatchMarker::NotDispatched);
            assert_eq!(job.job.attempt, 0);
            assert_eq!(job.job.lease_generation, 0);
            assert_eq!(job.job.run_id, None);
            assert!(job_digests_are_well_formed(&job.job));
            validate_job_row(&job.job).expect("row invariants");
            verify_job_fixture(&job.job).expect("fixture still resolves");
        }
    }

    /// The durable row must never carry the instruction body (INV-6).
    #[test]
    fn durable_rows_never_store_the_instruction() {
        let (store, directory) = harness();
        let outcome = schedule(&store, "p2gh-a", "idem-1");
        let runtime = store.main_store().db_runtime().expect("runtime");
        let (plan_json, fixture_json): (String, String) = runtime
            .read_blocking(move |conn| {
                conn.query_row(
                    "SELECT plan_json, fixture_refs_json FROM experiment_campaign_schedules
                      WHERE campaign_id = ?1",
                    params![outcome.accepted.campaign_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(crate::db::StoreError::from)
            })
            .expect("read");
        for payload in [&plan_json, &fixture_json] {
            assert!(!payload.contains("Reply with exactly"));
            assert!(!payload.contains("instruction\":"));
        }
        // And nothing anywhere in the database file carries the instruction.
        let bytes = std::fs::read(directory.path().join("schedule.db")).expect("db bytes");
        let needle = b"Reply with exactly: OK";
        assert!(
            !bytes.windows(needle.len()).any(|window| window == needle),
            "the fixture instruction must never be persisted"
        );
    }

    #[test]
    fn replaying_the_same_key_and_body_is_idempotent() {
        let (store, _dir) = harness();
        let first = schedule(&store, "p2gh-a", "idem-1");
        let second = schedule(&store, "p2gh-a", "idem-1");
        assert!(first.created);
        assert!(!second.created);
        assert_eq!(first.accepted.job_ids, second.accepted.job_ids);
        assert_eq!(first.accepted.schedule_hash, second.accepted.schedule_hash);
        assert_eq!(
            store.list_jobs(&first.accepted.campaign_id).unwrap().len(),
            2
        );
    }

    #[test]
    fn replaying_a_key_with_a_different_body_is_rejected() {
        let (store, _dir) = harness();
        schedule(&store, "p2gh-a", "idem-1");
        let error = store
            .schedule_campaign(&request("p2gh-b", two_candidate_plan()), "idem-1", T0)
            .expect_err("conflict");
        assert_eq!(error.code, ScheduleErrorCode::IdempotencyConflict);
    }

    #[test]
    fn the_same_campaign_key_cannot_bind_two_plans() {
        let (store, _dir) = harness();
        schedule(&store, "p2gh-a", "idem-1");
        let error = store
            .schedule_campaign(
                &request(
                    "p2gh-a",
                    serde_json::json!([
                        { "candidate_key": "baseline", "kind": "baseline" },
                        { "candidate_key": "cand-b", "kind": "candidate",
                          "mutable_surface": ["agent_prompt_ref"],
                          "agent_prompt_ref": "smoke-terse",
                          "prompt_hash": "0".repeat(64) }
                    ]),
                ),
                "idem-2",
                T0,
            )
            .expect_err("key conflict");
        assert_eq!(error.code, ScheduleErrorCode::IdempotencyConflict);
    }

    #[test]
    fn only_one_worker_wins_a_claim_and_the_generation_is_fenced() {
        let (store, _dir) = harness();
        let outcome = schedule(&store, "p2gh-a", "idem-1");
        let first = match store
            .claim_next_job("worker-a", T0, LEASE_MS)
            .expect("claim")
        {
            ClaimOutcome::Claimed(record) => *record,
            ClaimOutcome::Idle => panic!("expected a claim"),
        };
        assert_eq!(first.job.candidate_key, "baseline");
        assert_eq!(first.job.state, JobState::Preparing);
        assert_eq!(first.job.lease_generation, 1);
        assert_eq!(first.job.attempt, 1);
        assert_eq!(first.owner_id.as_deref(), Some("worker-a"));

        // A second worker cannot take the same job while the lease is live.
        let second = match store
            .claim_next_job("worker-b", T0 + 1, LEASE_MS)
            .expect("claim")
        {
            ClaimOutcome::Claimed(record) => *record,
            ClaimOutcome::Idle => panic!("second job is claimable"),
        };
        assert_eq!(second.job.candidate_key, "cand-a");
        assert_eq!(second.job.lease_generation, 1);

        // Nothing else is claimable while both leases are live.
        assert!(matches!(
            store
                .claim_next_job("worker-c", T0 + 2, LEASE_MS)
                .expect("claim"),
            ClaimOutcome::Idle
        ));

        // The first worker's lease is renewed past the second's expiry, so only
        // the second (baseline is never adopted: it is already renewed) is
        // adoptable once its own lease expires.
        let fence_a = OwnerFence::new("worker-a", first.job.lease_generation);
        store
            .heartbeat(&fence_a, &first.job.job_id, T0 + 10_000, LEASE_MS)
            .expect("renew");
        let adopted = match store
            .claim_next_job("worker-d", T0 + LEASE_MS + 1, LEASE_MS)
            .expect("claim")
        {
            ClaimOutcome::Claimed(record) => *record,
            ClaimOutcome::Idle => panic!("expired lease must be adoptable"),
        };
        assert_eq!(adopted.job.job_id, second.job.job_id);
        assert_eq!(adopted.job.lease_generation, 2);
        assert_eq!(adopted.job.attempt, 2);
        let journal = store.journal(&adopted.job.job_id).expect("journal");
        assert_eq!(journal.len(), 1);
        assert_eq!(journal[0].1, JobSagaStage::WorkspaceAcquired.as_str());
        assert_eq!(journal[0].2.as_deref(), Some("lease_adopted"));
        let _ = outcome;
    }

    #[test]
    fn a_superseded_worker_cannot_heartbeat_or_transition() {
        let (store, _dir) = harness();
        schedule(&store, "p2gh-a", "idem-1");
        let first = match store.claim_next_job("worker-a", T0, LEASE_MS).unwrap() {
            ClaimOutcome::Claimed(record) => *record,
            ClaimOutcome::Idle => panic!("claim"),
        };
        let stale = OwnerFence::new("worker-a", first.job.lease_generation);
        // A newer worker adopts the job after the lease expires.
        let adopted = store
            .adopt_recoverable_job(&first.job.job_id, "worker-b", T0 + LEASE_MS + 1, LEASE_MS)
            .expect("adopt");
        assert_eq!(adopted.job.lease_generation, 2);

        let error = store
            .heartbeat(&stale, &first.job.job_id, T0 + LEASE_MS + 2, LEASE_MS)
            .expect_err("stale heartbeat");
        assert_eq!(error.code, ScheduleErrorCode::LeaseLost);
        let error = store
            .record_stage(
                &stale,
                &first.job.job_id,
                JobSagaStage::BundleStaged,
                None,
                T0 + LEASE_MS + 2,
            )
            .expect_err("stale stage");
        assert_eq!(error.code, ScheduleErrorCode::LeaseLost);
        let error = store
            .transition(
                &stale,
                TransitionRequest {
                    job_id: &first.job.job_id,
                    from: JobState::Preparing,
                    to: JobState::Prepared,
                    marker: DispatchMarker::NotDispatched,
                    run_id: None,
                    session_id: None,
                    artifact_dir: None,
                    error_code: None,
                    journal: Some(JobSagaStage::EnvironmentReady),
                    journal_detail: None,
                    now_ms: T0 + LEASE_MS + 2,
                },
            )
            .expect_err("stale transition");
        assert_eq!(error.code, ScheduleErrorCode::LeaseLost);

        // A live foreign worker still cannot renew a lease it does not own.
        let error = store
            .heartbeat(
                &OwnerFence::new("worker-c", adopted.job.lease_generation),
                &first.job.job_id,
                T0 + LEASE_MS + 3,
                LEASE_MS,
            )
            .expect_err("wrong owner");
        assert_eq!(error.code, ScheduleErrorCode::LeaseLost);
    }

    #[test]
    fn a_dispatch_intent_is_never_removed_by_automatic_recovery() {
        let (store, _dir) = harness();
        schedule(&store, "p2gh-a", "idem-1");
        let claimed = match store.claim_next_job("worker-a", T0, LEASE_MS).unwrap() {
            ClaimOutcome::Claimed(record) => *record,
            ClaimOutcome::Idle => panic!("claim"),
        };
        let fence = OwnerFence::new("worker-a", claimed.job.lease_generation);
        // preparing -> prepared -> dispatching(intent_recorded)
        store
            .transition(
                &fence,
                TransitionRequest {
                    job_id: &claimed.job.job_id,
                    from: JobState::Preparing,
                    to: JobState::Prepared,
                    marker: DispatchMarker::NotDispatched,
                    run_id: None,
                    session_id: None,
                    artifact_dir: None,
                    error_code: None,
                    journal: Some(JobSagaStage::EnvironmentReady),
                    journal_detail: None,
                    now_ms: T0 + 1,
                },
            )
            .expect("prepared");
        store
            .mark_dispatch_intent(&fence, &claimed.job.job_id, T0 + 2)
            .expect("intent");

        let job = store.get_job(&claimed.job.job_id).expect("job");
        assert_eq!(job.job.state, JobState::Dispatching);
        assert_eq!(job.job.dispatch_marker, DispatchMarker::IntentRecorded);
        assert_eq!(job.job.run_id, None);

        // The classifier parks it: the effect cannot be proven absent.
        let classified = store.classify_restart(|_| None).expect("classify");
        let entry = classified
            .iter()
            .find(|record| record.job_id == claimed.job.job_id)
            .expect("classified");
        assert_eq!(entry.decision, RecoveryDecision::UnknownManual);

        // It is never claimable again: every further claim can only pick up the
        // campaign's other, still-queued job.
        let mut claimed_ids = Vec::new();
        loop {
            match store
                .claim_next_job("worker-z", T0 + 10, LEASE_MS)
                .expect("claim")
            {
                ClaimOutcome::Claimed(record) => claimed_ids.push(record.job.job_id.clone()),
                ClaimOutcome::Idle => break,
            }
            assert!(claimed_ids.len() < 8, "claim loop did not converge");
        }
        assert!(!claimed_ids.contains(&claimed.job.job_id));
        // Adoption refuses it even after the lease expires.
        let error = store
            .adopt_recoverable_job(&claimed.job.job_id, "worker-z", T0 + LEASE_MS + 1, LEASE_MS)
            .expect_err("never adopted");
        assert_eq!(error.code, ScheduleErrorCode::DispatchUncertain);

        // A live lease also blocks parking.
        let error = store
            .park_unknown_manual(&claimed.job.job_id, None, "restart", T0 + 3)
            .expect_err("live lease");
        assert_eq!(error.code, ScheduleErrorCode::LeaseConflict);

        let parked = store
            .park_unknown_manual(&claimed.job.job_id, None, "restart", T0 + LEASE_MS + 2)
            .expect("park");
        assert_eq!(parked.job.state, JobState::UnknownManual);
        assert_eq!(parked.job.dispatch_marker, DispatchMarker::IntentRecorded);
        assert_eq!(parked.job.run_id, None);
        assert_eq!(parked.job.error_code.as_deref(), Some("dispatch_uncertain"));

        // Parking is terminal: a second sweep does nothing.
        let classified = store.classify_restart(|_| None).expect("classify");
        assert!(classified
            .iter()
            .all(|record| record.job_id != claimed.job.job_id));
    }

    #[test]
    fn parking_refuses_work_that_is_still_recoverable() {
        let (store, _dir) = harness();
        schedule(&store, "p2gh-a", "idem-1");
        let claimed = match store.claim_next_job("worker-a", T0, LEASE_MS).unwrap() {
            ClaimOutcome::Claimed(record) => *record,
            ClaimOutcome::Idle => panic!("claim"),
        };
        let error = store
            .park_unknown_manual(&claimed.job.job_id, None, "restart", T0 + LEASE_MS + 1)
            .expect_err("must not park recoverable work");
        assert_eq!(error.code, ScheduleErrorCode::DispatchUncertain);
        assert_eq!(
            store.get_job(&claimed.job.job_id).unwrap().job.state,
            JobState::Preparing
        );
    }

    #[test]
    fn a_confirmed_run_is_never_restarted_and_only_reconciled() {
        let (store, _dir) = harness();
        schedule(&store, "p2gh-a", "idem-1");
        let claimed = match store.claim_next_job("worker-a", T0, LEASE_MS).unwrap() {
            ClaimOutcome::Claimed(record) => *record,
            ClaimOutcome::Idle => panic!("claim"),
        };
        let fence = OwnerFence::new("worker-a", claimed.job.lease_generation);
        store
            .transition(
                &fence,
                TransitionRequest {
                    job_id: &claimed.job.job_id,
                    from: JobState::Preparing,
                    to: JobState::Prepared,
                    marker: DispatchMarker::NotDispatched,
                    run_id: None,
                    session_id: None,
                    artifact_dir: None,
                    error_code: None,
                    journal: Some(JobSagaStage::EnvironmentReady),
                    journal_detail: None,
                    now_ms: T0 + 1,
                },
            )
            .expect("prepared");
        store
            .mark_dispatch_intent(&fence, &claimed.job.job_id, T0 + 2)
            .expect("intent");
        let intent_fence = OwnerFence::new("worker-a", claimed.job.lease_generation);
        // The dispatching job still holds the same generation until it confirms.
        store
            .transition(
                &intent_fence,
                TransitionRequest {
                    job_id: &claimed.job.job_id,
                    from: JobState::Dispatching,
                    to: JobState::Running,
                    marker: DispatchMarker::Confirmed,
                    run_id: Some("run-1"),
                    session_id: Some("session-1"),
                    artifact_dir: Some("/artifacts/run-1"),
                    error_code: None,
                    journal: Some(JobSagaStage::WorkflowStarted),
                    journal_detail: None,
                    now_ms: T0 + 3,
                },
            )
            .expect("confirmed");
        let job = store.get_job(&claimed.job.job_id).expect("job");
        assert_eq!(job.job.state, JobState::Running);
        assert_eq!(job.job.run_id.as_deref(), Some("run-1"));
        assert_eq!(job.session_id.as_deref(), Some("session-1"));

        // A terminal run may be collected; a non-terminal one is parked.
        let classified = store
            .classify_restart(|run_id| (run_id == "run-1").then_some(true))
            .expect("classify");
        let entry = classified
            .iter()
            .find(|record| record.job_id == claimed.job.job_id)
            .expect("classified");
        assert_eq!(entry.decision, RecoveryDecision::Collect);

        let classified = store
            .classify_restart(|run_id| (run_id == "run-1").then_some(false))
            .expect("classify");
        let entry = classified
            .iter()
            .find(|record| record.job_id == claimed.job.job_id)
            .expect("classified");
        assert_eq!(entry.decision, RecoveryDecision::UnknownManual);

        // The run kernel can never be re-entered: the confirmed job is never
        // claimable again, whatever the other candidate's state is.
        let mut claimed_ids = Vec::new();
        loop {
            match store
                .claim_next_job("worker-b", T0 + 4, LEASE_MS)
                .expect("claim")
            {
                ClaimOutcome::Claimed(record) => claimed_ids.push(record.job.job_id.clone()),
                ClaimOutcome::Idle => break,
            }
            assert!(claimed_ids.len() < 8, "claim loop did not converge");
        }
        assert!(!claimed_ids.contains(&claimed.job.job_id));
    }

    #[test]
    fn cancel_only_touches_provably_undispatched_jobs() {
        let (store, _dir) = harness();
        let outcome = schedule(&store, "p2gh-a", "idem-1");
        let first = match store.claim_next_job("worker-a", T0, LEASE_MS).unwrap() {
            ClaimOutcome::Claimed(record) => *record,
            ClaimOutcome::Idle => panic!("claim"),
        };
        let cancelled = store
            .cancel_pre_dispatch_jobs(&outcome.accepted.campaign_id, T0 + 1)
            .expect("cancel");
        assert_eq!(cancelled.len(), 2);
        let jobs = store.list_jobs(&outcome.accepted.campaign_id).unwrap();
        assert!(jobs.iter().all(|job| job.job.state == JobState::Cancelled));
        assert!(jobs
            .iter()
            .all(|job| job.job.dispatch_marker == DispatchMarker::NotDispatched));
        let _ = first;
    }

    #[test]
    fn closing_a_campaign_stops_new_claims() {
        let (store, _dir) = harness();
        let outcome = schedule(&store, "p2gh-a", "idem-1");
        let closed = store
            .set_campaign_status(
                &outcome.accepted.campaign_id,
                CampaignStatus::Closed,
                T0 + 1,
            )
            .expect("close");
        assert_eq!(closed.status, CampaignStatus::Closed);
        assert!(matches!(
            store.claim_next_job("worker-a", T0 + 2, LEASE_MS).unwrap(),
            ClaimOutcome::Idle
        ));
        // Closing is idempotent; re-opening is not part of the contract.
        store
            .set_campaign_status(
                &outcome.accepted.campaign_id,
                CampaignStatus::Closed,
                T0 + 3,
            )
            .expect("idempotent close");
        let error = store
            .set_campaign_status(
                &outcome.accepted.campaign_id,
                CampaignStatus::Active,
                T0 + 4,
            )
            .expect_err("cannot reopen");
        assert_eq!(error.code, ScheduleErrorCode::CampaignNotActive);
    }

    #[test]
    fn unknown_campaign_and_job_ids_fail_closed() {
        let (store, _dir) = harness();
        let error = store.get_campaign("camp-unknown").expect_err("campaign");
        assert_eq!(error.code, ScheduleErrorCode::UnknownCampaign);
        let error = store.get_job("job-unknown").expect_err("job");
        assert_eq!(error.code, ScheduleErrorCode::UnknownJob);
    }

    #[test]
    fn an_invalid_transition_is_rejected_before_any_write() {
        let (store, _dir) = harness();
        schedule(&store, "p2gh-a", "idem-1");
        let claimed = match store.claim_next_job("worker-a", T0, LEASE_MS).unwrap() {
            ClaimOutcome::Claimed(record) => *record,
            ClaimOutcome::Idle => panic!("claim"),
        };
        let fence = OwnerFence::new("worker-a", claimed.job.lease_generation);
        // preparing -> running skips the FSM.
        let error = store
            .transition(
                &fence,
                TransitionRequest {
                    job_id: &claimed.job.job_id,
                    from: JobState::Preparing,
                    to: JobState::Running,
                    marker: DispatchMarker::Confirmed,
                    run_id: Some("run-1"),
                    session_id: None,
                    artifact_dir: None,
                    error_code: None,
                    journal: None,
                    journal_detail: None,
                    now_ms: T0 + 1,
                },
            )
            .expect_err("illegal edge");
        assert_eq!(error.code, ScheduleErrorCode::InvalidJobTransition);
        assert_eq!(
            store.get_job(&claimed.job.job_id).unwrap().job.state,
            JobState::Preparing
        );
    }

    #[test]
    fn a_desktop_style_database_gains_only_empty_tables() {
        let directory = tempdir().expect("tempdir");
        let store = MainStore::new(directory.path().join("desktop.db")).expect("main store");
        let runtime = store.db_runtime().expect("runtime");
        let (version, markers, jobs): (i64, i64, i64) = runtime
            .read_blocking(|conn| {
                let version: i64 =
                    conn.query_row("SELECT MAX(version) FROM db_version", [], |row| row.get(0))?;
                let markers: i64 =
                    conn.query_row("SELECT COUNT(1) FROM experiment_domain", [], |row| {
                        row.get(0)
                    })?;
                let jobs: i64 =
                    conn.query_row("SELECT COUNT(1) FROM experiment_campaign_jobs", [], |row| {
                        row.get(0)
                    })?;
                Ok((version, markers, jobs))
            })
            .expect("read");
        // Phase 2I raises the latest schema to v20; the point of this test is
        // that a desktop database merely gains the empty experiment tables.
        assert_eq!(version, 20);
        assert_eq!(markers, 0, "a desktop database is never auto-marked");
        assert_eq!(jobs, 0);
    }
}
