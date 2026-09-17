//! Phase 2G+2H frozen contracts: durable campaign schedule, job FSM, lease
//! fencing, isolation manifests and the experiment-domain marker.
//!
//! This module is the *contract* half of U-1. It defines the strict, versioned
//! documents and the pure state machine that the backend store (U-2), the
//! durable scheduler (U-8), the execution owner (U-5/U-6), the bundle saga
//! (U-7) and the `cs` CLI (U-4) all share. Nothing here performs I/O, opens a
//! database, spawns a process or reaches a provider.
//!
//! Design rules enforced here (AC-1/2/3/4; INV-2/3/5/6/7/9):
//!
//! - Every external document is strict, snake_case and rejects unknown fields;
//!   caller-forbidden fields fail closed with a stable machine code *before*
//!   any effect.
//! - A durable schedule request carries **fixture refs and digests only**
//!   (`fixture.rs`), never the instruction body, never a host path, never a
//!   caller-minted scope id, never a secret, and never a promotion field.
//! - Job state, dispatch marker and lease generation are one typed FSM. The
//!   restart classification is a *pure function* over typed state, so recovery
//!   can never be inferred from a transcript, a log line or assistant text.
//! - Isolation is fail-closed by construction: an execution profile can only
//!   mount the run workspace or a verified bundle, a docker image must be
//!   digest-pinned, and no manifest carries a secret *value* — only the name of
//!   a secret reference resolved at runtime.

use crate::workflow::react::campaign::{
    campaign_id_for_plan, canonical_hash, parse_campaign_plan, CampaignPlanV1, CampaignSpecError,
};
use crate::workflow::react::experiment_schedule::fixture::FixtureTaskRefV1;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Fixed schema version literals. Each is a distinct, versioned contract.
pub const CAMPAIGN_SCHEDULE_V1: &str = "campaign_schedule.v1";
pub const CAMPAIGN_SCHEDULE_ACCEPTED_V1: &str = "campaign_schedule_accepted.v1";
pub const CAMPAIGN_JOB_V1: &str = "campaign_job.v1";
pub const EXECUTION_PROFILE_V1: &str = "execution_profile.v1";
pub const BUNDLE_MANIFEST_V1: &str = "bundle_manifest.v1";
pub const HARBOR_TASK_CAPABILITY_V1: &str = "harbor_task_capability.v1";
pub const EXPERIMENT_DOMAIN_MARKER_V1: &str = "experiment_domain_marker.v1";
/// Fixed schema version of the durable job-list projection.
pub const CAMPAIGN_JOB_LIST_V1: &str = "campaign_job_list.v1";
/// Fixed schema version of the durable cancel result.
pub const CAMPAIGN_CANCEL_V1: &str = "campaign_cancel.v1";
/// Fixed schema version of the evidence-only reconcile result.
pub const CAMPAIGN_RECONCILE_V1: &str = "campaign_reconcile.v1";

/// The only domain kind a `chatspeed-headless` instance may adopt. An existing
/// database without this marker is never taken over (AC-1/INV-9).
pub const DOMAIN_KIND_EXPERIMENT_V1: &str = "experiment.v1";

/// Stage 0/2G fix the durable queue at one ordered job at a time.
pub const SCHEDULE_CONCURRENCY: u32 = 1;

/// Canonical hash domains for the 2G+2H identities.
pub const SCHEDULE_HASH_DOMAIN: &str = "cs-schedule:request";
pub const JOB_ID_DOMAIN: &str = "cs-schedule:job-id";
pub const OWNER_TOKEN_DOMAIN: &str = "cs-schedule:owner-token";
pub const BUNDLE_CONTENT_HASH_DOMAIN: &str = "cs-bundle:content";
pub const CAPABILITY_HASH_DOMAIN: &str = "cs-harbor:capability";
pub const EXECUTION_PROFILE_HASH_DOMAIN: &str = "cs-execution:profile";

// ---------------------------------------------------------------------------
// Machine codes
// ---------------------------------------------------------------------------

/// Stable machine codes for 2G+2H contract rejections. They describe a request
/// rejected *before* an effect and are part of the HTTP/CLI contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleErrorCode {
    /// The document `schema_version` is missing or unsupported.
    UnsupportedVersion,
    /// A field is not part of the strict document schema.
    UnknownField,
    /// A caller-supplied field is forbidden for this document.
    ForbiddenField,
    /// The campaign key is empty or malformed.
    InvalidCampaignKey,
    /// The agent id is missing.
    InvalidAgentId,
    /// The fixture reference is missing, duplicated or malformed.
    InvalidFixture,
    /// The declared fixture digest does not match the pinned catalog.
    FixtureDigestMismatch,
    /// The referenced execution profile is not registered by the server.
    UnknownExecutionProfile,
    /// The execution profile document is structurally invalid.
    InvalidExecutionProfile,
    /// The owner kind is not implemented by this build.
    UnsupportedOwnerKind,
    /// The local runtime the owner needs is unavailable (fail closed).
    ExecutorUnavailable,
    /// The owner-prepared execution environment cannot be handed to the run
    /// kernel, so a schedule refuses to dispatch rather than risk host
    /// execution (fail closed).
    OwnerExecutionContextUnavailable,
    /// The container image is not pinned by digest.
    ImageNotDigestPinned,
    /// The requested network policy cannot be enforced.
    NetworkPolicyUnsupported,
    /// A referenced bundle is not allowlisted by the server.
    BundleRefUnknown,
    /// The bundle manifest document is structurally invalid.
    BundleManifestInvalid,
    /// The staged bundle content does not match its declared digest.
    BundleDigestMismatch,
    /// A bundle path is absolute, escaping or otherwise unsafe.
    BundlePathUnsafe,
    /// A bundle declares a secret value or an unapproved secret reference.
    BundleSecretForbidden,
    /// A bundle cannot be verified, so it is never registered.
    BundleNotVerifiable,
    /// A job/lease field combination is internally inconsistent.
    InvalidJobState,
    /// The requested job state transition is not part of the FSM.
    InvalidJobTransition,
    /// Another live owner holds a higher or equal lease generation.
    LeaseConflict,
    /// This worker's lease is no longer the current generation.
    LeaseLost,
    /// A dispatch intent was recorded but the effect cannot be proven absent.
    DispatchUncertain,
    /// An owner token does not prove ownership of the resource.
    OwnershipMismatch,
    /// A workspace path escapes its owned root.
    WorkspaceEscape,
    /// The input patch was rejected before it could be applied.
    InputPatchRejected,
    /// The output patch failed the secret/private scan.
    OutputPatchScanFailed,
    /// Atomic artifact publication failed.
    ArtifactPublicationFailed,
    /// The queue is at capacity; nothing was enqueued.
    CapacityExceeded,
    /// The cancel request conflicts with the current durable state.
    CancelConflict,
    /// The campaign id is unknown for this domain.
    UnknownCampaign,
    /// The job id is unknown for this domain.
    UnknownJob,
    /// The campaign is closed or cancelled and accepts no new work.
    CampaignNotActive,
    /// A durable writer transaction failed.
    PersistenceFailure,
    /// The data directory is not an experiment domain (fail closed).
    DomainUnmarked,
    /// Another live instance holds the singleton domain lease.
    DomainLocked,
    /// The data directory layout cannot be used safely.
    DomainLayoutUnsafe,
    /// The same idempotency key was replayed with a different body.
    IdempotencyConflict,
}

impl ScheduleErrorCode {
    /// Stable snake_case machine code.
    pub fn as_str(&self) -> &'static str {
        match self {
            ScheduleErrorCode::UnsupportedVersion => "unsupported_schedule_schema",
            ScheduleErrorCode::UnknownField => "unknown_field",
            ScheduleErrorCode::ForbiddenField => "forbidden_field",
            ScheduleErrorCode::InvalidCampaignKey => "invalid_campaign_key",
            ScheduleErrorCode::InvalidAgentId => "invalid_agent_id",
            ScheduleErrorCode::InvalidFixture => "invalid_fixture_ref",
            ScheduleErrorCode::FixtureDigestMismatch => "fixture_digest_mismatch",
            ScheduleErrorCode::UnknownExecutionProfile => "unknown_execution_profile",
            ScheduleErrorCode::InvalidExecutionProfile => "invalid_execution_profile",
            ScheduleErrorCode::UnsupportedOwnerKind => "unsupported_owner_kind",
            ScheduleErrorCode::ExecutorUnavailable => "executor_unavailable",
            ScheduleErrorCode::OwnerExecutionContextUnavailable => {
                "owner_execution_context_unavailable"
            }
            ScheduleErrorCode::ImageNotDigestPinned => "image_not_digest_pinned",
            ScheduleErrorCode::NetworkPolicyUnsupported => "network_policy_unsupported",
            ScheduleErrorCode::BundleRefUnknown => "bundle_ref_unknown",
            ScheduleErrorCode::BundleManifestInvalid => "bundle_manifest_invalid",
            ScheduleErrorCode::BundleDigestMismatch => "bundle_digest_mismatch",
            ScheduleErrorCode::BundlePathUnsafe => "bundle_path_unsafe",
            ScheduleErrorCode::BundleSecretForbidden => "bundle_secret_forbidden",
            ScheduleErrorCode::BundleNotVerifiable => "bundle_not_verifiable",
            ScheduleErrorCode::InvalidJobState => "invalid_job_state",
            ScheduleErrorCode::InvalidJobTransition => "invalid_job_transition",
            ScheduleErrorCode::LeaseConflict => "lease_conflict",
            ScheduleErrorCode::LeaseLost => "lease_lost",
            ScheduleErrorCode::DispatchUncertain => "dispatch_uncertain",
            ScheduleErrorCode::OwnershipMismatch => "ownership_mismatch",
            ScheduleErrorCode::WorkspaceEscape => "workspace_escape",
            ScheduleErrorCode::InputPatchRejected => "input_patch_rejected",
            ScheduleErrorCode::OutputPatchScanFailed => "output_patch_scan_failed",
            ScheduleErrorCode::ArtifactPublicationFailed => "artifact_publication_failed",
            ScheduleErrorCode::CapacityExceeded => "capacity_exceeded",
            ScheduleErrorCode::CancelConflict => "cancel_conflict",
            ScheduleErrorCode::UnknownCampaign => "unknown_campaign",
            ScheduleErrorCode::UnknownJob => "unknown_job",
            ScheduleErrorCode::CampaignNotActive => "campaign_not_active",
            ScheduleErrorCode::PersistenceFailure => "persistence_failure",
            ScheduleErrorCode::DomainUnmarked => "experiment_domain_unmarked",
            ScheduleErrorCode::DomainLocked => "experiment_domain_locked",
            ScheduleErrorCode::DomainLayoutUnsafe => "experiment_domain_layout_unsafe",
            ScheduleErrorCode::IdempotencyConflict => "idempotency_conflict",
        }
    }
}

/// A 2G+2H contract rejection carrying a stable machine code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleError {
    pub code: ScheduleErrorCode,
    pub message: String,
}

