//! The bounded, recoverable campaign scheduler (Phase 2H).
//!
//! The scheduler is the *only* component that turns a durable, queued job into
//! a run, and it is careful in one specific way: it never invokes the run kernel
//! unless it can prove, from the durable row, that no effect has happened yet.
//!
//! A tick is bounded and does at most this:
//!
//! ```text
//! recover   re-classify every non-terminal job from durable state + the run
//!           authority, parking anything whose effect cannot be proven absent
//! claim     CAS-claim the next queued (or lease-expired) job, minting a fresh
//!           lease generation
//! prepare   resolve the execution owner server-side, acquire the owned
//!           workspace, stage + verify the bundles and mint the capability lease
//! dispatch  write the transactional dispatch intent, then call the run kernel
//!           exactly once and record the confirmed run id
//! collect   wait for the authoritative terminal state, publish the output patch
//! cleanup   release the capability lease, remove the owned environment
//! ```
//!
//! Rules that make restart safe (AC-2, INV-1/INV-4/INV-5/INV-8):
//!
//! - Every state change is fenced by the job's `(owner, lease_generation)`, so a
//!   superseded worker's transition is rejected by the store rather than applied.
//! - Nothing is dispatched before the intent row is durable: a crash between the
//!   intent and the kernel call leaves a job that the classifier parks as
//!   `unknown_manual` instead of re-running it.
//! - An unresolved owner is a *pre-dispatch* failure: the job goes terminal as
//!   `failed_precondition` and the kernel is never called. There is no host
//!   fallback.
//! - A job whose run is still running is left durable; the next tick re-observes
//!   it instead of blocking. No tick holds a job open indefinitely.

use crate::db::experiment_schedule::{
    CampaignRecord, ClaimOutcome, ExperimentScheduleStore, JobRecord,
};
use crate::workflow::react::campaign::CampaignPlanV1;
use crate::workflow::react::experiment_owner::bundle::{self, BundleRegistry, StagedBundle};
use crate::workflow::react::experiment_owner::capabilities::{
    PreparedCapabilityLease, PreparedCapabilityLeaseSet, SecretEnvironment,
};
use crate::workflow::react::experiment_owner::patch::PatchContext;
use crate::workflow::react::experiment_owner::{
    ExecutionOwner, OwnerAcquireRequest, PreparedWorkspace,
};
use crate::workflow::react::experiment_schedule::types::{
    DispatchMarker, JobSagaStage, JobState, OwnerFence, OwnerKind, RecoveryDecision, ScheduleError,
    ScheduleErrorCode,
};
use std::path::PathBuf;
use std::sync::Arc;

/// The bounded limits of one scheduler instance.
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// Lease length granted to a claim.
    pub lease_ms: u64,
    /// Maximum jobs processed in one tick.
    pub max_jobs_per_tick: usize,
    /// How long one tick waits for a run to reach its terminal state.
    pub terminal_wait_ms: u64,
    /// Poll interval while waiting for the terminal state.
    pub terminal_poll_ms: u64,
    /// Sleep between ticks in the supervised loop.
    pub poll_ms: u64,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            lease_ms: 60_000,
            max_jobs_per_tick: 4,
            terminal_wait_ms: 30_000,
            terminal_poll_ms: 250,
            poll_ms: 1_000,
        }
    }
}

/// The run kernel the scheduler drives.
///
/// It is a trait so the scheduler has exactly one dispatch path and the
/// implementation stays in the application service (where the Windows and
/// campaigns live), never a second lifecycle.
pub trait ScheduledRunKernel: Send + Sync {
    /// Confirms, **before any dispatch intent is recorded**, that this job can
    /// actually execute inside the environment its owner prepared.
    ///
    /// This is the pre-dispatch gate: a kernel that cannot honour the owner
    /// context (or any other precondition) must refuse here, so the job ends as
    /// a clean pre-dispatch failure instead of leaving a dispatch intent that
    /// forces the job to be parked as an unknown effect.
    fn preflight_dispatch(&self, request: &ScheduledDispatch<'_>) -> Result<(), ScheduleError>;
    /// Starts one run for a claimed job and returns its backend identity.
    fn dispatch(&self, request: &ScheduledDispatch<'_>) -> Result<DispatchedRun, ScheduleError>;
    /// The authoritative verdict of a dispatched run.
    ///
    /// "Terminal" and "succeeded" are deliberately two different things, so the
    /// kernel answers with an explicit verdict instead of one boolean whose
    /// meaning a caller could invert (a live run must never look finished, and a
    /// failed run must never look successful).
    fn run_verdict(&self, run_id: &str) -> Result<RunVerdict, ScheduleError>;
    /// Best-effort stop, used only for cancellation of a known running job.
    fn stop_run(&self, run_id: &str) -> Result<(), ScheduleError>;
}

/// The authoritative verdict of a dispatched run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunVerdict {
    /// No authoritative terminal state yet: the run may still be executing.
    Running,
    /// The run reached a successful terminal state.
    Succeeded,
    /// The run reached a failed or cancelled terminal state.
    Failed,
}

impl RunVerdict {
    /// Whether the run has provably reached a terminal state.
    pub fn is_terminal(&self) -> bool {
        !matches!(self, RunVerdict::Running)
    }

    /// The value the restart classifier expects: `Some(true)` only when the run
    /// is provably terminal, `None` while it may still be running.
    pub fn terminal_flag(&self) -> Option<bool> {
        self.is_terminal().then_some(true)
    }
}

/// Everything the kernel needs to start one scheduled run.
pub struct ScheduledDispatch<'a> {
    pub campaign_id: &'a str,
    pub job_id: &'a str,
    pub candidate_key: &'a str,
    pub task_id: &'a str,
    pub suite: &'a str,
    pub plan: &'a CampaignPlanV1,
    /// The verified capability leases of this job, when the campaign declares
    /// bundles. Every declared bundle is retained; collisions fail before a
    /// dispatch intent exists.
    pub capabilities: Option<&'a PreparedCapabilityLeaseSet>,
    /// The owned workspace the run executes against.
    pub workspace: Option<&'a PreparedWorkspace>,
    /// The kind of owner that prepared this environment.
    ///
    /// The kernel uses it to decide whether an isolated execution instance can
    /// be derived: a filesystem-only owner provides none, so a scheduled run
    /// under it must be refused before any dispatch intent exists (INV-4).
    pub owner_kind: OwnerKind,
}

/// The identity a kernel reports back for a started run.
#[derive(Debug, Clone)]
pub struct DispatchedRun {
    pub run_id: String,
    pub session_id: Option<String>,
}

/// Resolves the server-side resources one job needs.
///
/// Every method is server-configured: an execution profile, a bundle allowlist
/// and the credential input decide what a job may use, so a caller cannot widen
/// it through the durable surface.
pub trait SchedulerResources: Send + Sync {
    /// The execution owner for one job. An `Err` is a pre-dispatch failure.
    fn owner_for(
        &self,
        job: &JobRecord,
        campaign: &CampaignRecord,
    ) -> Result<Box<dyn ExecutionOwner>, ScheduleError>;

    /// The allowlisted bundle root, when bundles are configured at all.
    fn bundle_registry(&self) -> Option<BundleRegistry> {
        None
    }

    /// The secret values this domain may resolve for a run.
    fn secrets(&self) -> SecretEnvironment {
        SecretEnvironment::default()
    }