impl ScheduleError {
    pub fn new(code: ScheduleErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for ScheduleError {}

/// Maps a nested 2F plan rejection onto the schedule error surface. The 2F
/// machine code is preserved verbatim in the message so an operator keeps the
/// existing, more specific diagnosis.
fn map_plan_error(error: CampaignSpecError) -> ScheduleError {
    let code = match error.code {
        crate::workflow::react::campaign::CampaignSpecErrorCode::ForbiddenField => {
            ScheduleErrorCode::ForbiddenField
        }
        _ => ScheduleErrorCode::UnknownField,
    };
    ScheduleError::new(code, format!("nested campaign plan rejected: {error}"))
}

// ---------------------------------------------------------------------------
// Job FSM
// ---------------------------------------------------------------------------

/// Durable job state. `succeeded` and the four error outcomes are terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    /// Persisted, ordered, not yet claimed by a worker.
    Queued,
    /// A worker holds the lease and is preparing the owned environment.
    Preparing,
    /// The owned environment is ready; nothing has been dispatched.
    Prepared,
    /// A dispatch intent is recorded; the run kernel may or may not have run.
    Dispatching,
    /// The run kernel confirmed a run id for this job.
    Running,
    /// The run is terminal; artifacts are being read and published.
    Collecting,
    /// Terminal success.
    Succeeded,
    /// Terminal pre-dispatch failure (runtime/policy/precondition).
    FailedPrecondition,
    /// Terminal run/collection failure.
    Failed,
    /// Terminal operator cancellation.
    Cancelled,
    /// Terminal: the effect cannot be proven absent; no automatic retry.
    UnknownManual,
}

impl JobState {
    pub fn as_str(&self) -> &'static str {
        match self {
            JobState::Queued => "queued",
            JobState::Preparing => "preparing",
            JobState::Prepared => "prepared",
            JobState::Dispatching => "dispatching",
            JobState::Running => "running",
            JobState::Collecting => "collecting",
            JobState::Succeeded => "succeeded",
            JobState::FailedPrecondition => "failed_precondition",
            JobState::Failed => "failed",
            JobState::Cancelled => "cancelled",
            JobState::UnknownManual => "unknown_manual",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "queued" => JobState::Queued,
            "preparing" => JobState::Preparing,
            "prepared" => JobState::Prepared,
            "dispatching" => JobState::Dispatching,
            "running" => JobState::Running,
            "collecting" => JobState::Collecting,
            "succeeded" => JobState::Succeeded,
            "failed_precondition" => JobState::FailedPrecondition,
            "failed" => JobState::Failed,
            "cancelled" => JobState::Cancelled,
            "unknown_manual" => JobState::UnknownManual,
            _ => return None,
        })
    }

    /// Terminal states never transition again and never resume automatically.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            JobState::Succeeded
                | JobState::FailedPrecondition
                | JobState::Failed
                | JobState::Cancelled
                | JobState::UnknownManual
        )
    }

    /// States in which the run kernel has provably not been called.
    pub fn is_pre_dispatch(&self) -> bool {
        matches!(
            self,
            JobState::Queued | JobState::Preparing | JobState::Prepared
        )
    }

    /// States that may be requeued after a proven-cleanup pre-dispatch abort.
    pub fn is_requeueable(&self) -> bool {
        matches!(self, JobState::Preparing | JobState::Prepared)
    }
}

/// Whether the run kernel has been invoked for a job. The marker — not any
/// transcript, log or agent text — is the dispatch authority (INV-3/INV-5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchMarker {
    /// No dispatch intent was ever written.
    NotDispatched,
    /// A dispatch intent is durable; the effect may or may not have happened.
    IntentRecorded,
    /// The run kernel returned a run id, so the effect provably happened once.
    Confirmed,
}

impl DispatchMarker {
    pub fn as_str(&self) -> &'static str {
        match self {
            DispatchMarker::NotDispatched => "not_dispatched",
            DispatchMarker::IntentRecorded => "intent_recorded",
            DispatchMarker::Confirmed => "confirmed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "not_dispatched" => DispatchMarker::NotDispatched,
            "intent_recorded" => DispatchMarker::IntentRecorded,
            "confirmed" => DispatchMarker::Confirmed,
            _ => return None,
        })
    }
}

/// Returns whether `from -> to` is one of the FSM's edges. A terminal state may
/// only be re-written to itself (idempotent durable update); nothing else.
pub fn transition_allowed(from: JobState, to: JobState) -> bool {
    use JobState::*;
    if from == to {
        return from.is_terminal();
    }
    matches!(
        (from, to),
        (Queued, Preparing)
            | (Queued, Cancelled)
            | (Queued, FailedPrecondition)
            | (Preparing, Prepared)
            | (Preparing, Queued)
            | (Preparing, FailedPrecondition)
            | (Preparing, Cancelled)
            | (Prepared, Dispatching)
            | (Prepared, Queued)
            | (Prepared, FailedPrecondition)
            | (Prepared, Cancelled)
            | (Dispatching, Running)
            | (Dispatching, UnknownManual)
            | (Running, Collecting)
            | (Running, Failed)
            | (Running, Cancelled)
            | (Running, UnknownManual)
            | (Collecting, Succeeded)
            | (Collecting, Failed)
            | (Collecting, Cancelled)
            | (Collecting, UnknownManual)
    )
}

/// Validates the FSM edge and the marker/run-id consistency that makes the
/// durable row self-describing.
pub fn validate_transition(
    from: JobState,
    to: JobState,
    marker: DispatchMarker,
    run_id: Option<&str>,
) -> Result<(), ScheduleError> {
    if !transition_allowed(from, to) {
        return Err(ScheduleError::new(
            ScheduleErrorCode::InvalidJobTransition,
            format!(
                "job transition {} -> {} is not allowed",
                from.as_str(),
                to.as_str()
            ),
        ));
    }
    validate_dispatch_invariant(to, marker, run_id)
}

/// The dispatch marker and the recorded run id must agree, and the marker must
/// match the state. A mismatch is a contract violation, not a recoverable
/// condition: recovery branches on this invariant.
pub fn validate_dispatch_invariant(
    state: JobState,
    marker: DispatchMarker,
    run_id: Option<&str>,
) -> Result<(), ScheduleError> {
    let consistent = match state {
        JobState::Queued
        | JobState::Preparing
        | JobState::Prepared
        | JobState::FailedPrecondition
        | JobState::Cancelled => {
            matches!(marker, DispatchMarker::NotDispatched) && run_id.is_none()
        }
        JobState::Dispatching => matches!(
            (marker, run_id),
            (DispatchMarker::IntentRecorded, None) | (DispatchMarker::Confirmed, Some(_))
        ),
        JobState::Running | JobState::Collecting | JobState::Succeeded | JobState::Failed => {
            matches!(marker, DispatchMarker::Confirmed) && run_id.is_some()
        }
        JobState::UnknownManual => {
            matches!(
                (marker, run_id),
                (DispatchMarker::IntentRecorded, None) | (DispatchMarker::Confirmed, Some(_))
            )
        }
    };
    if consistent {
        Ok(())
    } else {
        Err(ScheduleError::new(
            ScheduleErrorCode::InvalidJobState,
            format!(
                "job state '{}' is inconsistent with dispatch marker '{}' (run id {})",
                state.as_str(),
                marker.as_str(),
                match run_id {
                    Some(run_id) => format!("'{run_id}'"),
                    None => "absent".to_string(),
                }
            ),
        ))
    }
}

/// The deterministic restart-recovery outcome for a durable job. This is the
/// only place that decides what a restart may do; every arm that could replay
/// an unknown effect resolves to [`RecoveryDecision::UnknownManual`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryDecision {
    /// Terminal already: keep the recorded outcome, do no work.
    Terminal,
    /// Provably pre-dispatch: a new lease may adopt/compensate and continue.
    Resume,
    /// Read-only authoritative reconcile; may advance to collecting or park.
    Reconcile,
    /// Artifact-only completion; pure reads plus atomic publication.
    Collect,
    /// No automatic action; a human must resolve the ambiguity.
    UnknownManual,
}

impl RecoveryDecision {
    pub fn as_str(&self) -> &'static str {
        match self {
            RecoveryDecision::Terminal => "terminal",
            RecoveryDecision::Resume => "resume",
            RecoveryDecision::Reconcile => "reconcile",
            RecoveryDecision::Collect => "collect",
            RecoveryDecision::UnknownManual => "unknown_manual",
        }
    }
}

/// Classifies restart recovery from typed durable state plus the authoritative
/// run terminality (when a run id exists). `run_terminal` is `None` when no run
/// id is recorded.
pub fn classify_restart_recovery(
    state: JobState,
    marker: DispatchMarker,
    run_id: Option<&str>,
    run_terminal: Option<bool>,
) -> RecoveryDecision {
    if state.is_terminal() {
        return RecoveryDecision::Terminal;
    }
    // A durable row that contradicts its own invariants is never resumed.
    if validate_dispatch_invariant(state, marker, run_id).is_err() {
        return RecoveryDecision::UnknownManual;
    }
    match state {
        JobState::Queued => RecoveryDecision::Resume,
        JobState::Preparing | JobState::Prepared => match marker {
            DispatchMarker::NotDispatched => RecoveryDecision::Resume,
            // Defensive: intent is written only on entering `dispatching`.
            _ => RecoveryDecision::UnknownManual,
        },
        JobState::Dispatching => match (marker, run_id) {
            // Intent recorded but no run id: the run kernel may or may not have
            // executed. Never re-dispatch (INV-5).
            (DispatchMarker::IntentRecorded, None) => RecoveryDecision::UnknownManual,
            (DispatchMarker::Confirmed, Some(_)) => RecoveryDecision::Reconcile,
            _ => RecoveryDecision::UnknownManual,
        },
        JobState::Running => match run_terminal {
            Some(true) => RecoveryDecision::Collect,
            // A run that is not provably terminal is never restarted and never
            // re-dispatched.
            _ => RecoveryDecision::UnknownManual,
        },
        JobState::Collecting => RecoveryDecision::Collect,
        JobState::Succeeded
        | JobState::FailedPrecondition
        | JobState::Failed
        | JobState::Cancelled
        | JobState::UnknownManual => RecoveryDecision::Terminal,
    }
}

/// The ordered saga stages recorded in the job journal. Each stage is written
/// before its side effect completes, so compensation always has a typed
/// starting point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobSagaStage {
    WorkspaceAcquired,
    InputPatchVerified,
    BundleStaged,
    BundleVerified,
    CapabilitiesRegistered,
    EnvironmentReady,
    DispatchIntent,
    WorkflowStarted,
    ArtifactsCollected,
    CleanupDone,
}