    /// The immutable base revision the owner must create the workspace at.
    ///
    /// It comes from the server-side execution profile, never from a job or a
    /// caller, so a schedule can never point a run at another revision.
    fn base_revision(&self, campaign: &CampaignRecord) -> String;

    /// Where published patches go.
    fn artifacts_root(&self) -> PathBuf;

    /// Where staged bundles go.
    fn bundles_root(&self) -> PathBuf;
}

/// What one tick did.
#[derive(Debug, Clone)]
pub enum TickOutcome {
    /// No claimable work.
    Idle,
    /// Work was processed; the outcomes are in claim order.
    Processed(Vec<JobOutcome>),
}

/// The durable outcome of one processed job.
#[derive(Debug, Clone)]
pub struct JobOutcome {
    pub job_id: String,
    pub state: JobState,
    pub run_id: Option<String>,
    pub artifact_path: Option<String>,
    pub error_code: Option<String>,
}

/// The bounded, recoverable campaign scheduler.
pub struct CampaignScheduler {
    store: ExperimentScheduleStore,
    owner_id: String,
    config: SchedulerConfig,
    resources: Arc<dyn SchedulerResources>,
    kernel: Arc<dyn ScheduledRunKernel>,
    /// Whether the restart classification has already run for this process.
    ///
    /// Restart classification and steady-state observation ask different
    /// questions: only a process that has just started may treat a live run as
    /// an unprovable effect to park, and a tick that simply finds a run still
    /// executing must leave it alone.
    startup_classification_done: std::sync::atomic::AtomicBool,
}