impl JobSagaStage {
    pub fn as_str(&self) -> &'static str {
        match self {
            JobSagaStage::WorkspaceAcquired => "workspace_acquired",
            JobSagaStage::InputPatchVerified => "input_patch_verified",
            JobSagaStage::BundleStaged => "bundle_staged",
            JobSagaStage::BundleVerified => "bundle_verified",
            JobSagaStage::CapabilitiesRegistered => "capabilities_registered",
            JobSagaStage::EnvironmentReady => "environment_ready",
            JobSagaStage::DispatchIntent => "dispatch_intent",
            JobSagaStage::WorkflowStarted => "workflow_started",
            JobSagaStage::ArtifactsCollected => "artifacts_collected",
            JobSagaStage::CleanupDone => "cleanup_done",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "workspace_acquired" => JobSagaStage::WorkspaceAcquired,
            "input_patch_verified" => JobSagaStage::InputPatchVerified,
            "bundle_staged" => JobSagaStage::BundleStaged,
            "bundle_verified" => JobSagaStage::BundleVerified,
            "capabilities_registered" => JobSagaStage::CapabilitiesRegistered,
            "environment_ready" => JobSagaStage::EnvironmentReady,
            "dispatch_intent" => JobSagaStage::DispatchIntent,
            "workflow_started" => JobSagaStage::WorkflowStarted,
            "artifacts_collected" => JobSagaStage::ArtifactsCollected,
            "cleanup_done" => JobSagaStage::CleanupDone,
            _ => return None,
        })
    }
}

/// Fenced ownership of a claimed job. Both the owner id and the lease
/// generation must match for any mutation; a stale generation can neither
/// complete, adopt, exec nor clean up (INV-8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerFence {
    pub owner_id: String,
    pub lease_generation: i64,
}

impl OwnerFence {
    pub fn new(owner_id: impl Into<String>, lease_generation: i64) -> Self {
        Self {
            owner_id: owner_id.into(),
            lease_generation,
        }
    }

    /// The opaque ownership token hash embedded in container labels and
    /// workspace markers. It binds job, owner and generation, so a token can
    /// never be replayed by a different owner or generation.
    pub fn token_hash(&self, job_id: &str) -> String {
        canonical_hash(
            OWNER_TOKEN_DOMAIN,
            &serde_json::json!({
                "job_id": job_id,
                "owner_id": self.owner_id,
                "lease_generation": self.lease_generation,
            }),
        )
    }
}

/// Deterministic backend-minted job id. A caller can never supply one.
pub fn job_id_for(campaign_id: &str, candidate_key: &str, task_id: &str, ordinal: u32) -> String {
    let digest = canonical_hash(
        JOB_ID_DOMAIN,
        &serde_json::json!({
            "campaign_id": campaign_id,
            "candidate_key": candidate_key,
            "task_id": task_id,
            "ordinal": ordinal,
        }),
    );
    format!("job-{}", &digest[..32])
}

// ---------------------------------------------------------------------------
// Durable schedule request
// ---------------------------------------------------------------------------

/// Plan-level and schedule-level keys a caller may never inject. Each one
/// would let a caller reach outside the allowlisted declarative surface, mint
/// backend identity, or smuggle a secret/host path into the durable queue.
const FORBIDDEN_SCHEDULE_KEYS: &[&str] = &[
    "campaign_id",
    "campaign_scope_id",
    "candidate_scope_id",
    "trial_scope_id",
    "request_scope_id",
    "scope_id",
    "scope_ids",
    "job_id",
    "job_ids",
    "job_state",
    "dispatch_marker",
    "lease_generation",
    "owner_id",
    "owner_token",
    "budget",
    "budget_override",
    "caps",
    "envelope",
    "verifier",
    "verifier_id",
    "verifier_digest",
    "evaluator",
    "evaluator_dir",
    "verdict",
    "verdict_dir",
    "sandbox",
    "sandbox_config",
    "sandbox_scheme_id",
    "sandbox_execution_mode",
    "allowed_paths",
    "approval_level",
    "shell_policy",
    "auto_approve",
    "path_guard",
    "holdout",
    "private_holdout",
    "promotion",
    "promotion_policy",
    "ledger",
    "instruction",
    "prompt",
    "raw_prompt",
    "system_prompt",
    "planning_prompt",
    "raw_response",
    "transcript",
    "api_key",
    "api_keys",
    "token",
    "tokens",
    "secret",
    "secrets",
    "env",
    "environment",
    "host_path",
    "base_repo_path",
    "worktree_path",
    "artifact_path",
    "image",
    "network_mode",
    "mounts",
    "tools",
    "mcp_tools",
    "skills",
];

fn reject_forbidden_keys(value: &Value, context: &str) -> Result<(), ScheduleError> {
    let Some(map) = value.as_object() else {
        return Err(ScheduleError::new(
            ScheduleErrorCode::UnknownField,
            format!("{context} must be a JSON object"),
        ));
    };
    for key in map.keys() {
        if FORBIDDEN_SCHEDULE_KEYS.contains(&key.as_str()) {
            return Err(ScheduleError::new(
                ScheduleErrorCode::ForbiddenField,
                format!("'{key}' is not an allowed field for {context}"),
            ));
        }
    }
    Ok(())
}

/// The strict, versioned durable campaign schedule request.
///
/// It carries the frozen 2F plan, the digest-bound fixture refs and the
/// server-registered execution profile / bundle refs. It carries no
/// instruction, no path, no scope id, no secret and no promotion field.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct CampaignScheduleRequestV1 {
    pub schema_version: String,
    /// The frozen 2F plan. Its canonical hash is the campaign identity.
    pub plan: CampaignPlanV1,
    /// Digest-bound fixture identity, resolved by the backend at dispatch.
    pub fixture_refs: Vec<FixtureTaskRefV1>,
    /// Server-registered execution profile reference (never a host path).
    pub execution_profile_ref: String,
    /// Server-allowlisted bundle references (never inline tool/skill bodies).
    pub bundle_refs: Vec<String>,
}

/// The digest-bound identity of a durable schedule request. Excludes volatile
/// fields, so the same request always yields the same campaign identity.
pub fn schedule_request_hash(request: &CampaignScheduleRequestV1) -> String {
    canonical_hash(
        SCHEDULE_HASH_DOMAIN,
        &serde_json::json!({
            "plan": request.plan,
            "execution_profile_ref": request.execution_profile_ref,
            "bundle_refs": request.bundle_refs,
        }),
    )
}

/// The campaign identity a durable schedule request maps to.
///
/// The backend derives it from the frozen plan hash, so the path campaign id of
/// a schedule submission is authoritative and a caller can never mint one
/// (INV-2).
pub fn campaign_id_for_schedule(request: &CampaignScheduleRequestV1) -> String {
    campaign_id_for_plan(&request.plan.plan_hash())
}

/// Parses and validates a strict durable schedule request.
///
/// Ordering matters: caller-forbidden keys are rejected first, then the nested
/// 2F plan is parsed by the shared 2F parser (so the 2F machine codes and
/// forbidden-field list keep applying), and only then is the outer document
/// deserialized. No value is persisted before every check passes.
pub fn parse_and_validate_campaign_schedule_request(
    value: &Value,
) -> Result<CampaignScheduleRequestV1, ScheduleError> {
    reject_forbidden_keys(value, "a campaign schedule request")?;

    let plan_value = value.get("plan").cloned().ok_or_else(|| {
        ScheduleError::new(
            ScheduleErrorCode::UnknownField,
            "campaign schedule request is missing the frozen plan",
        )
    })?;
    parse_campaign_plan(&plan_value).map_err(map_plan_error)?;

    let request: CampaignScheduleRequestV1 =
        serde_json::from_value(value.clone()).map_err(|error| {
            ScheduleError::new(
                ScheduleErrorCode::UnknownField,
                format!("campaign schedule request is not a valid strict document: {error}"),
            )
        })?;
    validate_campaign_schedule_request(&request)?;
    Ok(request)
}

/// Semantic validation of an already-deserialized schedule request.
pub fn validate_campaign_schedule_request(
    request: &CampaignScheduleRequestV1,
) -> Result<(), ScheduleError> {
    if request.schema_version != CAMPAIGN_SCHEDULE_V1 {
        return Err(ScheduleError::new(
            ScheduleErrorCode::UnsupportedVersion,
            format!(
                "unsupported campaign schedule schema_version '{}'",
                request.schema_version
            ),
        ));
    }
    let plan = &request.plan;
    if plan.campaign_key.trim().is_empty() {
        return Err(ScheduleError::new(
            ScheduleErrorCode::InvalidCampaignKey,
            "campaign schedule request has an empty campaign key",
        ));
    }
    if plan.agent_id.trim().is_empty() {
        return Err(ScheduleError::new(
            ScheduleErrorCode::InvalidAgentId,
            "campaign schedule request has an empty agent id",
        ));
    }
    if plan.concurrency != SCHEDULE_CONCURRENCY {
        return Err(ScheduleError::new(
            ScheduleErrorCode::InvalidJobState,
            format!("durable schedule requires concurrency {SCHEDULE_CONCURRENCY}"),
        ));
    }
    if request.execution_profile_ref.trim().is_empty() {
        return Err(ScheduleError::new(
            ScheduleErrorCode::UnknownExecutionProfile,
            "campaign schedule request has an empty execution profile reference",
        ));
    }
    // The durable request must bind exactly the fixture the plan declares, and
    // the refs must be internally consistent digests.
    if request.fixture_refs.len() != 1 {
        return Err(ScheduleError::new(
            ScheduleErrorCode::InvalidFixture,
            format!(
                "campaign schedule request must bind exactly one fixture ref, got {}",
                request.fixture_refs.len()
            ),
        ));
    }
    let fixture = &request.fixture_refs[0];
    if fixture.suite != plan.suite || fixture.task_id != plan.task {
        return Err(ScheduleError::new(
            ScheduleErrorCode::InvalidFixture,
            format!(
                "fixture ref '{}/{}' does not match the plan's '{}/{}'",
                fixture.suite, fixture.task_id, plan.suite, plan.task
            ),
        ));
    }
    for (field, value) in [
        ("manifest_digest", &fixture.manifest_digest),
        ("task_digest", &fixture.task_digest),
        ("instruction_hash", &fixture.instruction_hash),
    ] {
        if !is_sha256_hex(value) {
            return Err(ScheduleError::new(
                ScheduleErrorCode::InvalidFixture,
                format!("fixture ref {field} is not a sha256 hex digest"),
            ));
        }
    }
    for bundle_ref in &request.bundle_refs {
        if !is_valid_key(bundle_ref) {
            return Err(ScheduleError::new(
                ScheduleErrorCode::BundleRefUnknown,
                format!("bundle ref '{bundle_ref}' is not a valid allowlist key"),
            ));
        }
    }
    let mut sorted = request.bundle_refs.clone();
    sorted.sort();
    let before = sorted.len();
    sorted.dedup();
    if sorted.len() != before {
        return Err(ScheduleError::new(
            ScheduleErrorCode::BundleRefUnknown,
            "campaign schedule request declares a duplicate bundle ref",
        ));
    }
    Ok(())
}

/// The accepted projection returned by `POST /control/v1/campaigns/{id}/schedule`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct CampaignScheduleAcceptedV1 {
    pub schema_version: String,
    pub campaign_id: String,
    pub campaign_key: String,
    pub plan_hash: String,
    pub schedule_hash: String,
    pub execution_profile_ref: String,
    pub job_ids: Vec<String>,
    pub concurrency: u32,
}

impl CampaignScheduleAcceptedV1 {
    pub fn new(
        campaign_id: impl Into<String>,
        campaign_key: impl Into<String>,
        plan_hash: impl Into<String>,
        schedule_hash: impl Into<String>,
        execution_profile_ref: impl Into<String>,
        job_ids: Vec<String>,
    ) -> Self {
        Self {
            schema_version: CAMPAIGN_SCHEDULE_ACCEPTED_V1.to_string(),
            campaign_id: campaign_id.into(),
            campaign_key: campaign_key.into(),
            plan_hash: plan_hash.into(),
            schedule_hash: schedule_hash.into(),
            execution_profile_ref: execution_profile_ref.into(),
            job_ids,
            concurrency: SCHEDULE_CONCURRENCY,
        }
    }
}

/// The strict, instruction-free projection of one durable job.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct CampaignJobV1 {
    pub schema_version: String,
    pub job_id: String,
    pub campaign_id: String,
    pub ordinal: u32,
    pub candidate_key: String,
    pub task_id: String,
    pub suite: String,
    pub dataset_id: String,
    pub dataset_version: u32,
    pub split: String,
    pub manifest_digest: String,
    pub task_digest: String,
    pub instruction_hash: String,
    pub execution_profile_ref: String,
    pub state: JobState,
    pub dispatch_marker: DispatchMarker,
    #[serde(default)]
    pub run_id: Option<String>,
    pub attempt: u32,
    #[serde(default)]
    pub lease_generation: i64,
    #[serde(default)]
    pub last_stage: Option<String>,
    #[serde(default)]
    pub error_code: Option<String>,
}

/// The durable job list of one campaign.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct CampaignJobListV1 {
    pub schema_version: String,
    pub campaign_id: String,
    pub jobs: Vec<CampaignJobV1>,
}

/// The result of cancelling a campaign's pre-dispatch work.
///
/// Already-dispatched jobs are listed separately and are never cancelled here:
/// their effect may have happened, so they are stopped through the run kernel
/// or parked by reconciliation (INV-5).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct CampaignCancelV1 {
    pub schema_version: String,
    pub campaign_id: String,
    pub status: String,
    pub cancelled_job_ids: Vec<String>,
    pub dispatched_job_ids: Vec<String>,
}

/// The evidence-based classification of one non-terminal job.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct JobRecoveryProjectionV1 {
    pub job_id: String,
    pub state: JobState,
    pub dispatch_marker: DispatchMarker,
    #[serde(default)]
    pub run_id: Option<String>,
    /// Authoritative terminality of the recorded run, when a run id exists.
    #[serde(default)]
    pub run_terminal: Option<bool>,
    pub decision: RecoveryDecision,
    /// Whether this reconcile pass parked the job as `unknown_manual`.
    pub parked: bool,
}

/// The evidence-only reconcile result. It never requeues and never calls the
/// run kernel; it reports what the durable state plus the run authority imply.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct CampaignReconcileV1 {
    pub schema_version: String,
    pub campaign_id: String,
    pub jobs: Vec<JobRecoveryProjectionV1>,
}

// ---------------------------------------------------------------------------
// Isolation manifests (server-registered; never caller-supplied)
// ---------------------------------------------------------------------------

/// Who owns the execution environment for a run. There is no silent fallback
/// between these: a profile names exactly one, and failure to satisfy it is a
/// pre-dispatch `failed_precondition` (INV-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnerKind {
    /// Filesystem-only worktree owner: no host shell execution authority.
    HostWorktree,
    /// A persistent, label-fenced Docker container.
    PersistentDocker,
    /// The current Harbor task environment, proven by a capability manifest.
    HarborTask,
}

impl OwnerKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            OwnerKind::HostWorktree => "host_worktree",
            OwnerKind::PersistentDocker => "persistent_docker",
            OwnerKind::HarborTask => "harbor_task",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "host_worktree" => OwnerKind::HostWorktree,
            "persistent_docker" => OwnerKind::PersistentDocker,
            "harbor_task" => OwnerKind::HarborTask,
            _ => return None,
        })
    }

    /// Only owners that provide a real, fenced execution environment may
    /// dispatch a run that executes shell/tool actions.
    pub fn provides_execution_isolation(&self) -> bool {
        matches!(self, OwnerKind::PersistentDocker | OwnerKind::HarborTask)
    }
}

/// Container network policy. Requesting a policy the runtime cannot enforce is
/// a pre-dispatch failure, never a silent downgrade.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct NetworkPolicyV1 {
    /// `none` or `egress_allowlist`.
    pub mode: String,
    #[serde(default)]
    pub allow_hosts: Vec<String>,
}

impl NetworkPolicyV1 {
    pub const MODE_NONE: &'static str = "none";
    pub const MODE_EGRESS_ALLOWLIST: &'static str = "egress_allowlist";

    pub fn is_known_mode(&self) -> bool {
        matches!(
            self.mode.as_str(),
            Self::MODE_NONE | Self::MODE_EGRESS_ALLOWLIST
        )
    }
}

/// A mount the owner may create. Only the run workspace and verified bundles
/// can be mounted, so an execution profile can never expose an arbitrary host
/// path to the run (INV-4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct MountSpecV1 {
    /// `workspace` or `bundle`.
    pub source_kind: String,
    /// Container-relative destination, always absolute inside the container.
    pub container_path: String,
    pub read_only: bool,
}

impl MountSpecV1 {
    pub const SOURCE_WORKSPACE: &'static str = "workspace";
    pub const SOURCE_BUNDLE: &'static str = "bundle";

    pub fn is_known_source(&self) -> bool {
        matches!(
            self.source_kind.as_str(),
            Self::SOURCE_WORKSPACE | Self::SOURCE_BUNDLE
        )
    }
}

/// Resource caps applied by the owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct ResourceLimitsV1 {
    pub cpu_millis: u64,
    pub memory_bytes: u64,
    pub pids: u64,
    pub no_new_privileges: bool,
}

/// A server-registered execution profile. It is the *only* place a base repo,
/// image digest, network policy or mount allowlist is declared; a schedule
/// request merely references it by name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct ExecutionProfileV1 {
    pub schema_version: String,
    pub profile_ref: String,
    pub owner_kind: OwnerKind,
    /// Opaque server-side repository reference resolved by the backend.
    pub base_repo_ref: String,
    /// Immutable base revision the run worktree is created from.
    pub base_revision: String,
    /// Digest-pinned container image (`repo@sha256:...`); required for the
    /// `persistent_docker` owner and rejected if absent.
    #[serde(default)]
    pub image_reference: Option<String>,
    #[serde(default)]
    pub network_policy: Option<NetworkPolicyV1>,
    #[serde(default)]
    pub mounts: Vec<MountSpecV1>,
    pub resources: ResourceLimitsV1,
    /// Bundle refs this profile permits a job to stage.
    #[serde(default)]
    pub allowed_bundle_refs: Vec<String>,
    /// Opaque server-side reference to the reviewed input patch, if any.
    #[serde(default)]
    pub input_patch_ref: Option<String>,
    /// Digest of the input patch; required whenever `input_patch_ref` is set.
    #[serde(default)]
    pub input_patch_digest: Option<String>,
}

impl ExecutionProfileV1 {
    /// Validates the profile in isolation. Every failure is a pre-dispatch
    /// fail-closed rejection (AC-3/INV-4).
    pub fn validate(&self) -> Result<(), ScheduleError> {
        if self.schema_version != EXECUTION_PROFILE_V1 {
            return Err(ScheduleError::new(
                ScheduleErrorCode::InvalidExecutionProfile,
                format!(
                    "unsupported execution profile schema_version '{}'",
                    self.schema_version
                ),
            ));
        }
        if !is_valid_key(&self.profile_ref) {
            return Err(ScheduleError::new(
                ScheduleErrorCode::InvalidExecutionProfile,
                "execution profile has an invalid profile_ref",
            ));
        }
        if self.base_repo_ref.trim().is_empty() || self.base_revision.trim().is_empty() {
            return Err(ScheduleError::new(
                ScheduleErrorCode::InvalidExecutionProfile,
                "execution profile must declare a base repo ref and revision",
            ));
        }
        if let Some(policy) = &self.network_policy {
            if !policy.is_known_mode() {
                return Err(ScheduleError::new(
                    ScheduleErrorCode::NetworkPolicyUnsupported,
                    format!("unsupported network policy mode '{}'", policy.mode),
                ));
            }
        }
        let mut workspace_mounts = 0usize;
        let mut bundle_mounts = 0usize;
        for mount in &self.mounts {
            if !mount.is_known_source() {
                return Err(ScheduleError::new(
                    ScheduleErrorCode::InvalidExecutionProfile,
                    format!(
                        "mount source kind '{}' is not allowed (workspace or bundle only)",
                        mount.source_kind
                    ),
                ));
            }
            if !mount.container_path.starts_with('/') {
                return Err(ScheduleError::new(
                    ScheduleErrorCode::InvalidExecutionProfile,
                    format!(
                        "mount container path '{}' must be absolute",
                        mount.container_path
                    ),
                ));
            }
            match mount.source_kind.as_str() {
                MountSpecV1::SOURCE_WORKSPACE => workspace_mounts += 1,
                MountSpecV1::SOURCE_BUNDLE if mount.read_only => bundle_mounts += 1,
                MountSpecV1::SOURCE_BUNDLE => {
                    return Err(ScheduleError::new(
                        ScheduleErrorCode::InvalidExecutionProfile,
                        "a bundle mount must be read-only",
                    ));
                }
                _ => unreachable!("known mount sources were checked above"),
            }
        }
        if workspace_mounts > 1 || bundle_mounts > 1 {
            return Err(ScheduleError::new(
                ScheduleErrorCode::InvalidExecutionProfile,
                "an execution profile may declare at most one workspace mount and one read-only bundle mount",
            ));
        }
        if (self.input_patch_ref.is_some()) != (self.input_patch_digest.is_some()) {
            return Err(ScheduleError::new(
                ScheduleErrorCode::InvalidExecutionProfile,
                "input patch ref and digest must be declared together",
            ));
        }
        if let Some(digest) = &self.input_patch_digest {
            if !is_sha256_hex(digest) {
                return Err(ScheduleError::new(
                    ScheduleErrorCode::InvalidExecutionProfile,
                    "input patch digest is not a sha256 hex digest",
                ));
            }
        }
        match self.owner_kind {
            OwnerKind::PersistentDocker => {
                let image = self.image_reference.as_deref().ok_or_else(|| {
                    ScheduleError::new(
                        ScheduleErrorCode::InvalidExecutionProfile,
                        "persistent_docker execution profile requires an image reference",
                    )
                })?;
                if !is_digest_pinned_image(image) {
                    return Err(ScheduleError::new(
                        ScheduleErrorCode::ImageNotDigestPinned,
                        format!("image reference '{image}' is not pinned by sha256 digest"),
                    ));
                }
            }
            OwnerKind::HostWorktree | OwnerKind::HarborTask => {}
        }
        Ok(())
    }