impl CampaignScheduler {
    pub fn new(
        store: ExperimentScheduleStore,
        owner_id: impl Into<String>,
        config: SchedulerConfig,
        resources: Arc<dyn SchedulerResources>,
        kernel: Arc<dyn ScheduledRunKernel>,
    ) -> Self {
        Self {
            store,
            owner_id: owner_id.into(),
            config,
            resources,
            kernel,
            startup_classification_done: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn config(&self) -> &SchedulerConfig {
        &self.config
    }

    /// One bounded pass: recover, then process at most `max_jobs_per_tick` jobs.
    pub fn tick(&self, now_ms: u64) -> Result<TickOutcome, ScheduleError> {
        let startup = !self
            .startup_classification_done
            .swap(true, std::sync::atomic::Ordering::SeqCst);
        self.recover(now_ms, startup)?;
        let mut outcomes = Vec::new();
        for _ in 0..self.config.max_jobs_per_tick {
            match self
                .store
                .claim_next_job(&self.owner_id, now_ms, self.config.lease_ms)
            {
                Ok(ClaimOutcome::Idle) => break,
                Ok(ClaimOutcome::Claimed(record)) => {
                    outcomes.push(self.process(*record, now_ms));
                }
                Err(error) if error.code == ScheduleErrorCode::LeaseConflict => {
                    // A live lease means another worker (or an earlier
                    // generation of this one) owns the job. That is the lease
                    // doing its job, not a scheduler failure.
                    log::debug!("[Scheduler] nothing claimable: {}", error.message);
                    break;
                }
                Err(error) => return Err(error),
            }
        }
        if outcomes.is_empty() {
            Ok(TickOutcome::Idle)
        } else {
            Ok(TickOutcome::Processed(outcomes))
        }
    }

    /// Whether a steady-state sweep may park a job whose effect it cannot prove
    /// absent.
    ///
    /// Only a `running` job can be a *live* run the supervisor itself started,
    /// so only that state needs the restart gate. Every other unprovable state —
    /// a dispatch intent without a confirmed run id above all — is a crash
    /// artifact no healthy tick leaves behind, and it must still be parked by a
    /// later sweep: the restart classification runs once per process, and a job
    /// it had to defer while the dead generation's lease was still live would
    /// otherwise never be re-classified.
    fn may_park_in_steady_state(state: JobState) -> bool {
        state != JobState::Running
    }

    /// Re-classifies every non-terminal job from durable state plus the run
    /// authority.
    ///
    /// `startup` distinguishes the two questions this sweep answers:
    ///
    /// - A `running` job whose run is not provably terminal is parked as
    ///   `unknown_manual` **only** by a process that has just started: in steady
    ///   state that state is the supervisor's own live run, and parking it would
    ///   abandon every run slower than one tick. Every *other* unprovable state
    ///   (a dispatch intent with no confirmed run above all) is a crash artifact
    ///   and is parked by any sweep, because the restart classification runs once
    ///   per process and a job it deferred while the dead generation's lease was
    ///   still live must not be left unclassified forever.
    /// - A confirmed run that has reached its terminal state is collected here,
    ///   in either mode, because collection is pure observation plus atomic
    ///   publication. A run that outlives its dispatching tick is never claimed
    ///   again (claims are for `not_dispatched` work), so without this sweep it
    ///   would stay `running` forever.
    ///
    /// This never dispatches, never requeues and never calls the run kernel.
    pub fn recover(&self, now_ms: u64, startup: bool) -> Result<usize, ScheduleError> {
        let kernel = self.kernel.clone();
        // The classifier's flag means "provably terminal", never "succeeded".
        let records = self.store.classify_restart(move |run_id| {
            kernel
                .run_verdict(run_id)
                .ok()
                .and_then(|verdict| verdict.terminal_flag())
        })?;
        let mut handled = 0usize;
        for record in records {
            match record.decision {
                RecoveryDecision::UnknownManual => {
                    if !startup && !Self::may_park_in_steady_state(record.state) {
                        log::debug!(
                            "[Scheduler][job={}] a live run in steady state is left durable",
                            record.job_id
                        );
                        continue;
                    }
                    match self.store.park_unknown_manual(
                        &record.job_id,
                        record.run_id.as_deref().map(|_| false),
                        "restart recovery could not prove the effect absent",
                        now_ms,
                    ) {
                        Ok(_) => {
                            handled += 1;
                            log::warn!(
                                "[Scheduler][job={}] Parked as unknown_manual after restart",
                                record.job_id
                            );
                        }
                        // A live lease means a worker still owns this job, so
                        // recovery must not touch it; the next sweep re-checks.
                        Err(error) if error.code == ScheduleErrorCode::LeaseConflict => {
                            log::debug!(
                                "[Scheduler][job={}] still leased; deferring recovery",
                                record.job_id
                            );
                        }
                        Err(error) => return Err(error),
                    }
                }
                RecoveryDecision::Collect => {
                    if self.advance_collection(&record.job_id, now_ms)? {
                        handled += 1;
                    }
                }
                decision => log::debug!(
                    "[Scheduler][job={}] recovery decision {}",
                    record.job_id,
                    decision.as_str()
                ),
            }
        }
        Ok(handled)
    }

    /// Collects a job whose run is already terminal.
    ///
    /// It adopts the environment the same owner generation created and never
    /// acquires a fresh one, so it cannot dispatch, prepare or restart anything.
    /// A live run is left durable, and an environment that cannot be adopted is
    /// parked rather than reported as a (possibly false) success.
    fn advance_collection(&self, job_id: &str, now_ms: u64) -> Result<bool, ScheduleError> {
        let record = self.store.get_job(job_id)?;
        if record.job.state.is_terminal() {
            return Ok(false);
        }
        let Some(run_id) = record.job.run_id.clone() else {
            return Ok(false);
        };
        let verdict = self.kernel.run_verdict(&run_id)?;
        if !verdict.is_terminal() {
            // A live run stays durable; the next sweep re-observes it.
            return Ok(false);
        }
        let campaign = self.store.get_campaign(&record.job.campaign_id)?;
        let fence = OwnerFence::new(self.owner_id.clone(), record.job.lease_generation);
        let owner = self.resources.owner_for(&record, &campaign)?;
        let acquire = OwnerAcquireRequest {
            job_id: record.job.job_id.clone(),
            fence: fence.clone(),
            base_revision: self.resources.base_revision(&campaign),
            input_patch: None,
            bundle_source_root: (!campaign.bundle_refs.is_empty())
                .then(|| self.resources.bundles_root().join(&record.job.job_id)),
        };
        let workspace = match owner.adopt(&acquire) {
            Ok(Some(workspace)) => workspace,
            Ok(None) => {
                // The run is provably terminal, so no process can still need
                // this job's staged bundle tree. The owner itself is absent and
                // therefore cannot be safely cleaned through its proof, but the
                // job-scoped staging root has a bounded, idempotent cleanup.
                if let Err(error) =
                    bundle::release_job_staging(&self.resources.bundles_root(), job_id)
                {
                    log::warn!(
                        "[Scheduler][job={job_id}] failed to release terminal job staging ({}): {}",
                        error.code.as_str(),
                        error.message
                    );
                }
                // The run finished but its owned environment is gone, so the
                // output patch cannot be published. A false success is worse
                // than an explicit, operator-visible terminal state.
                log::warn!(
                    "[Scheduler][job={job_id}] run is terminal but its owned workspace cannot be \
                     adopted; parking it instead of reporting an unverifiable success"
                );
                self.store.park_unknown_manual(
                    job_id,
                    Some(false),
                    "the run is terminal but its owned workspace cannot be adopted",
                    now_ms,
                )?;
                return Ok(true);
            }
            Err(error) => {
                // A terminal verdict permits removal of only the bounded job
                // staging root. Do not guess at owner cleanup without an
                // adopted proof: an unavailable owner transport is not proof
                // that an observed container is ours.
                if let Err(release_error) =
                    bundle::release_job_staging(&self.resources.bundles_root(), job_id)
                {
                    log::warn!(
                        "[Scheduler][job={job_id}] failed to release terminal job staging after adopt error ({}): {}",
                        release_error.code.as_str(),
                        release_error.message
                    );
                }
                return Err(error);
            }
        };
        let outcome = self.collect_dispatched(
            &record,
            owner.as_ref(),
            &workspace,
            &run_id,
            // The durable row stores the run id, which is also the session id of
            // the run this scheduler dispatched.
            Some(run_id.as_str()),
            verdict == RunVerdict::Succeeded,
            now_ms,
        )?;
        log::info!(
            "[Scheduler][job={job_id}] collected a run that outlived its dispatching tick: {:?}",
            outcome.state
        );
        Ok(true)
    }

    /// Processes one claimed job end to end.
    fn process(&self, record: JobRecord, now_ms: u64) -> JobOutcome {
        let job_id = record.job.job_id.clone();
        let fence = OwnerFence::new(self.owner_id.clone(), record.job.lease_generation);
        match self.process_inner(&record, &fence, now_ms) {
            Ok(outcome) => outcome,
            Err(error) => {
                log::warn!(
                    "[Scheduler][job={}] failed ({}): {}",
                    job_id,
                    error.code.as_str(),
                    error.message
                );
                // Nothing was dispatched on any error path below the intent
                // marker, so the job is a pre-dispatch failure.
                let state = match self.store.get_job(&job_id) {
                    Ok(current) => current.job.state,
                    Err(_) => JobState::FailedPrecondition,
                };
                if !state.is_terminal() {
                    let _ = self.store.transition(
                        &fence,
                        crate::db::experiment_schedule::TransitionRequest {
                            job_id: &job_id,
                            from: state,
                            to: JobState::FailedPrecondition,
                            marker: DispatchMarker::NotDispatched,
                            run_id: None,
                            session_id: None,
                            artifact_dir: None,
                            error_code: Some(error.code.as_str()),
                            journal: None,
                            journal_detail: Some(&error.message),
                            now_ms,
                        },
                    );
                }
                JobOutcome {
                    job_id,
                    state: JobState::FailedPrecondition,
                    run_id: None,
                    artifact_path: None,
                    error_code: Some(error.code.as_str().to_string()),
                }
            }
        }
    }

    fn process_inner(
        &self,
        record: &JobRecord,
        fence: &OwnerFence,
        now_ms: u64,
    ) -> Result<JobOutcome, ScheduleError> {
        let job = &record.job;
        let campaign = self.store.get_campaign(&job.campaign_id)?;
        if campaign.execution_profile_hash.as_deref() != record.execution_profile_hash.as_deref()
            || record.execution_profile_hash.is_none()
            || record
                .execution_profile_hash
                .as_deref()
                .is_some_and(|hash| {
                    !crate::workflow::react::experiment_schedule::types::is_sha256_hex(hash)
                })
        {
            return Err(ScheduleError::new(
                ScheduleErrorCode::InvalidExecutionProfile,
                "the durable job and campaign do not carry the same historical execution profile digest",
            ));
        }

        // A campaign that stopped accepting work ends its claimable jobs here.
        if !campaign.status.accepts_new_work() {
            self.store.transition(
                fence,
                crate::db::experiment_schedule::TransitionRequest {
                    job_id: &job.job_id,
                    from: job.state,
                    to: JobState::Cancelled,
                    marker: DispatchMarker::NotDispatched,
                    run_id: None,
                    session_id: None,
                    artifact_dir: None,
                    error_code: None,
                    journal: Some(JobSagaStage::CleanupDone),
                    journal_detail: Some("campaign no longer accepts work"),
                    now_ms,
                },
            )?;
            return Ok(JobOutcome {
                job_id: job.job_id.clone(),
                state: JobState::Cancelled,
                run_id: None,
                artifact_path: None,
                error_code: None,
            });
        }

        // 1. Resolve and preflight the owner *before* anything is prepared: an
        //    unavailable runtime is a pre-dispatch failure with no host fallback.
        let owner = self.resources.owner_for(record, &campaign)?;
        owner.preflight()?;

        // 2. Allocate the server-derived job bundle root before acquiring a
        // Docker owner. A profile-declared read-only bundle mount must exist at
        // container creation time; it never comes from the schedule request.
        let bundle_source_root = (!campaign.bundle_refs.is_empty())
            .then(|| self.resources.bundles_root().join(&job.job_id));
        if let Some(root) = &bundle_source_root {
            std::fs::create_dir_all(root).map_err(|error| {
                ScheduleError::new(
                    ScheduleErrorCode::BundleNotVerifiable,
                    format!(
                        "failed to create verified bundle root '{}': {error}",
                        root.display()
                    ),
                )
            })?;
        }

        // 3. Acquire the owned workspace.
        let acquire = OwnerAcquireRequest {
            job_id: job.job_id.clone(),
            fence: fence.clone(),
            base_revision: self.resources.base_revision(&campaign),
            input_patch: None,
            bundle_source_root,
        };
        let workspace = match owner.acquire(&acquire) {
            Ok(workspace) => workspace,
            Err(error) => {
                let _ = bundle::release_job_staging(&self.resources.bundles_root(), &job.job_id);
                return Err(error);
            }
        };
        self.store.record_stage(
            fence,
            &job.job_id,
            JobSagaStage::WorkspaceAcquired,
            Some(&workspace.proof.describe()),
            now_ms,
        )?;

        // 4. Stage and verify the campaign's bundles, then mint the capability
        //    lease set. Nothing is registered before verification succeeds.
        let mut staged_bundles: Vec<StagedBundle> = Vec::new();
        let lease_result = (|| -> Result<Option<PreparedCapabilityLeaseSet>, ScheduleError> {
            if campaign.bundle_refs.is_empty() {
                return Ok(None);
            }
            let registry = self.resources.bundle_registry().ok_or_else(|| {
                ScheduleError::new(
                    ScheduleErrorCode::BundleRefUnknown,
                    "this domain configures no bundle allowlist",
                )
            })?;
            let bundles_root = self.resources.bundles_root();
            let secrets = self.resources.secrets();
            let mut leases = Vec::with_capacity(campaign.bundle_refs.len());
            for bundle_ref in &campaign.bundle_refs {
                let source = registry.acquire(bundle_ref)?;
                let staged = bundle::stage_bundle(&source, &bundles_root, &job.job_id)?;
                self.store.record_stage(
                    fence,
                    &job.job_id,
                    JobSagaStage::BundleStaged,
                    Some(bundle_ref),
                    now_ms,
                )?;
                let minted = PreparedCapabilityLease::from_verified_bundle(
                    &staged,
                    &fence.token_hash(&job.job_id),
                    &secrets,
                )?;
                self.store.record_stage(
                    fence,
                    &job.job_id,
                    JobSagaStage::BundleVerified,
                    Some(&minted.describe()),
                    now_ms,
                )?;
                staged_bundles.push(staged);
                leases.push(minted);
            }
            PreparedCapabilityLeaseSet::new(leases).map(Some)
        })();

        let capabilities = match lease_result {
            Ok(lease) => lease,
            Err(error) => {
                // Nothing was dispatched, so the whole preparation rolls back.
                release_all(&staged_bundles);
                let _ = bundle::release_job_staging(&self.resources.bundles_root(), &job.job_id);
                let _ = owner.cleanup(&workspace);
                return Err(error);
            }
        };

        // 4. The environment is ready: Prepared with no dispatch marker.
        let prepared = self.store.transition(
            fence,
            crate::db::experiment_schedule::TransitionRequest {
                job_id: &job.job_id,
                from: JobState::Preparing,
                to: JobState::Prepared,
                marker: DispatchMarker::NotDispatched,
                run_id: None,
                session_id: None,
                artifact_dir: None,
                error_code: None,
                journal: Some(JobSagaStage::EnvironmentReady),
                journal_detail: None,
                now_ms,
            },
        )?;
        debug_assert_eq!(prepared.job.state, JobState::Prepared);

        let dispatch = ScheduledDispatch {
            campaign_id: &campaign.campaign_id,
            job_id: &job.job_id,
            candidate_key: &job.candidate_key,
            task_id: &job.task_id,
            suite: &job.suite,
            plan: &campaign.plan,
            capabilities: capabilities.as_ref(),
            workspace: Some(&workspace),
            owner_kind: owner.kind(),
        };

        // 5. Ask the kernel *before* recording any intent whether it can honour
        //    this job's owner context. A refusal here is a clean pre-dispatch
        //    failure (INV-4), and it must not leave a dispatch intent behind:
        //    an intent recorded for a run that never started would force the job
        //    to be parked as an unprovable effect on the next restart.
        if let Err(error) = self.kernel.preflight_dispatch(&dispatch) {
            release_all(&staged_bundles);
            let _ = bundle::release_job_staging(&self.resources.bundles_root(), &job.job_id);
            let _ = owner.cleanup(&workspace);
            return Err(error);
        }

        // 6. Durable dispatch intent, written *before* the kernel is called.
        self.store
            .mark_dispatch_intent(fence, &job.job_id, now_ms)?;

        // 7. Dispatch exactly once.
        let dispatched = match self.kernel.dispatch(&dispatch) {
            Ok(dispatched) => dispatched,
            Err(error) => {
                // The kernel refused before any effect; the intent stays
                // recorded so a later tick re-classifies rather than silently
                // re-dispatching.
                release_all(&staged_bundles);
                let _ = bundle::release_job_staging(&self.resources.bundles_root(), &job.job_id);
                let _ = owner.cleanup(&workspace);
                return Err(error);
            }
        };

        // 8. Record the confirmed run and wait for its authoritative terminal
        //    state within this tick's bounded budget.
        self.store.transition(
            fence,
            crate::db::experiment_schedule::TransitionRequest {
                job_id: &job.job_id,
                from: JobState::Dispatching,
                to: JobState::Running,
                marker: DispatchMarker::Confirmed,
                run_id: Some(&dispatched.run_id),
                session_id: dispatched.session_id.as_deref(),
                artifact_dir: None,
                error_code: None,
                journal: Some(JobSagaStage::WorkflowStarted),
                journal_detail: None,
                now_ms,
            },
        )?;

        // The run is confirmed, so the capabilities this job verified are now
        // attached to it: record the stage so a later reader can prove the run
        // only ever saw the bundle that was verified for this job (AC-4).
        if let Some(lease) = capabilities.as_ref() {
            self.store.record_stage(
                fence,
                &job.job_id,
                JobSagaStage::CapabilitiesRegistered,
                Some(&lease.describe()),
                now_ms,
            )?;
        }

        let verdict = self.wait_for_terminal(&dispatched.run_id);
        let Some(run_succeeded) = verdict else {
            // Still running: leave the job durable for the next tick. The owned
            // environment stays, because the run is still using it.
            return Ok(JobOutcome {
                job_id: job.job_id.clone(),
                state: JobState::Running,
                run_id: Some(dispatched.run_id),
                artifact_path: None,
                error_code: None,
            });
        };

        // 9. Collect the output patch while the environment still exists, then
        //    record the terminal state the run's own verdict earned.
        let outcome = self.collect_dispatched(
            record,
            owner.as_ref(),
            &workspace,
            &dispatched.run_id,
            dispatched.session_id.as_deref(),
            run_succeeded,
            now_ms,
        );
        if outcome.is_err() {
            // The run is authoritatively terminal, so collection failure must
            // not leave its capability staging or owner environment behind.
            let _ = bundle::release_job_staging(&self.resources.bundles_root(), &job.job_id);
            let _ = owner.cleanup(&workspace);
        }
        release_all(&staged_bundles);
        outcome
    }

    /// Publishes the run's evidence and finalizes the job.
    ///
    /// It is the single place that turns "the run is terminal" into a durable
    /// terminal job, and it is deliberately pure observation plus atomic
    /// publication, so it can run both inside the dispatching tick and from the
    /// sweep that finds a run that outlived its tick.
    fn collect_dispatched(
        &self,
        job: &JobRecord,
        owner: &dyn ExecutionOwner,
        workspace: &PreparedWorkspace,
        run_id: &str,
        session_id: Option<&str>,
        run_succeeded: bool,
        now_ms: u64,
    ) -> Result<JobOutcome, ScheduleError> {
        let fence = OwnerFence::new(self.owner_id.clone(), job.job.lease_generation);
        // The durable row is the authority: the record in hand may still carry
        // the state the job was claimed in, and a previous collection attempt may
        // already have moved the job into `collecting`.
        let durable_state = self.store.get_job(&job.job.job_id)?.job.state;
        if durable_state == JobState::Running {
            self.store.transition(
                &fence,
                crate::db::experiment_schedule::TransitionRequest {
                    job_id: &job.job.job_id,
                    from: JobState::Running,
                    to: JobState::Collecting,
                    marker: DispatchMarker::Confirmed,
                    run_id: Some(run_id),
                    session_id,
                    artifact_dir: None,
                    error_code: None,
                    journal: None,
                    journal_detail: None,
                    now_ms,
                },
            )?;
        }
        let context = PatchContext {
            job_id: job.job.job_id.clone(),
            run_id: Some(run_id.to_string()),
            session_id: session_id.map(str::to_string),
            candidate_key: job.job.candidate_key.clone(),
            base_revision: workspace.proof.base_revision.clone(),
            // An owner that confines publication (the Harbor task environment)
            // declares the root Harbor actually collects; only an owner that
            // declares none falls back to the scheduler's own artifact root.
            destination_root: owner
                .artifact_root()
                .unwrap_or_else(|| self.resources.artifacts_root()),
        };
        let artifact = owner.collect_output_patch(workspace, &context)?;
        let artifact_dir = artifact.directory.to_string_lossy().to_string();
        self.store.record_stage(
            &fence,
            &job.job.job_id,
            JobSagaStage::ArtifactsCollected,
            Some(&artifact.relative_path),
            now_ms,
        )?;

        // 10. Tear the owned environment down and journal it while the job is still
        //     `collecting`: the fenced journal append belongs to the running saga,
        //     and the terminal transition below closes it.
        if let Err(error) =
            bundle::release_job_staging(&self.resources.bundles_root(), &job.job.job_id)
        {
            log::warn!(
                "[Scheduler][job={}] failed to release staged bundles ({})",
                job.job.job_id,
                error.code.as_str()
            );
        }
        if let Err(error) = owner.cleanup(workspace) {
            log::warn!(
                "[Scheduler][job={}] cleanup failed ({}): {}",
                job.job.job_id,
                error.code.as_str(),
                error.message
            );
        }
        self.store.record_stage(
            &fence,
            &job.job.job_id,
            JobSagaStage::CleanupDone,
            None,
            now_ms,
        )?;

        // 11. The authoritative terminal state decides the job's own terminal
        //     state: a run that ended in failure must never be recorded as a
        //     success, even though its evidence was collected.
        let (final_state, error_code) = if run_succeeded {
            (JobState::Succeeded, None)
        } else {
            (JobState::Failed, Some("run_failed"))
        };
        self.store.transition(
            &fence,
            crate::db::experiment_schedule::TransitionRequest {
                job_id: &job.job.job_id,
                from: JobState::Collecting,
                to: final_state,
                marker: DispatchMarker::Confirmed,
                run_id: Some(run_id),
                session_id,
                artifact_dir: Some(&artifact_dir),
                error_code,
                journal: None,
                journal_detail: None,
                now_ms,
            },
        )?;

        Ok(JobOutcome {
            job_id: job.job.job_id.clone(),
            state: final_state,
            run_id: Some(run_id.to_string()),
            artifact_path: Some(artifact.relative_path),
            error_code: error_code.map(str::to_string),
        })
    }

    /// Waits for the run's authoritative verdict within the tick budget.
    ///
    /// `None` means "no terminal verdict inside this tick", which is not a
    /// failure: the job stays durable and the next tick re-observes it.
    fn wait_for_terminal(&self, run_id: &str) -> Option<bool> {
        let deadline = std::time::Instant::now()
            + std::time::Duration::from_millis(self.config.terminal_wait_ms);
        let poll = std::time::Duration::from_millis(self.config.terminal_poll_ms.max(10));
        loop {
            match self.kernel.run_verdict(run_id) {
                Ok(RunVerdict::Succeeded) => return Some(true),
                Ok(RunVerdict::Failed) => return Some(false),
                Ok(RunVerdict::Running) => {}
                Err(error) => {
                    log::warn!(
                        "[Scheduler] run verdict probe failed ({}): {}",
                        error.code.as_str(),
                        error.message
                    );
                    return None;
                }
            }
            if std::time::Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(poll);
        }
    }
}

/// Releases every staged bundle, logging failures instead of masking them.
fn release_all(bundles: &[StagedBundle]) {
    for staged in bundles {
        if let Err(error) = bundle::release_staged_bundle(staged) {
            log::warn!(
                "[Scheduler] failed to release the staged bundle of job {} ({}): {}",
                staged.job_id,
                error.code.as_str(),
                error.message
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::experiment_schedule::ExperimentScheduleStore;
    use crate::headless::domain::ExperimentDomain;
    use crate::workflow::react::experiment_owner::patch::{PatchArtifact, PatchContext};
    use crate::workflow::react::experiment_owner::worktree::HostWorktreeOwner;
    use crate::workflow::react::experiment_owner::{InputPatch, OwnerAcquireRequest};
    use crate::workflow::react::experiment_schedule::types::{
        parse_and_validate_campaign_schedule_request, CAMPAIGN_SCHEDULE_V1,
    };
    use serde_json::json;
    use std::path::Path;
    use std::process::{Command, Stdio};
    use std::sync::Mutex;
    use tempfile::tempdir;

    /// A kernel that records dispatches and answers terminality as configured.
    struct FakeKernel {
        dispatches: Mutex<Vec<String>>,
        /// The verdict every dispatched run reports. Mutable so a test can let a
        /// run reach its terminal state between ticks.
        verdict: Mutex<RunVerdict>,
        /// When set, the pre-dispatch gate refuses with this code.
        preflight_refusal: Option<ScheduleErrorCode>,
    }

    impl FakeKernel {
        fn new(verdict: RunVerdict) -> Arc<Self> {
            Arc::new(Self {
                dispatches: Mutex::new(Vec::new()),
                verdict: Mutex::new(verdict),
                preflight_refusal: None,
            })
        }

        /// A kernel that refuses at the pre-dispatch gate.
        fn refusing(code: ScheduleErrorCode) -> Arc<Self> {
            Arc::new(Self {
                dispatches: Mutex::new(Vec::new()),
                verdict: Mutex::new(RunVerdict::Running),
                preflight_refusal: Some(code),
            })
        }

        /// Records that every in-flight run reached this verdict.
        fn set_verdict(&self, verdict: RunVerdict) {
            *self.verdict.lock().expect("lock") = verdict;
        }

        fn dispatch_count(&self) -> usize {
            self.dispatches.lock().expect("lock").len()
        }
    }

    impl ScheduledRunKernel for FakeKernel {
        fn preflight_dispatch(
            &self,
            _request: &ScheduledDispatch<'_>,
        ) -> Result<(), ScheduleError> {
            match self.preflight_refusal {
                Some(code) => Err(ScheduleError::new(code, "fake preflight refusal")),
                None => Ok(()),
            }
        }

        fn dispatch(
            &self,
            request: &ScheduledDispatch<'_>,
        ) -> Result<DispatchedRun, ScheduleError> {
            self.dispatches
                .lock()
                .expect("lock")
                .push(request.job_id.to_string());
            Ok(DispatchedRun {
                run_id: format!("run-{}", request.job_id),
                session_id: Some(format!("session-{}", request.job_id)),
            })
        }

        fn run_verdict(&self, _run_id: &str) -> Result<RunVerdict, ScheduleError> {
            Ok(*self.verdict.lock().expect("lock"))
        }

        fn stop_run(&self, _run_id: &str) -> Result<(), ScheduleError> {
            Ok(())
        }
    }

    /// Server-side resources over a real, temporary base repository.
    #[derive(Clone)]
    struct FakeResources {
        repo: PathBuf,
        worktrees: PathBuf,
        artifacts: PathBuf,
        bundles: PathBuf,
        unavailable: bool,
        lose_adoption: bool,
    }

    struct LostAdoptionOwner {
        inner: HostWorktreeOwner,
    }

    impl ExecutionOwner for LostAdoptionOwner {
        fn kind(&self) -> OwnerKind {
            self.inner.kind()
        }

        fn preflight(&self) -> Result<(), ScheduleError> {
            self.inner.preflight()
        }

        fn acquire(
            &self,
            request: &OwnerAcquireRequest,
        ) -> Result<PreparedWorkspace, ScheduleError> {
            self.inner.acquire(request)
        }

        fn adopt(
            &self,
            _request: &OwnerAcquireRequest,
        ) -> Result<Option<PreparedWorkspace>, ScheduleError> {
            Ok(None)
        }

        fn apply_input_patch(
            &self,
            workspace: &PreparedWorkspace,
            patch: &InputPatch,
        ) -> Result<(), ScheduleError> {
            self.inner.apply_input_patch(workspace, patch)
        }

        fn collect_output_patch(
            &self,
            workspace: &PreparedWorkspace,
            context: &PatchContext,
        ) -> Result<PatchArtifact, ScheduleError> {
            self.inner.collect_output_patch(workspace, context)
        }

        fn cleanup(&self, workspace: &PreparedWorkspace) -> Result<(), ScheduleError> {
            self.inner.cleanup(workspace)
        }
    }

    impl SchedulerResources for FakeResources {
        fn owner_for(
            &self,
            _job: &JobRecord,
            _campaign: &CampaignRecord,
        ) -> Result<Box<dyn ExecutionOwner>, ScheduleError> {
            if self.unavailable {
                return Err(ScheduleError::new(
                    ScheduleErrorCode::ExecutorUnavailable,
                    "the configured execution owner is unavailable",
                ));
            }
            let owner = HostWorktreeOwner::new(self.repo.clone(), self.worktrees.clone());
            if self.lose_adoption {
                Ok(Box::new(LostAdoptionOwner { inner: owner }))
            } else {
                Ok(Box::new(owner))
            }
        }

        fn base_revision(&self, _campaign: &CampaignRecord) -> String {
            "HEAD".to_string()
        }

        fn artifacts_root(&self) -> PathBuf {
            self.artifacts.clone()
        }

        fn bundles_root(&self) -> PathBuf {
            self.bundles.clone()
        }
    }

    fn git_in(directory: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(directory)
            .args(args)
            .env("GIT_AUTHOR_NAME", "cs-test")
            .env("GIT_AUTHOR_EMAIL", "cs-test@example.invalid")
            .env("GIT_COMMITTER_NAME", "cs-test")
            .env("GIT_COMMITTER_EMAIL", "cs-test@example.invalid")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("run git");
        assert!(status.success(), "git {args:?} failed");
    }

    fn base_repo(directory: &Path) -> PathBuf {
        let repo = directory.join("base");
        std::fs::create_dir_all(&repo).expect("create base");
        git_in(&repo, &["init", "--quiet"]);
        git_in(&repo, &["config", "user.email", "cs-test@example.invalid"]);
        git_in(&repo, &["config", "user.name", "cs-test"]);
        std::fs::write(repo.join("app.py"), "print('base')\n").expect("write app");
        git_in(&repo, &["add", "-A"]);
        git_in(&repo, &["commit", "--quiet", "-m", "base"]);
        repo
    }

    /// A strict durable schedule request for two candidates.
    fn schedule_request() -> serde_json::Value {
        let resolved = crate::workflow::react::experiment_schedule::fixture::resolve_task(
            "chatspeed-smoke",
            "smoke_reply_ok",
        )
        .expect("fixture");
        json!({
            "schema_version": CAMPAIGN_SCHEDULE_V1,
            "plan": {
                "schema_version": "campaign_plan.v1",
                "campaign_key": "p2gh-scheduler",
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
                "candidates": [
                    { "candidate_key": "baseline", "kind": "baseline" },
                    { "candidate_key": "cand-a", "kind": "candidate",
                      "mutable_surface": ["agent_prompt_ref"],
                      "agent_prompt_ref": "smoke-terse-v1",
                      "prompt_hash": "bb41d700c9a2cdd26bffe26b5a3deac849188ce1966414c32700e38d51d2bf88" }
                ]
            },
            "fixture_refs": [serde_json::to_value(resolved.task_ref()).expect("ref")],
            "execution_profile_ref": "smoke-local",
            "bundle_refs": []
        })
    }

    /// Opens a fresh experiment domain and schedules one campaign.
    fn scheduled_domain(directory: &Path) -> (ExperimentDomain, ExperimentScheduleStore) {
        let domain_root = directory.join("domain");
        std::fs::create_dir_all(&domain_root).expect("create domain");
        let domain = ExperimentDomain::open(&domain_root, "owner-test").expect("open domain");
        let store = ExperimentScheduleStore::new(domain.store().clone());
        let request =
            parse_and_validate_campaign_schedule_request(&schedule_request()).expect("request");
        store
            .schedule_campaign(
                &request,
                "idem-1",
                &"a".repeat(64),
                crate::headless::domain::now_ms(),
            )
            .expect("schedule");
        (domain, store)
    }

    fn resources(directory: &Path, unavailable: bool) -> Arc<FakeResources> {
        Arc::new(FakeResources {
            repo: base_repo(directory),
            worktrees: directory.join("worktrees"),
            artifacts: directory.join("artifacts"),
            bundles: directory.join("bundles"),
            unavailable,
            lose_adoption: false,
        })
    }

    fn scheduler(
        store: ExperimentScheduleStore,
        resources: Arc<FakeResources>,
        kernel: Arc<FakeKernel>,
    ) -> CampaignScheduler {
        scheduler_with_lease(store, resources, kernel, 60_000)
    }

    fn scheduler_with_lease(
        store: ExperimentScheduleStore,
        resources: Arc<FakeResources>,
        kernel: Arc<FakeKernel>,
        lease_ms: u64,
    ) -> CampaignScheduler {
        CampaignScheduler::new(
            store,
            "scheduler-1",
            SchedulerConfig {
                lease_ms,
                max_jobs_per_tick: 4,
                terminal_wait_ms: 50,
                terminal_poll_ms: 5,
                poll_ms: 50,
            },
            resources,
            kernel,
        )
    }

    #[test]
    fn a_queued_campaign_runs_the_whole_saga_and_publishes_a_patch_per_job() {
        let directory = tempdir().expect("tempdir");
        let (_domain, store) = scheduled_domain(directory.path());
        let resources = resources(directory.path(), false);
        let kernel = FakeKernel::new(RunVerdict::Succeeded);
        let scheduler = scheduler(store.clone(), resources.clone(), kernel.clone());

        let outcome = scheduler
            .tick(crate::headless::domain::now_ms())
            .expect("tick");
        let outcomes = match outcome {
            TickOutcome::Processed(outcomes) => outcomes,
            TickOutcome::Idle => panic!("the scheduled campaign must produce work"),
        };
        assert_eq!(outcomes.len(), 2, "one job per candidate");
        for job in &outcomes {
            assert_eq!(job.state, JobState::Succeeded);
            assert!(job.run_id.is_some());
            assert!(job.artifact_path.is_some());
            assert_eq!(job.error_code, None);
        }
        assert_eq!(
            kernel.dispatch_count(),
            2,
            "each job dispatches exactly once"
        );

        // Every job is durable-terminal with its artifact recorded.
        let campaign_id = outcomes[0].job_id.clone();
        let _ = campaign_id;
        let jobs = store.list_jobs(&outcomes[0].job_id).unwrap_or_default();
        let _ = jobs;
        for job in &outcomes {
            let record = store.get_job(&job.job_id).expect("job");
            assert_eq!(record.job.state, JobState::Succeeded);
            assert_eq!(record.job.dispatch_marker, DispatchMarker::Confirmed);
            assert!(record.artifact_dir.is_some());
            assert!(record.job.run_id.is_some());
        }

        // The published patches exist and the owned worktrees are gone.
        let published = std::fs::read_dir(&resources.artifacts)
            .expect("artifacts root")
            .count();
        assert_eq!(published, 2, "one published patch directory per job");
        assert!(
            std::fs::read_dir(&resources.worktrees)
                .map(|entries| entries.count())
                .unwrap_or(0)
                == 0,
            "a succeeded job must leave no owned worktree behind"
        );

        // The saga recorded the ordered stages, and a second tick is idle: a
        // terminal job is never re-dispatched.
        assert!(matches!(
            scheduler
                .tick(crate::headless::domain::now_ms())
                .expect("second tick"),
            TickOutcome::Idle
        ));
        assert_eq!(kernel.dispatch_count(), 2);
    }

    #[test]
    fn an_unavailable_owner_fails_the_job_before_any_dispatch() {
        let directory = tempdir().expect("tempdir");
        let (_domain, store) = scheduled_domain(directory.path());
        let kernel = FakeKernel::new(RunVerdict::Succeeded);
        let scheduler = scheduler(
            store.clone(),
            resources(directory.path(), true),
            kernel.clone(),
        );

        let outcome = scheduler
            .tick(crate::headless::domain::now_ms())
            .expect("tick");
        let outcomes = match outcome {
            TickOutcome::Processed(outcomes) => outcomes,
            TickOutcome::Idle => panic!("the job must be claimed and failed"),
        };
        assert_eq!(outcomes.len(), 2, "both claimed candidates fail");
        for outcome in &outcomes {
            assert_eq!(outcome.state, JobState::FailedPrecondition);
            assert_eq!(
                outcome.error_code.as_deref(),
                Some(ScheduleErrorCode::ExecutorUnavailable.as_str())
            );
        }
        // The run kernel was never called: there is no host fallback.
        assert_eq!(kernel.dispatch_count(), 0);
        let record = store.get_job(&outcomes[0].job_id).expect("job");
        assert_eq!(record.job.state, JobState::FailedPrecondition);
        assert_eq!(record.job.dispatch_marker, DispatchMarker::NotDispatched);
        assert_eq!(
            record.job.error_code.as_deref(),
            Some(ScheduleErrorCode::ExecutorUnavailable.as_str())
        );
    }

    /// A steady-state sweep still parks a crash artifact, and never a live run.
    ///
    /// The restart classification runs once per process, so a job it had to defer
    /// while the crashed generation's lease was still live — a recorded dispatch
    /// intent with no confirmed run above all — is parked by a *later* sweep.
    /// Only a `running` job may still be the supervisor's own live run, so only
    /// that state is left alone in steady state.
    #[test]
    fn a_steady_state_sweep_parks_crash_artifacts_but_never_a_live_run() {
        assert!(
            CampaignScheduler::may_park_in_steady_state(JobState::Dispatching),
            "a dispatch intent with no confirmed run is a crash artifact"
        );
        assert!(
            CampaignScheduler::may_park_in_steady_state(JobState::Preparing)
                && CampaignScheduler::may_park_in_steady_state(JobState::Prepared),
            "a half-prepared job is a crash artifact too"
        );
        assert!(
            !CampaignScheduler::may_park_in_steady_state(JobState::Running),
            "a running job may be the supervisor's own live run"
        );
    }

    /// A run that outlives its dispatching tick is still collected.
    /// Dispatched work is never claimable again (claims are for
    /// `not_dispatched` jobs), so a run whose terminal state arrives after the
    /// tick budget expired used to stay `running` forever. The recovery sweep has
    /// to adopt the environment its own generation created, publish the evidence
    /// and finalize the job — without ever calling the kernel again.
    #[test]
    fn a_run_that_outlives_its_tick_is_collected_by_a_later_tick() {
        let directory = tempdir().expect("tempdir");
        let (_domain, store) = scheduled_domain(directory.path());
        // The run is still executing when the dispatching tick gives up.
        let kernel = FakeKernel::new(RunVerdict::Running);
        let scheduler = scheduler(
            store.clone(),
            resources(directory.path(), false),
            kernel.clone(),
        );

        let first = scheduler
            .tick(crate::headless::domain::now_ms())
            .expect("first tick");
        let outcomes = match first {
            TickOutcome::Processed(outcomes) => outcomes,
            TickOutcome::Idle => panic!("the job must be dispatched"),
        };
        assert_eq!(outcomes.len(), 2, "both claimed candidates are dispatched");
        for outcome in &outcomes {
            assert_eq!(outcome.state, JobState::Running);
        }
        assert_eq!(kernel.dispatch_count(), 2);

        // The run reaches its authoritative terminal state after the tick.
        kernel.set_verdict(RunVerdict::Succeeded);
        scheduler
            .tick(crate::headless::domain::now_ms())
            .expect("collecting tick");

        for outcome in &outcomes {
            let record = store.get_job(&outcome.job_id).expect("job");
            assert_eq!(
                record.job.state,
                JobState::Succeeded,
                "a terminal run must be collected and finalized"
            );
            assert_eq!(record.job.dispatch_marker, DispatchMarker::Confirmed);
            let stages: Vec<String> = store
                .journal(&outcome.job_id)
                .expect("journal")
                .into_iter()
                .map(|(_, stage, _)| stage)
                .collect();
            assert!(
                stages.iter().any(|stage| stage == "artifacts_collected"),
                "the output patch must be published: {stages:?}"
            );
            assert!(
                stages.iter().any(|stage| stage == "cleanup_done"),
                "the owned environment must be cleaned up: {stages:?}"
            );
        }
        assert_eq!(
            kernel.dispatch_count(),
            2,
            "collecting a terminal run never dispatches again"
        );
    }

    #[test]
    fn terminal_run_with_unadoptable_owner_releases_job_staging_and_parks() {
        let directory = tempdir().expect("tempdir");
        let (_domain, store) = scheduled_domain(directory.path());
        let resources = resources(directory.path(), false);
        let mut recovery_resources = (*resources).clone();
        recovery_resources.lose_adoption = true;
        let resources = Arc::new(recovery_resources);
        let kernel = FakeKernel::new(RunVerdict::Running);
        let scheduler = scheduler_with_lease(store.clone(), resources.clone(), kernel.clone(), 1);

        let first = scheduler
            .tick(crate::headless::domain::now_ms())
            .expect("dispatching tick");
        let outcomes = match first {
            TickOutcome::Processed(outcomes) => outcomes,
            TickOutcome::Idle => panic!("the jobs must be dispatched"),
        };
        let job_id = &outcomes[0].job_id;
        let staging = resources.bundles.join(job_id).join("bundle-a");
        std::fs::create_dir_all(&staging).expect("staging");
        std::fs::write(staging.join("marker"), "staged").expect("marker");

        std::thread::sleep(std::time::Duration::from_millis(5));
        kernel.set_verdict(RunVerdict::Succeeded);
        scheduler
            .tick(crate::headless::domain::now_ms())
            .expect("terminal recovery tick");

        let record = store.get_job(job_id).expect("job");
        assert_eq!(record.job.state, JobState::UnknownManual);
        assert!(
            !resources.bundles.join(job_id).exists(),
            "terminal-but-unadoptable jobs must release their bounded staging root"
        );
        assert_eq!(
            kernel.dispatch_count(),
            2,
            "recovery must never dispatch again"
        );
    }

    #[test]
    fn an_unprovable_post_dispatch_effect_is_parked_and_never_redispatched() {
        let directory = tempdir().expect("tempdir");
        let (_domain, store) = scheduled_domain(directory.path());
        // The run authority can never prove the run terminal in this test.
        let kernel = FakeKernel::new(RunVerdict::Running);
        let shared_resources = resources(directory.path(), false);
        let scheduler =
            scheduler_with_lease(store.clone(), shared_resources.clone(), kernel.clone(), 1);

        // First tick: the job is dispatched, then left durable as running
        // because the tick budget elapses without an authoritative verdict.
        let first = scheduler
            .tick(crate::headless::domain::now_ms())
            .expect("first tick");
        let outcomes = match first {
            TickOutcome::Processed(outcomes) => outcomes,
            TickOutcome::Idle => panic!("the job must be dispatched"),
        };
        assert_eq!(outcomes.len(), 2, "both claimed candidates are dispatched");
        for outcome in &outcomes {
            assert_eq!(outcome.state, JobState::Running);
        }
        assert_eq!(kernel.dispatch_count(), 2);

        // Steady state: the supervisor that dispatched this run re-observes it on
        // every tick, and a run it can still see executing must stay durable. If
        // a tick parked its own live run, every run slower than one tick would
        // be abandoned as an unprovable effect.
        let steady = scheduler
            .tick(crate::headless::domain::now_ms())
            .expect("steady-state tick");
        let record = store.get_job(&outcomes[0].job_id).expect("job");
        assert_eq!(
            record.job.state,
            JobState::Running,
            "a live run must not be parked by the supervisor that owns it: {steady:?}"
        );

        // Second tick, after the lease expires: recovery classifies the
        // unprovable effect as unknown_manual and parks it, and the claim then
        // finds no claimable work.
        //
        // A restart is a *new* owner process, so it is modelled as a new
        // scheduler over the same durable state: only a supervisor that has just
        // started may park a run it never dispatched.
        std::thread::sleep(std::time::Duration::from_millis(5));
        let restarted =
            scheduler_with_lease(store.clone(), shared_resources.clone(), kernel.clone(), 1);
        let second = restarted
            .tick(crate::headless::domain::now_ms())
            .expect("second tick");
        assert_eq!(
            kernel.dispatch_count(),
            2,
            "a job whose effect cannot be proven absent is never re-dispatched"
        );
        match second {
            TickOutcome::Idle => {}
            TickOutcome::Processed(outcomes) => {
                panic!("no job may be dispatched again: {outcomes:?}")
            }
        }
        // The job is parked for a human: it is never re-dispatched, and its
        // dispatch marker still records that an effect may have happened.
        let record = store.get_job(&outcomes[0].job_id).expect("job");
        assert_eq!(record.job.state, JobState::UnknownManual);
        assert_eq!(record.job.dispatch_marker, DispatchMarker::Confirmed);
        assert!(record.job.run_id.is_some());
    }

    #[test]
    fn a_run_that_ends_in_failure_marks_the_job_failed_not_succeeded() {
        let directory = tempdir().expect("tempdir");
        let (_domain, store) = scheduled_domain(directory.path());
        let kernel = FakeKernel::new(RunVerdict::Failed);
        let scheduler = scheduler(
            store.clone(),
            resources(directory.path(), false),
            kernel.clone(),
        );

        let outcome = scheduler
            .tick(crate::headless::domain::now_ms())
            .expect("tick");
        let outcomes = match outcome {
            TickOutcome::Processed(outcomes) => outcomes,
            TickOutcome::Idle => panic!("the job must be dispatched"),
        };
        assert_eq!(outcomes.len(), 2);
        for job in &outcomes {
            assert_eq!(job.state, JobState::Failed);
            assert_eq!(job.error_code.as_deref(), Some("run_failed"));
            // The evidence of a failed run is still collected and published.
            assert!(job.artifact_path.is_some());
            let record = store.get_job(&job.job_id).expect("job");
            assert_eq!(record.job.state, JobState::Failed);
            assert_eq!(record.job.error_code.as_deref(), Some("run_failed"));
        }
    }

    #[test]
    fn a_pre_dispatch_refusal_ends_the_job_cleanly_without_an_intent() {
        let directory = tempdir().expect("tempdir");
        let (_domain, store) = scheduled_domain(directory.path());
        // The kernel refuses at the pre-dispatch gate, as the real kernel must
        // while it cannot honour an owner-confirmed execution instance.
        let kernel = FakeKernel::refusing(ScheduleErrorCode::OwnerExecutionContextUnavailable);
        let resources = resources(directory.path(), false);
        let scheduler = scheduler(store.clone(), resources.clone(), kernel.clone());

        let outcome = scheduler
            .tick(crate::headless::domain::now_ms())
            .expect("tick");
        let outcomes = match outcome {
            TickOutcome::Processed(outcomes) => outcomes,
            TickOutcome::Idle => panic!("the job must be claimed and refused"),
        };
        assert_eq!(outcomes.len(), 2);
        for job in &outcomes {
            assert_eq!(job.state, JobState::FailedPrecondition);
            assert_eq!(
                job.error_code.as_deref(),
                Some(ScheduleErrorCode::OwnerExecutionContextUnavailable.as_str())
            );
            // The refusal happened before any dispatch intent: the job is a
            // clean pre-dispatch failure, not an unprovable effect.
            let record = store.get_job(&job.job_id).expect("job");
            assert_eq!(record.job.state, JobState::FailedPrecondition);
            assert_eq!(record.job.dispatch_marker, DispatchMarker::NotDispatched);
            assert!(record.job.run_id.is_none());
        }
        // The run kernel was never asked to start anything, and the owned
        // worktrees were rolled back.
        assert_eq!(kernel.dispatch_count(), 0);
        assert!(
            std::fs::read_dir(&resources.worktrees)
                .map(|entries| entries.count())
                .unwrap_or(0)
                == 0,
            "a refused job must leave no owned worktree behind"
        );
    }

    #[test]
    fn a_tick_without_work_is_idle() {
        let directory = tempdir().expect("tempdir");
        let domain_root = directory.path().join("domain");
        std::fs::create_dir_all(&domain_root).expect("create domain");
        let domain = ExperimentDomain::open(&domain_root, "owner-test").expect("open domain");
        let store = ExperimentScheduleStore::new(domain.store().clone());
        let scheduler = scheduler(
            store,
            resources(directory.path(), false),
            FakeKernel::new(RunVerdict::Succeeded),
        );
        assert!(matches!(
            scheduler
                .tick(crate::headless::domain::now_ms())
                .expect("tick"),
            TickOutcome::Idle
        ));
    }
}