    /// Canonical identity of the profile, excluding nothing: the whole document
    /// is server-registered and immutable once loaded.
    pub fn profile_hash(&self) -> String {
        canonical_hash(
            EXECUTION_PROFILE_HASH_DOMAIN,
            &serde_json::to_value(self).unwrap_or(Value::Null),
        )
    }
}

/// One file inside a bundle. Only relative, non-escaping paths are valid, and
/// a symlink is expressed as an explicit `symlink_target` rather than a silent
/// filesystem property, so staging can reject it up front.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct BundleFileV1 {
    pub relative_path: String,
    pub size_bytes: u64,
    pub sha256: String,
    #[serde(default = "default_file_mode")]
    pub mode: u32,
    #[serde(default)]
    pub executable: bool,
    #[serde(default)]
    pub symlink_target: Option<String>,
}

fn default_file_mode() -> u32 {
    0o644
}

/// An MCP server declared by a bundle. `command` must be a bundle-relative
/// program; secrets are referenced by name only and resolved at run time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct BundleMcpServerV1 {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env_secret_refs: Vec<String>,
    /// Secret references whose values are injected as environment variables.
    #[serde(default)]
    pub env_secret_env_names: Vec<String>,
}

/// A skill declared by a bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct BundleSkillV1 {
    pub name: String,
    pub entry_path: String,
}

/// The strict bundle manifest. It declares identity, content digests,
/// permissions and capability declarations — never a secret value, never a
/// URL, never a remote install step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct BundleManifestV1 {
    pub schema_version: String,
    pub bundle_ref: String,
    pub bundle_version: String,
    pub content_digest: String,
    pub files: Vec<BundleFileV1>,
    #[serde(default)]
    pub mcp_servers: Vec<BundleMcpServerV1>,
    #[serde(default)]
    pub skills: Vec<BundleSkillV1>,
    /// Names of secrets the run may resolve from its restricted credential
    /// input. Values never appear in the manifest or in any artifact.
    #[serde(default)]
    pub env_secret_refs: Vec<String>,
}

impl BundleManifestV1 {
    /// The canonical content digest over the declared file list. Staging
    /// recomputes it and refuses to register a bundle whose bytes differ.
    pub fn computed_content_digest(&self) -> String {
        let mut files: Vec<&BundleFileV1> = self.files.iter().collect();
        files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        let entries: Vec<Value> = files
            .iter()
            .map(|file| {
                serde_json::json!({
                    "relative_path": file.relative_path,
                    "size_bytes": file.size_bytes,
                    "sha256": file.sha256,
                    "mode": file.mode,
                    "executable": file.executable,
                    "symlink_target": file.symlink_target,
                })
            })
            .collect();
        canonical_hash(
            BUNDLE_CONTENT_HASH_DOMAIN,
            &serde_json::json!({ "bundle_ref": self.bundle_ref, "files": entries }),
        )
    }

    /// Full structural validation, including path safety and the absence of
    /// any secret *value* or remote fetch instruction.
    pub fn validate(&self) -> Result<(), ScheduleError> {
        if self.schema_version != BUNDLE_MANIFEST_V1 {
            return Err(ScheduleError::new(
                ScheduleErrorCode::BundleManifestInvalid,
                format!(
                    "unsupported bundle manifest schema_version '{}'",
                    self.schema_version
                ),
            ));
        }
        if !is_valid_key(&self.bundle_ref) {
            return Err(ScheduleError::new(
                ScheduleErrorCode::BundleManifestInvalid,
                "bundle manifest has an invalid bundle_ref",
            ));
        }
        if self.bundle_version.trim().is_empty() {
            return Err(ScheduleError::new(
                ScheduleErrorCode::BundleManifestInvalid,
                "bundle manifest has an empty bundle_version",
            ));
        }
        if self.files.is_empty() {
            return Err(ScheduleError::new(
                ScheduleErrorCode::BundleManifestInvalid,
                "bundle manifest declares no files",
            ));
        }
        let mut seen: Vec<&str> = Vec::with_capacity(self.files.len());
        for file in &self.files {
            validate_bundle_relative_path(&file.relative_path)?;
            if !is_sha256_hex(&file.sha256) {
                return Err(ScheduleError::new(
                    ScheduleErrorCode::BundleManifestInvalid,
                    format!(
                        "bundle file '{}' has a malformed sha256",
                        file.relative_path
                    ),
                ));
            }
            if let Some(target) = &file.symlink_target {
                if target.trim().is_empty() {
                    return Err(ScheduleError::new(
                        ScheduleErrorCode::BundlePathUnsafe,
                        format!(
                            "bundle file '{}' declares an empty symlink target",
                            file.relative_path
                        ),
                    ));
                }
                validate_bundle_relative_path(target)?;
            }
            if file.mode & 0o7777 != file.mode {
                return Err(ScheduleError::new(
                    ScheduleErrorCode::BundleManifestInvalid,
                    format!(
                        "bundle file '{}' has an out-of-range mode",
                        file.relative_path
                    ),
                ));
            }
            if seen.contains(&file.relative_path.as_str()) {
                return Err(ScheduleError::new(
                    ScheduleErrorCode::BundleManifestInvalid,
                    format!("bundle declares '{}' twice", file.relative_path),
                ));
            }
            seen.push(&file.relative_path);
        }
        for server in &self.mcp_servers {
            if !is_valid_key(&server.name) {
                return Err(ScheduleError::new(
                    ScheduleErrorCode::BundleManifestInvalid,
                    "bundle MCP server has an invalid name",
                ));
            }
            // A bundle program is never a remote fetch or a shell pipeline.
            if !server.command.starts_with("./") {
                return Err(ScheduleError::new(
                    ScheduleErrorCode::BundleNotVerifiable,
                    format!(
                        "bundle MCP server '{}' must use a bundle-relative program, got '{}'",
                        server.name, server.command
                    ),
                ));
            }
            if server.env_secret_refs.len() != server.env_secret_env_names.len() {
                return Err(ScheduleError::new(
                    ScheduleErrorCode::BundleManifestInvalid,
                    format!(
                        "bundle MCP server '{}' must pair every env secret ref with an env name",
                        server.name
                    ),
                ));
            }
            for value in server.args.iter().chain(server.env_secret_refs.iter()) {
                if looks_like_secret_value(value) {
                    return Err(ScheduleError::new(
                        ScheduleErrorCode::BundleSecretForbidden,
                        format!(
                            "bundle MCP server '{}' appears to declare a secret value",
                            server.name
                        ),
                    ));
                }
                if looks_like_remote_fetch(value) {
                    return Err(ScheduleError::new(
                        ScheduleErrorCode::BundleNotVerifiable,
                        format!(
                            "bundle MCP server '{}' appears to fetch from a remote location",
                            server.name
                        ),
                    ));
                }
            }
        }
        for skill in &self.skills {
            if !is_valid_key(&skill.name) {
                return Err(ScheduleError::new(
                    ScheduleErrorCode::BundleManifestInvalid,
                    "bundle skill has an invalid name",
                ));
            }
            validate_bundle_relative_path(&skill.entry_path)?;
        }
        for reference in &self.env_secret_refs {
            if !is_valid_key(reference) {
                return Err(ScheduleError::new(
                    ScheduleErrorCode::BundleSecretForbidden,
                    format!("bundle declares an invalid secret ref '{reference}'"),
                ));
            }
        }
        if !is_sha256_hex(&self.content_digest)
            || self.content_digest != self.computed_content_digest()
        {
            return Err(ScheduleError::new(
                ScheduleErrorCode::BundleDigestMismatch,
                "bundle manifest content_digest does not match its declared files",
            ));
        }
        Ok(())
    }
}

fn validate_bundle_relative_path(path: &str) -> Result<(), ScheduleError> {
    let unsafe_path = path.is_empty()
        || path.starts_with('/')
        || path.starts_with('\\')
        || path.contains('\\')
        || path.contains('\0')
        || path.split('/').any(|segment| {
            segment.is_empty() || segment == "." || segment == ".." || segment.contains(':')
        });
    if unsafe_path {
        return Err(ScheduleError::new(
            ScheduleErrorCode::BundlePathUnsafe,
            format!("bundle path '{path}' is absolute, escaping or otherwise unsafe"),
        ));
    }
    Ok(())
}

/// Heuristic rejection of a declared secret value (as opposed to a ref name).
/// It is deliberately over-inclusive: a bundle that *might* carry a secret
/// value is refused rather than registered and patched up later (INV-6).
fn looks_like_secret_value(value: &str) -> bool {
    let lowered = value.to_ascii_lowercase();
    let looks_like_token = ["sk-", "ghp_", "xoxb-", "bearer ", "authorization:"]
        .iter()
        .any(|marker| lowered.contains(marker));
    looks_like_token
        || value.contains("-----BEGIN")
        || (value.len() >= 32 && value.chars().all(|c| c.is_ascii_hexdigit()))
}

/// Heuristic rejection of a remote fetch instruction inside a bundle.
fn looks_like_remote_fetch(value: &str) -> bool {
    let lowered = value.to_ascii_lowercase();
    lowered.contains("http://")
        || lowered.contains("https://")
        || lowered.contains("git+")
        || lowered.contains("curl ")
        || lowered.contains("wget ")
}

/// The capability manifest a `HarborTaskOwner` adapter writes inside the task
/// environment. It is the *proof* that the current environment is an owned
/// Harbor sandbox, so no manifest means no owner — and never a host fallback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct HarborTaskCapabilityV1 {
    pub schema_version: String,
    pub task_id: String,
    pub nonce: String,
    pub owner_token_hash: String,
    pub task_root: String,
    pub workspace_root: String,
    pub artifact_roots: Vec<String>,
    pub network_policy: NetworkPolicyV1,
    /// Roots the run may read but must never write.
    #[serde(default)]
    pub read_only_roots: Vec<String>,
    /// Excluded from the capability hash so a manifest stays verifiable.
    pub issued_at: String,
}

impl HarborTaskCapabilityV1 {
    /// Canonical identity of the capability proof, excluding `issued_at`.
    pub fn capability_hash(&self) -> String {
        canonical_hash(
            CAPABILITY_HASH_DOMAIN,
            &serde_json::json!({
                "schema_version": self.schema_version,
                "task_id": self.task_id,
                "nonce": self.nonce,
                "owner_token_hash": self.owner_token_hash,
                "task_root": self.task_root,
                "workspace_root": self.workspace_root,
                "artifact_roots": self.artifact_roots,
                "network_policy": self.network_policy,
                "read_only_roots": self.read_only_roots,
            }),
        )
    }

    /// Validates the proof shape. A malformed capability is never accepted and
    /// never downgraded to host execution.
    pub fn validate(&self) -> Result<(), ScheduleError> {
        if self.schema_version != HARBOR_TASK_CAPABILITY_V1 {
            return Err(ScheduleError::new(
                ScheduleErrorCode::UnsupportedOwnerKind,
                format!(
                    "unsupported harbor capability schema_version '{}'",
                    self.schema_version
                ),
            ));
        }
        if self.task_id.trim().is_empty() || self.nonce.trim().is_empty() {
            return Err(ScheduleError::new(
                ScheduleErrorCode::OwnershipMismatch,
                "harbor capability is missing its task id or nonce",
            ));
        }
        if !is_sha256_hex(&self.owner_token_hash) {
            return Err(ScheduleError::new(
                ScheduleErrorCode::OwnershipMismatch,
                "harbor capability owner token hash is malformed",
            ));
        }
        if !self.task_root.starts_with('/') || !self.workspace_root.starts_with('/') {
            return Err(ScheduleError::new(
                ScheduleErrorCode::OwnershipMismatch,
                "harbor capability roots must be absolute container paths",
            ));
        }
        if self.artifact_roots.is_empty() {
            return Err(ScheduleError::new(
                ScheduleErrorCode::OwnershipMismatch,
                "harbor capability declares no artifact root",
            ));
        }
        for root in self
            .artifact_roots
            .iter()
            .chain(self.read_only_roots.iter())
        {
            if !root.starts_with('/') {
                return Err(ScheduleError::new(
                    ScheduleErrorCode::OwnershipMismatch,
                    format!("harbor capability root '{root}' must be absolute"),
                ));
            }
        }
        if !self.network_policy.is_known_mode() {
            return Err(ScheduleError::new(
                ScheduleErrorCode::NetworkPolicyUnsupported,
                format!(
                    "unsupported harbor network policy mode '{}'",
                    self.network_policy.mode
                ),
            ));
        }
        Ok(())
    }

    /// Confirms that a path the owner is about to use really lives inside a
    /// declared, owned root.
    pub fn contains_owned_path(&self, path: &str) -> bool {
        let roots = std::iter::once(&self.workspace_root)
            .chain(self.artifact_roots.iter())
            .chain(self.read_only_roots.iter());
        roots.into_iter().any(|root| {
            let trimmed = root.trim_end_matches('/');
            path == trimmed || path.starts_with(&format!("{trimmed}/"))
        })
    }
}

/// The on-disk experiment-domain marker. A database is adopted by
/// `chatspeed-headless` only when this marker is present (AC-1/INV-9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct ExperimentDomainMarkerV1 {
    pub schema_version: String,
    pub domain_kind: String,
    pub domain_id: String,
    pub created_at: String,
}

impl ExperimentDomainMarkerV1 {
    pub fn new(domain_id: impl Into<String>, created_at: impl Into<String>) -> Self {
        Self {
            schema_version: EXPERIMENT_DOMAIN_MARKER_V1.to_string(),
            domain_kind: DOMAIN_KIND_EXPERIMENT_V1.to_string(),
            domain_id: domain_id.into(),
            created_at: created_at.into(),
        }
    }

    pub fn is_experiment_v1(&self) -> bool {
        self.schema_version == EXPERIMENT_DOMAIN_MARKER_V1
            && self.domain_kind == DOMAIN_KIND_EXPERIMENT_V1
    }
}

// ---------------------------------------------------------------------------
// Small shared validators
// ---------------------------------------------------------------------------

/// Matches the 2F key alphabet so keys stay comparable across contracts.
pub fn is_valid_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// A lowercase hex sha256 digest.
pub fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

/// An image reference pinned by an immutable content digest.
///
/// Two forms are accepted, and both are content-addressed:
///
/// - `repo[:tag]@sha256:<64 hex>` — a registry digest, which stays valid across
///   machines, and
/// - `sha256:<64 hex>` — a local image id, which is how an image that was built
///   or loaded locally (and therefore has no `RepoDigests`) is pinned.
///
/// A mutable tag alone is never accepted (AC-3/INV-4).
pub fn is_digest_pinned_image(value: &str) -> bool {
    if let Some(digest) = value.strip_prefix("sha256:") {
        return is_sha256_hex(digest);
    }
    let Some((repository, digest)) = value.rsplit_once("@sha256:") else {
        return false;
    };
    !repository.trim().is_empty() && is_sha256_hex(digest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::react::experiment_schedule::fixture;

    fn valid_profile(owner_kind: OwnerKind) -> ExecutionProfileV1 {
        ExecutionProfileV1 {
            schema_version: EXECUTION_PROFILE_V1.to_string(),
            profile_ref: "smoke-local".to_string(),
            owner_kind,
            base_repo_ref: "repo:primary".to_string(),
            base_revision: "refs/heads/main".to_string(),
            image_reference: match owner_kind {
                OwnerKind::PersistentDocker => {
                    Some(format!("chatspeed/runner@sha256:{}", "a".repeat(64)))
                }
                _ => None,
            },
            network_policy: Some(NetworkPolicyV1 {
                mode: NetworkPolicyV1::MODE_NONE.to_string(),
                allow_hosts: Vec::new(),
            }),
            mounts: vec![
                MountSpecV1 {
                    source_kind: MountSpecV1::SOURCE_WORKSPACE.to_string(),
                    container_path: "/workspace".to_string(),
                    read_only: false,
                },
                MountSpecV1 {
                    source_kind: MountSpecV1::SOURCE_BUNDLE.to_string(),
                    container_path: "/opt/bundle".to_string(),
                    read_only: true,
                },
            ],
            resources: ResourceLimitsV1 {
                cpu_millis: 1000,
                memory_bytes: 1 << 30,
                pids: 256,
                no_new_privileges: true,
            },
            allowed_bundle_refs: vec!["smoke-tools".to_string()],
            input_patch_ref: None,
            input_patch_digest: None,
        }
    }

    fn sample_schedule_value() -> Value {
        let resolved = fixture::resolve_task(fixture::SUITE_ID, "smoke_reply_ok").expect("fixture");
        serde_json::json!({
            "schema_version": CAMPAIGN_SCHEDULE_V1,
            "plan": {
                "schema_version": "campaign_plan.v1",
                "campaign_key": "p2gh-smoke",
                "stage": "stage_0_manual",
                "agent_id": "agent-1",
                "suite": "chatspeed-smoke",
                "task": "smoke_reply_ok",
                "model": "cs@free:ds-v4-flash",
                "concurrency": 1,
                "budget": {
                    "money_mode": { "mode": "token_resource_only" },
                    "caps": {
                        "input_tokens": 65536,
                        "output_tokens": 128000,
                        "wall_time_ms": 300000,
                        "tool_calls": 0,
                        "processes": 0,
                        "concurrency": 1
                    },
                    "required_dimensions": [],
                    "max_attempts": 1
                },
                "candidates": [
                    { "candidate_key": "baseline", "kind": "baseline" }
                ]
            },
            "fixture_refs": [serde_json::to_value(resolved.task_ref()).expect("ref")],
            "execution_profile_ref": "smoke-local",
            "bundle_refs": ["smoke-tools"]
        })
    }

    #[test]
    fn terminal_and_pre_dispatch_classification_is_stable() {
        for state in [
            JobState::Succeeded,
            JobState::FailedPrecondition,
            JobState::Failed,
            JobState::Cancelled,
            JobState::UnknownManual,
        ] {
            assert!(state.is_terminal());
            assert!(!state.is_pre_dispatch());
            assert!(!state.is_requeueable());
        }
        for state in [JobState::Queued, JobState::Preparing, JobState::Prepared] {
            assert!(state.is_pre_dispatch());
            assert!(!state.is_terminal());
        }
        assert!(!JobState::Dispatching.is_pre_dispatch());
        assert!(JobState::Preparing.is_requeueable());
        assert!(JobState::Prepared.is_requeueable());
        assert!(!JobState::Queued.is_requeueable());
    }

    #[test]
    fn job_state_round_trips_through_strings() {
        for state in [
            JobState::Queued,
            JobState::Preparing,
            JobState::Prepared,
            JobState::Dispatching,
            JobState::Running,
            JobState::Collecting,
            JobState::Succeeded,
            JobState::FailedPrecondition,
            JobState::Failed,
            JobState::Cancelled,
            JobState::UnknownManual,
        ] {
            assert_eq!(JobState::parse(state.as_str()), Some(state));
            let encoded = serde_json::to_value(state).expect("serializes");
            assert_eq!(encoded, Value::String(state.as_str().to_string()));
            let decoded: JobState = serde_json::from_value(encoded).expect("deserializes");
            assert_eq!(decoded, state);
        }
        assert_eq!(JobState::parse("not_a_state"), None);
    }

    #[test]
    fn dispatch_marker_round_trips_through_strings() {
        for marker in [
            DispatchMarker::NotDispatched,
            DispatchMarker::IntentRecorded,
            DispatchMarker::Confirmed,
        ] {
            assert_eq!(DispatchMarker::parse(marker.as_str()), Some(marker));
        }
        assert_eq!(DispatchMarker::parse("maybe"), None);
    }

    #[test]
    fn happy_path_transitions_are_allowed() {
        let edges = [
            (JobState::Queued, JobState::Preparing),
            (JobState::Preparing, JobState::Prepared),
            (JobState::Prepared, JobState::Dispatching),
            (JobState::Dispatching, JobState::Running),
            (JobState::Running, JobState::Collecting),
            (JobState::Collecting, JobState::Succeeded),
        ];
        for (from, to) in edges {
            assert!(transition_allowed(from, to), "{from:?} -> {to:?}");
        }
    }

    #[test]
    fn illegal_transitions_are_rejected() {
        let edges = [
            (JobState::Queued, JobState::Dispatching),
            (JobState::Queued, JobState::Succeeded),
            (JobState::Prepared, JobState::Running),
            (JobState::Dispatching, JobState::Prepared),
            (JobState::Succeeded, JobState::Running),
            (JobState::Running, JobState::Queued),
            (JobState::Collecting, JobState::Queued),
        ];
        for (from, to) in edges {
            assert!(!transition_allowed(from, to), "{from:?} -> {to:?}");
        }
        // A terminal state may only be re-written to itself.
        assert!(transition_allowed(JobState::Succeeded, JobState::Succeeded));
        assert!(!transition_allowed(JobState::Queued, JobState::Queued));
    }

    #[test]
    fn transition_validates_the_dispatch_invariant() {
        // Entering `dispatching` must record the intent with no run id yet.
        let error = validate_transition(
            JobState::Prepared,
            JobState::Dispatching,
            DispatchMarker::NotDispatched,
            None,
        )
        .expect_err("marker must be recorded");
        assert_eq!(error.code, ScheduleErrorCode::InvalidJobState);

        validate_transition(
            JobState::Prepared,
            JobState::Dispatching,
            DispatchMarker::IntentRecorded,
            None,
        )
        .expect("intent recorded");

        validate_transition(
            JobState::Dispatching,
            JobState::Running,
            DispatchMarker::Confirmed,
            Some("run-1"),
        )
        .expect("confirmed");

        // `running` without a run id is never valid.
        let error = validate_transition(
            JobState::Dispatching,
            JobState::Running,
            DispatchMarker::IntentRecorded,
            None,
        )
        .expect_err("run id required");
        assert_eq!(error.code, ScheduleErrorCode::InvalidJobState);
    }

    #[test]
    fn restart_recovery_never_replays_an_unknown_effect() {
        // Provably pre-dispatch work is resumable.
        assert_eq!(
            classify_restart_recovery(JobState::Queued, DispatchMarker::NotDispatched, None, None),
            RecoveryDecision::Resume
        );
        assert_eq!(
            classify_restart_recovery(
                JobState::Prepared,
                DispatchMarker::NotDispatched,
                None,
                None
            ),
            RecoveryDecision::Resume
        );
        // Intent recorded, no run id: parked for a human, never re-dispatched.
        assert_eq!(
            classify_restart_recovery(
                JobState::Dispatching,
                DispatchMarker::IntentRecorded,
                None,
                None
            ),
            RecoveryDecision::UnknownManual
        );
        // Confirmed run id: read-only reconcile only.
        assert_eq!(
            classify_restart_recovery(
                JobState::Dispatching,
                DispatchMarker::Confirmed,
                Some("run-1"),
                None
            ),
            RecoveryDecision::Reconcile
        );
        // A confirmed run that is provably terminal may be collected.
        assert_eq!(
            classify_restart_recovery(
                JobState::Running,
                DispatchMarker::Confirmed,
                Some("run-1"),
                Some(true)
            ),
            RecoveryDecision::Collect
        );
        // A confirmed run that is not provably terminal is parked.
        assert_eq!(
            classify_restart_recovery(
                JobState::Running,
                DispatchMarker::Confirmed,
                Some("run-1"),
                Some(false)
            ),
            RecoveryDecision::UnknownManual
        );
        assert_eq!(
            classify_restart_recovery(
                JobState::Running,
                DispatchMarker::Confirmed,
                Some("run-1"),
                None
            ),
            RecoveryDecision::UnknownManual
        );
        // Collection is pure reads plus atomic publication: idempotent.
        assert_eq!(
            classify_restart_recovery(
                JobState::Collecting,
                DispatchMarker::Confirmed,
                Some("run-1"),
                Some(true)
            ),
            RecoveryDecision::Collect
        );
        // Terminal outcomes stay terminal.
        assert_eq!(
            classify_restart_recovery(
                JobState::UnknownManual,
                DispatchMarker::IntentRecorded,
                None,
                None
            ),
            RecoveryDecision::Terminal
        );
        assert_eq!(
            classify_restart_recovery(
                JobState::Succeeded,
                DispatchMarker::Confirmed,
                Some("run-1"),
                Some(true)
            ),
            RecoveryDecision::Terminal
        );
    }

    #[test]
    fn restart_recovery_fails_closed_on_a_corrupt_row() {
        // `queued` with a recorded intent is internally inconsistent.
        assert_eq!(
            classify_restart_recovery(JobState::Queued, DispatchMarker::IntentRecorded, None, None),
            RecoveryDecision::UnknownManual
        );
        // `prepared` with an intent must not exist, and must not resume.
        assert_eq!(
            classify_restart_recovery(
                JobState::Prepared,
                DispatchMarker::IntentRecorded,
                None,
                None
            ),
            RecoveryDecision::UnknownManual
        );
    }

    #[test]
    fn owner_token_binds_job_owner_and_generation() {
        let fence = OwnerFence::new("owner-a", 3);
        let first = fence.token_hash("job-1");
        assert_eq!(first, OwnerFence::new("owner-a", 3).token_hash("job-1"));
        assert_ne!(first, OwnerFence::new("owner-a", 4).token_hash("job-1"));
        assert_ne!(first, OwnerFence::new("owner-b", 3).token_hash("job-1"));
        assert_ne!(first, fence.token_hash("job-2"));
        assert!(is_sha256_hex(&first));
    }

    #[test]
    fn job_ids_are_deterministic_and_backend_minted() {
        let first = job_id_for("campaign-1", "baseline", "smoke_reply_ok", 0);
        assert_eq!(
            first,
            job_id_for("campaign-1", "baseline", "smoke_reply_ok", 0)
        );
        assert_ne!(
            first,
            job_id_for("campaign-1", "baseline", "smoke_reply_ok", 1)
        );
        assert_ne!(
            first,
            job_id_for("campaign-1", "other", "smoke_reply_ok", 0)
        );
        assert!(first.starts_with("job-"));
    }

    #[test]
    fn schedule_request_parses_and_hashes_deterministically() {
        let value = sample_schedule_value();
        let request = parse_and_validate_campaign_schedule_request(&value).expect("valid");
        assert_eq!(request.schema_version, CAMPAIGN_SCHEDULE_V1);
        assert_eq!(request.plan.campaign_key, "p2gh-smoke");
        assert_eq!(request.fixture_refs.len(), 1);
        let first = schedule_request_hash(&request);
        let second =
            schedule_request_hash(&parse_and_validate_campaign_schedule_request(&value).unwrap());
        assert_eq!(first, second);
        assert!(is_sha256_hex(&first));
    }

    #[test]
    fn schedule_request_rejects_unknown_and_forbidden_fields() {
        let mut unknown = sample_schedule_value();
        unknown["extra"] = serde_json::json!(true);
        let error = parse_and_validate_campaign_schedule_request(&unknown).expect_err("unknown");
        assert_eq!(error.code, ScheduleErrorCode::UnknownField);

        for key in [
            "instruction",
            "scope_id",
            "host_path",
            "sandbox",
            "promotion",
        ] {
            let mut forbidden = sample_schedule_value();
            forbidden[key] = serde_json::json!("x");
            let error =
                parse_and_validate_campaign_schedule_request(&forbidden).expect_err("forbidden");
            assert_eq!(error.code, ScheduleErrorCode::ForbiddenField, "key {key}");
        }
    }

    #[test]
    fn schedule_request_rejects_a_mismatched_or_malformed_fixture_ref() {
        let mut mismatched = sample_schedule_value();
        mismatched["fixture_refs"][0]["task_id"] = serde_json::json!("smoke_echo_ping");
        let error =
            parse_and_validate_campaign_schedule_request(&mismatched).expect_err("mismatch");
        assert_eq!(error.code, ScheduleErrorCode::InvalidFixture);

        let mut malformed = sample_schedule_value();
        malformed["fixture_refs"][0]["task_digest"] = serde_json::json!("not-a-digest");
        let error =
            parse_and_validate_campaign_schedule_request(&malformed).expect_err("malformed");
        assert_eq!(error.code, ScheduleErrorCode::InvalidFixture);

        let mut empty = sample_schedule_value();
        empty["fixture_refs"] = serde_json::json!([]);
        let error = parse_and_validate_campaign_schedule_request(&empty).expect_err("empty");
        assert_eq!(error.code, ScheduleErrorCode::InvalidFixture);
    }

    #[test]
    fn schedule_request_rejects_a_non_singleton_concurrency() {
        let mut value = sample_schedule_value();
        value["plan"]["concurrency"] = serde_json::json!(2);
        let error = parse_and_validate_campaign_schedule_request(&value).expect_err("concurrency");
        assert_eq!(error.code, ScheduleErrorCode::InvalidJobState);
    }

    #[test]
    fn schedule_request_requires_a_supported_version() {
        let mut value = sample_schedule_value();
        value["schema_version"] = serde_json::json!("campaign_schedule.v2");
        let error = parse_and_validate_campaign_schedule_request(&value).expect_err("version");
        assert_eq!(error.code, ScheduleErrorCode::UnsupportedVersion);
    }

    #[test]
    fn schedule_request_rejects_duplicate_bundle_refs() {
        let mut value = sample_schedule_value();
        value["bundle_refs"] = serde_json::json!(["smoke-tools", "smoke-tools"]);
        let error = parse_and_validate_campaign_schedule_request(&value).expect_err("duplicate");
        assert_eq!(error.code, ScheduleErrorCode::BundleRefUnknown);
    }

    #[test]
    fn execution_profile_accepts_the_valid_shapes() {
        for owner_kind in [
            OwnerKind::HostWorktree,
            OwnerKind::PersistentDocker,
            OwnerKind::HarborTask,
        ] {
            let profile = valid_profile(owner_kind);
            profile.validate().expect("valid profile");
            assert!(is_sha256_hex(&profile.profile_hash()));
        }
    }

    #[test]
    fn execution_profile_requires_a_digest_pinned_image() {
        let mut profile = valid_profile(OwnerKind::PersistentDocker);
        profile.image_reference = Some("chatspeed/runner:latest".to_string());
        let error = profile.validate().expect_err("unpinned");
        assert_eq!(error.code, ScheduleErrorCode::ImageNotDigestPinned);

        let mut missing = valid_profile(OwnerKind::PersistentDocker);
        missing.image_reference = None;
        let error = missing.validate().expect_err("missing image");
        assert_eq!(error.code, ScheduleErrorCode::InvalidExecutionProfile);
    }

    #[test]
    fn execution_profile_rejects_host_path_mounts() {
        let mut profile = valid_profile(OwnerKind::PersistentDocker);
        profile.mounts.push(MountSpecV1 {
            source_kind: "host_path".to_string(),
            container_path: "/host".to_string(),
            read_only: true,
        });
        let error = profile.validate().expect_err("host mount");
        assert_eq!(error.code, ScheduleErrorCode::InvalidExecutionProfile);
    }

    #[test]
    fn execution_profile_rejects_an_unknown_network_mode() {
        let mut profile = valid_profile(OwnerKind::PersistentDocker);
        profile.network_policy = Some(NetworkPolicyV1 {
            mode: "host".to_string(),
            allow_hosts: Vec::new(),
        });
        let error = profile.validate().expect_err("network");
        assert_eq!(error.code, ScheduleErrorCode::NetworkPolicyUnsupported);
    }

    #[test]
    fn execution_profile_requires_input_patch_ref_and_digest_together() {
        let mut profile = valid_profile(OwnerKind::HostWorktree);
        profile.input_patch_ref = Some("patch:reviewed".to_string());
        let error = profile.validate().expect_err("digest missing");
        assert_eq!(error.code, ScheduleErrorCode::InvalidExecutionProfile);

        profile.input_patch_digest = Some("b".repeat(64));
        profile.validate().expect("paired");
    }

    #[test]
    fn only_executing_owners_provide_isolation() {
        assert!(!OwnerKind::HostWorktree.provides_execution_isolation());
        assert!(OwnerKind::PersistentDocker.provides_execution_isolation());
        assert!(OwnerKind::HarborTask.provides_execution_isolation());
        assert_eq!(
            OwnerKind::parse("persistent_docker"),
            Some(OwnerKind::PersistentDocker)
        );
        assert_eq!(OwnerKind::parse("nope"), None);
    }

    fn sample_bundle() -> BundleManifestV1 {
        let mut bundle = BundleManifestV1 {
            schema_version: BUNDLE_MANIFEST_V1.to_string(),
            bundle_ref: "smoke-tools".to_string(),
            bundle_version: "1".to_string(),
            content_digest: String::new(),
            files: vec![BundleFileV1 {
                relative_path: "bin/echo_server.py".to_string(),
                size_bytes: 12,
                sha256: "c".repeat(64),
                mode: 0o755,
                executable: true,
                symlink_target: None,
            }],
            mcp_servers: vec![BundleMcpServerV1 {
                name: "echo".to_string(),
                command: "./bin/echo_server.py".to_string(),
                args: vec![],
                env_secret_refs: vec![],
                env_secret_env_names: vec![],
            }],
            skills: vec![BundleSkillV1 {
                name: "smoke".to_string(),
                entry_path: "skills/smoke.md".to_string(),
            }],
            env_secret_refs: vec![],
        };
        bundle.content_digest = bundle.computed_content_digest();
        bundle
    }

    #[test]
    fn bundle_manifest_accepts_a_valid_declaration() {
        let bundle = sample_bundle();
        bundle.validate().expect("valid bundle");
        assert_eq!(bundle.computed_content_digest(), bundle.content_digest);
    }

    #[test]
    fn bundle_manifest_rejects_escaping_and_absolute_paths() {
        for path in [
            "/etc/passwd",
            "../escape",
            "a/../../b",
            "C:/windows",
            "a\\b",
        ] {
            let mut bundle = sample_bundle();
            bundle.files[0].relative_path = path.to_string();
            bundle.content_digest = bundle.computed_content_digest();
            let error = bundle.validate().expect_err(path);
            assert_eq!(
                error.code,
                ScheduleErrorCode::BundlePathUnsafe,
                "path {path}"
            );
        }
    }

    #[test]
    fn bundle_manifest_rejects_a_stale_content_digest() {
        let mut bundle = sample_bundle();
        bundle.content_digest = "d".repeat(64);
        let error = bundle.validate().expect_err("stale digest");
        assert_eq!(error.code, ScheduleErrorCode::BundleDigestMismatch);
    }

    #[test]
    fn bundle_manifest_rejects_remote_fetch_and_secret_values() {
        let mut remote = sample_bundle();
        remote.mcp_servers[0].command = "https://example.invalid/install.sh".to_string();
        let error = remote.validate().expect_err("remote command");
        assert_eq!(error.code, ScheduleErrorCode::BundleNotVerifiable);

        let mut secret = sample_bundle();
        secret.mcp_servers[0].args = vec!["--token=sk-abcdefghijklmnopqrstuvwxyz".to_string()];
        let error = secret.validate().expect_err("secret value");
        assert_eq!(error.code, ScheduleErrorCode::BundleSecretForbidden);

        let mut fetch = sample_bundle();
        fetch.mcp_servers[0].args = vec!["curl https://example.invalid".to_string()];
        let error = fetch.validate().expect_err("fetch arg");
        assert_eq!(error.code, ScheduleErrorCode::BundleNotVerifiable);
    }

    #[test]
    fn bundle_manifest_requires_a_relative_program() {
        let mut bundle = sample_bundle();
        bundle.mcp_servers[0].command = "python3".to_string();
        let error = bundle.validate().expect_err("absolute program");
        assert_eq!(error.code, ScheduleErrorCode::BundleNotVerifiable);
    }

    #[test]
    fn bundle_manifest_rejects_duplicate_and_out_of_range_entries() {
        let mut duplicate = sample_bundle();
        let first = duplicate.files[0].clone();
        duplicate.files.push(first);
        duplicate.content_digest = duplicate.computed_content_digest();
        let error = duplicate.validate().expect_err("duplicate file");
        assert_eq!(error.code, ScheduleErrorCode::BundleManifestInvalid);

        let mut mode = sample_bundle();
        mode.files[0].mode = 0o10000;
        mode.content_digest = mode.computed_content_digest();
        let error = mode.validate().expect_err("bad mode");
        assert_eq!(error.code, ScheduleErrorCode::BundleManifestInvalid);
    }

    #[test]
    fn bundle_manifest_rejects_a_malformed_file_hash() {
        let mut bundle = sample_bundle();
        bundle.files[0].sha256 = "not-a-digest".to_string();
        bundle.content_digest = bundle.computed_content_digest();
        let error = bundle.validate().expect_err("bad hash");
        assert_eq!(error.code, ScheduleErrorCode::BundleManifestInvalid);
    }

    #[test]
    fn bundle_manifest_rejects_an_unpaired_env_secret_ref() {
        let mut bundle = sample_bundle();
        bundle.mcp_servers[0].env_secret_refs = vec!["smoke-token".to_string()];
        bundle.content_digest = bundle.computed_content_digest();
        let error = bundle.validate().expect_err("unpaired secret");
        assert_eq!(error.code, ScheduleErrorCode::BundleManifestInvalid);
    }

    fn sample_capability() -> HarborTaskCapabilityV1 {
        HarborTaskCapabilityV1 {
            schema_version: HARBOR_TASK_CAPABILITY_V1.to_string(),
            task_id: "trial-1".to_string(),
            nonce: "nonce-1".to_string(),
            owner_token_hash: "e".repeat(64),
            task_root: "/task".to_string(),
            workspace_root: "/task/workspace".to_string(),
            artifact_roots: vec!["/logs/artifacts".to_string()],
            network_policy: NetworkPolicyV1 {
                mode: NetworkPolicyV1::MODE_NONE.to_string(),
                allow_hosts: Vec::new(),
            },
            read_only_roots: vec!["/task/fixtures".to_string()],
            issued_at: "2026-09-16T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn harbor_capability_validates_and_confines_paths() {
        let capability = sample_capability();
        capability.validate().expect("valid capability");
        assert!(capability.contains_owned_path("/task/workspace/repo"));
        assert!(capability.contains_owned_path("/logs/artifacts/patch.diff"));
        assert!(capability.contains_owned_path("/task/fixtures/input.patch"));
        assert!(!capability.contains_owned_path("/etc/passwd"));
        assert!(!capability.contains_owned_path("/task/workspaceX"));
        assert!(is_sha256_hex(&capability.capability_hash()));
    }

    #[test]
    fn harbor_capability_hash_excludes_issued_at() {
        let mut later = sample_capability();
        later.issued_at = "2026-09-17T00:00:00Z".to_string();
        assert_eq!(
            later.capability_hash(),
            sample_capability().capability_hash()
        );
    }

    #[test]
    fn harbor_capability_rejects_malformed_proofs() {
        let mut wrong_version = sample_capability();
        wrong_version.schema_version = "harbor_task_capability.v2".to_string();
        let error = wrong_version.validate().expect_err("version");
        assert_eq!(error.code, ScheduleErrorCode::UnsupportedOwnerKind);

        let mut bad_token = sample_capability();
        bad_token.owner_token_hash = "not-a-digest".to_string();
        let error = bad_token.validate().expect_err("token");
        assert_eq!(error.code, ScheduleErrorCode::OwnershipMismatch);

        let mut relative = sample_capability();
        relative.workspace_root = "workspace".to_string();
        let error = relative.validate().expect_err("relative root");
        assert_eq!(error.code, ScheduleErrorCode::OwnershipMismatch);

        let mut no_artifacts = sample_capability();
        no_artifacts.artifact_roots = vec![];
        let error = no_artifacts.validate().expect_err("no artifact root");
        assert_eq!(error.code, ScheduleErrorCode::OwnershipMismatch);
    }

    #[test]
    fn domain_marker_is_strict_and_versioned() {
        let marker = ExperimentDomainMarkerV1::new("domain-1", "2026-09-16T00:00:00Z");
        assert!(marker.is_experiment_v1());
        let encoded = serde_json::to_value(&marker).expect("serializes");
        let decoded: ExperimentDomainMarkerV1 =
            serde_json::from_value(encoded).expect("deserializes");
        assert_eq!(decoded.domain_kind, DOMAIN_KIND_EXPERIMENT_V1);

        let mut wrong_kind = marker;
        wrong_kind.domain_kind = "desktop.v1".to_string();
        assert!(!wrong_kind.is_experiment_v1());
    }

    #[test]
    fn saga_stage_round_trips_through_strings() {
        for stage in [
            JobSagaStage::WorkspaceAcquired,
            JobSagaStage::InputPatchVerified,
            JobSagaStage::BundleStaged,
            JobSagaStage::BundleVerified,
            JobSagaStage::CapabilitiesRegistered,
            JobSagaStage::EnvironmentReady,
            JobSagaStage::DispatchIntent,
            JobSagaStage::WorkflowStarted,
            JobSagaStage::ArtifactsCollected,
            JobSagaStage::CleanupDone,
        ] {
            assert_eq!(JobSagaStage::parse(stage.as_str()), Some(stage));
        }
        assert_eq!(JobSagaStage::parse("nope"), None);
    }

    #[test]
    fn schedule_accepted_projection_is_strict() {
        let accepted = CampaignScheduleAcceptedV1::new(
            "campaign-1",
            "p2gh-smoke",
            "a".repeat(64),
            "b".repeat(64),
            "smoke-local",
            vec!["job-1".to_string()],
        );
        assert_eq!(accepted.schema_version, CAMPAIGN_SCHEDULE_ACCEPTED_V1);
        assert_eq!(accepted.concurrency, SCHEDULE_CONCURRENCY);
        let mut encoded = serde_json::to_value(&accepted).expect("serializes");
        encoded["unexpected"] = serde_json::json!(1);
        assert!(serde_json::from_value::<CampaignScheduleAcceptedV1>(encoded).is_err());
    }

    #[test]
    fn schedule_validators_reject_weak_keys_and_digests() {
        assert!(is_valid_key("smoke-tools"));
        assert!(!is_valid_key("Smoke Tools"));
        assert!(!is_valid_key(""));
        assert!(is_sha256_hex(&"a".repeat(64)));
        assert!(!is_sha256_hex(&"A".repeat(64)));
        assert!(!is_sha256_hex("abc"));
        assert!(is_digest_pinned_image(&format!(
            "chatspeed/runner@sha256:{}",
            "a".repeat(64)
        )));
        assert!(!is_digest_pinned_image("chatspeed/runner:latest"));
        assert!(!is_digest_pinned_image("@sha256:abc"));
        // A locally built image is pinned by its immutable image id.
        assert!(is_digest_pinned_image(&format!(
            "sha256:{}",
            "a".repeat(64)
        )));
        assert!(!is_digest_pinned_image("sha256:abc"));
    }
}
