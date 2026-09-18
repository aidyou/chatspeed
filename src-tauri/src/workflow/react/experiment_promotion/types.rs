//! Phase 2I frozen contracts: promotion evidence, the promotion FSM, effect
//! intents, restart classification and the strict paired-canary result.
//!
//! This module is the *contract* half of Phase 2I. It defines the strict,
//! versioned documents and the pure state machine that the promotion store
//! (U-2), the evidence/policy bridge (U-3), the checkpoint owner (U-4), the
//! canary runner (U-5), the promotion supervisor (U-6) and the `cs` CLI (U-7)
//! all share. Nothing here performs I/O, opens a database, spawns a process or
//! reaches a provider.
//!
//! Design rules enforced here (AC-2/AC-3/AC-7; INV-2/3/6/7/8):
//!
//! - Every external document is strict, snake_case and rejects unknown fields;
//!   caller-forbidden fields fail closed with a stable machine code *before*
//!   any effect.
//! - A promotion request carries verified facts and digests only: never an
//!   instruction, never a host path, never a branch name, never a command,
//!   never a threshold and never a secret. The target, the policy and the
//!   canary programme are resolved by the backend from its own registry.
//! - The promotion FSM, the effect intents and the restart classification are
//!   *pure* functions over typed state, so recovery can never be inferred from
//!   a transcript, a log line or assistant text.
//! - A canary result is a typed document whose pass/fail verdict is recomputed
//!   by the runner; the programme's own declaration is only cross-checked.

use crate::workflow::react::campaign::{canonical_hash, validate_campaign_id};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Fixed schema version literals. Each is a distinct, versioned contract.
pub const PROMOTION_EVIDENCE_V1: &str = "promotion_evidence.v1";
pub const PROMOTION_REQUEST_V1: &str = "promotion_request.v1";
pub const CANARY_RESULT_V1: &str = "canary_result.v1";
/// Fixed schema version of one arm's single-stage measurement.
pub const CANARY_ARM_SAMPLE_V1: &str = "canary_arm_sample.v1";
/// Fixed schema version of the promotion status projection.
pub const PROMOTION_PROJECTION_V1: &str = "promotion_projection.v1";
/// Fixed schema version of the evidence-only reconcile result.
pub const PROMOTION_RECONCILE_V1: &str = "promotion_reconcile.v1";
/// Fixed schema version of the offline-verifiable audit bundle document.
pub const PROMOTION_AUDIT_V1: &str = "promotion_audit.v1";

/// Canonical hash domains for the Phase 2I identities.
pub const PROMOTION_EVIDENCE_HASH_DOMAIN: &str = "cs-promotion:evidence";
pub const PROMOTION_REQUEST_HASH_DOMAIN: &str = "cs-promotion:request";
pub const PROMOTION_ID_DOMAIN: &str = "cs-promotion:id";
pub const PROMOTION_OWNER_TOKEN_DOMAIN: &str = "cs-promotion:owner-token";
pub const CANARY_RESULT_HASH_DOMAIN: &str = "cs-promotion:canary-result";
/// Hash domain of the ordered, append-only promotion journal.
pub const PROMOTION_JOURNAL_HASH_DOMAIN: &str = "cs-promotion:journal";

/// Namespace every promotion checkpoint ref lives under. A checkpoint ref is
/// backend-minted from the promotion id and is never a caller input.
pub const CHECKPOINT_REF_PREFIX: &str = "refs/chatspeed/checkpoints/";

/// Hard cap on declared canary stages. The target registry is server-owned, so
/// this is a defence against an accidentally unbounded target document rather
/// than a caller-facing limit.
pub const MAX_CANARY_STAGES: usize = 8;

/// Hard cap on a single canary programme's structured output.
pub const MAX_CANARY_OUTPUT_BYTES: u64 = 1 << 20;

/// Hard cap on a canary programme's wall clock, in milliseconds.
pub const MAX_CANARY_TIMEOUT_MS: u64 = 15 * 60 * 1000;

// ---------------------------------------------------------------------------
// Machine codes
// ---------------------------------------------------------------------------

/// Stable machine codes for Phase 2I contract and gate rejections. They
/// describe a request rejected *before* an effect and are part of the
/// HTTP/CLI contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromotionErrorCode {
    /// The document `schema_version` is missing or unsupported.
    UnsupportedVersion,
    /// A field is not part of the strict document schema.
    UnknownField,
    /// A caller-supplied field is forbidden for this document.
    ForbiddenField,
    /// The campaign id is missing or not backend-minted.
    InvalidCampaignId,
    /// The candidate key is empty or malformed.
    InvalidCandidateKey,
    /// The referenced promotion target is not registered by the server.
    UnknownPromotionTarget,
    /// The promotion target document is structurally invalid.
    InvalidPromotionTarget,
    /// The promotion policy document is structurally invalid.
    InvalidPromotionPolicy,
    /// The canary specification is invalid or unsafe.
    InvalidCanarySpec,
    /// The referenced execution profile is not registered by the server.
    UnknownExecutionProfile,
    /// The declared branch ref is missing, relative or otherwise unsafe.
    UnsafeTargetRef,
    /// A digest or id field is malformed.
    MalformedDigest,
    /// A declaration is internally inconsistent (paired fields disagree).
    InconsistentEvidence,
    /// The durable campaign the request names is unknown for this domain.
    UnknownCampaign,
    /// The campaign is not in a state that admits promotion.
    CampaignNotActive,
    /// The named candidate is not a member of the campaign.
    CandidateNotInCampaign,
    /// A durable baseline/candidate job could not be resolved.
    JobUnresolved,
    /// A durable job is not in the `succeeded` terminal state.
    JobNotSucceeded,
    /// The bound artifact could not be independently re-verified.
    ArtifactUnbound,
    /// The immutable patch manifest could not be independently re-verified.
    PatchUnbound,
    /// The verified facts are not internally consistent with the durable rows.
    EvidenceMismatch,
    /// The 2E verdict did not report an overall pass.
    VerdictNotPassed,
    /// The 2E verdict reported a safety failure.
    SafetyGateFailed,
    /// The 2E verdict reported an infrastructure failure.
    InfraGateFailed,
    /// A budget admission rejected the run, or the budget facts are unknown.
    BudgetGateFailed,
    /// The evidence binds too few samples for the policy to decide.
    InsufficientSamples,
    /// No pre-registered metric improved by the required margin.
    NoImprovement,
    /// At least one pre-registered metric regressed beyond its allowance.
    CriticalRegression,
    /// The promotion request was already decided and is rejected.
    PromotionRejected,
    /// The promotion state transition is not part of the FSM.
    InvalidPromotionTransition,
    /// The promotion row is internally inconsistent.
    InvalidPromotionState,
    /// Another live promotion holds the single-flight slot for this target.
    PromotionInFlight,
    /// Another live owner holds a higher or equal lease generation.
    LeaseConflict,
    /// This worker's lease is no longer the current generation.
    LeaseLost,
    /// An effect intent was recorded but the effect cannot be proven absent.
    EffectUncertain,
    /// The expected old head no longer matches the registered branch.
    BranchHeadDrift,
    /// The target branch is checked out in a worktree and cannot be updated.
    BranchCheckedOut,
    /// The local repository or target ref cannot be resolved by this backend.
    RepositoryUnavailable,
    /// The local runtime the effect needs (docker, git, a container) is
    /// unavailable, so nothing was attempted (fail closed).
    ExecutorUnavailable,
    /// The input patch failed to apply cleanly onto the expected base.
    PatchApplyFailed,
    /// The checkpoint commit or ref was not produced.
    CheckpointFailed,
    /// A canary stage failed, regressed or timed out.
    CanaryStageFailed,
    /// The canary result document is malformed, oversized or inconsistent.
    CanaryResultInvalid,
    /// The canary programme attempted an effect the target forbids.
    CanaryEffectForbidden,
    /// The same idempotency key was replayed with a different body.
    IdempotencyConflict,
    /// A durable writer transaction failed.
    PersistenceFailure,
    /// The audit bundle could not be published atomically.
    AuditPublicationFailed,
    /// The promotion id is unknown for this domain.
    UnknownPromotion,
}

impl PromotionErrorCode {
    /// Stable snake_case machine code.
    pub fn as_str(&self) -> &'static str {
        match self {
            PromotionErrorCode::UnsupportedVersion => "unsupported_promotion_schema",
            PromotionErrorCode::UnknownField => "unknown_field",
            PromotionErrorCode::ForbiddenField => "forbidden_field",
            PromotionErrorCode::InvalidCampaignId => "invalid_campaign_id",
            PromotionErrorCode::InvalidCandidateKey => "invalid_candidate_key",
            PromotionErrorCode::UnknownPromotionTarget => "unknown_promotion_target",
            PromotionErrorCode::InvalidPromotionTarget => "invalid_promotion_target",
            PromotionErrorCode::InvalidPromotionPolicy => "invalid_promotion_policy",
            PromotionErrorCode::InvalidCanarySpec => "invalid_canary_spec",
            PromotionErrorCode::UnknownExecutionProfile => "unknown_execution_profile",
            PromotionErrorCode::UnsafeTargetRef => "unsafe_target_ref",
            PromotionErrorCode::MalformedDigest => "malformed_digest",
            PromotionErrorCode::InconsistentEvidence => "inconsistent_evidence",
            PromotionErrorCode::UnknownCampaign => "unknown_campaign",
            PromotionErrorCode::CampaignNotActive => "campaign_not_active",
            PromotionErrorCode::CandidateNotInCampaign => "candidate_not_in_campaign",
            PromotionErrorCode::JobUnresolved => "job_unresolved",
            PromotionErrorCode::JobNotSucceeded => "job_not_succeeded",
            PromotionErrorCode::ArtifactUnbound => "artifact_unbound",
            PromotionErrorCode::PatchUnbound => "patch_unbound",
            PromotionErrorCode::EvidenceMismatch => "evidence_mismatch",
            PromotionErrorCode::VerdictNotPassed => "verdict_not_passed",
            PromotionErrorCode::SafetyGateFailed => "safety_gate_failed",
            PromotionErrorCode::InfraGateFailed => "infra_gate_failed",
            PromotionErrorCode::BudgetGateFailed => "budget_gate_failed",
            PromotionErrorCode::InsufficientSamples => "insufficient_samples",
            PromotionErrorCode::NoImprovement => "no_improvement",
            PromotionErrorCode::CriticalRegression => "critical_regression",
            PromotionErrorCode::PromotionRejected => "promotion_rejected",
            PromotionErrorCode::InvalidPromotionTransition => "invalid_promotion_transition",
            PromotionErrorCode::InvalidPromotionState => "invalid_promotion_state",
            PromotionErrorCode::PromotionInFlight => "promotion_in_flight",
            PromotionErrorCode::LeaseConflict => "lease_conflict",
            PromotionErrorCode::LeaseLost => "lease_lost",
            PromotionErrorCode::EffectUncertain => "effect_uncertain",
            PromotionErrorCode::BranchHeadDrift => "branch_head_drift",
            PromotionErrorCode::BranchCheckedOut => "branch_checked_out",
            PromotionErrorCode::RepositoryUnavailable => "repository_unavailable",
            PromotionErrorCode::ExecutorUnavailable => "executor_unavailable",
            PromotionErrorCode::PatchApplyFailed => "patch_apply_failed",
            PromotionErrorCode::CheckpointFailed => "checkpoint_failed",
            PromotionErrorCode::CanaryStageFailed => "canary_stage_failed",
            PromotionErrorCode::CanaryResultInvalid => "canary_result_invalid",
            PromotionErrorCode::CanaryEffectForbidden => "canary_effect_forbidden",
            PromotionErrorCode::IdempotencyConflict => "idempotency_conflict",
            PromotionErrorCode::PersistenceFailure => "persistence_failure",
            PromotionErrorCode::AuditPublicationFailed => "audit_publication_failed",
            PromotionErrorCode::UnknownPromotion => "unknown_promotion",
        }
    }
}

/// A Phase 2I contract rejection carrying a stable machine code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromotionError {
    pub code: PromotionErrorCode,
    pub message: String,
}

impl PromotionError {
    pub fn new(code: PromotionErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for PromotionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for PromotionError {}

fn error(code: PromotionErrorCode, message: impl Into<String>) -> PromotionError {
    PromotionError::new(code, message)
}

// ---------------------------------------------------------------------------
// Promotion FSM
// ---------------------------------------------------------------------------

/// Durable promotion state.
///
/// ```text
/// queued → evidence_validating → rejected
///                              ↘ checkpointing → checkpointed
///                                                 → canary_running → canary_failed
///                                                                  → ready_to_advance
///                                                                     → advancing → promoted
/// ```
///
/// The abnormal terminal states are `rolled_back` and `unknown_manual`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromotionState {
    /// Persisted, not yet claimed by a promotion worker.
    Queued,
    /// The backend is re-validating the projected evidence and policy.
    EvidenceValidating,
    /// The evidence or policy gate rejected the candidate. Terminal.
    Rejected,
    /// A checkpoint commit/ref is being created for this attempt.
    Checkpointing,
    /// The checkpoint ref immutably pins the candidate commit.
    Checkpointed,
    /// The paired staged canary is running against old and checkpoint.
    CanaryRunning,
    /// At least one canary stage failed. Terminal; the branch never moved.
    CanaryFailed,
    /// Every canary stage passed; the branch CAS may now run.
    ReadyToAdvance,
    /// A branch advance intent is durable; the CAS may or may not have run.
    Advancing,
    /// The registered experiment branch now points at the checkpoint. Terminal.
    Promoted,
    /// A post-CAS failure was compensated back to the old head. Terminal.
    RolledBack,
    /// The durable state cannot be resolved without a human. Terminal.
    UnknownManual,
}

impl PromotionState {
    pub fn as_str(&self) -> &'static str {
        match self {
            PromotionState::Queued => "queued",
            PromotionState::EvidenceValidating => "evidence_validating",
            PromotionState::Rejected => "rejected",
            PromotionState::Checkpointing => "checkpointing",
            PromotionState::Checkpointed => "checkpointed",
            PromotionState::CanaryRunning => "canary_running",
            PromotionState::CanaryFailed => "canary_failed",
            PromotionState::ReadyToAdvance => "ready_to_advance",
            PromotionState::Advancing => "advancing",
            PromotionState::Promoted => "promoted",
            PromotionState::RolledBack => "rolled_back",
            PromotionState::UnknownManual => "unknown_manual",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "queued" => PromotionState::Queued,
            "evidence_validating" => PromotionState::EvidenceValidating,
            "rejected" => PromotionState::Rejected,
            "checkpointing" => PromotionState::Checkpointing,
            "checkpointed" => PromotionState::Checkpointed,
            "canary_running" => PromotionState::CanaryRunning,
            "canary_failed" => PromotionState::CanaryFailed,
            "ready_to_advance" => PromotionState::ReadyToAdvance,
            "advancing" => PromotionState::Advancing,
            "promoted" => PromotionState::Promoted,
            "rolled_back" => PromotionState::RolledBack,
            "unknown_manual" => PromotionState::UnknownManual,
            _ => return None,
        })
    }

    /// Terminal states accept no further mutation and are never resumed.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            PromotionState::Rejected
                | PromotionState::CanaryFailed
                | PromotionState::Promoted
                | PromotionState::RolledBack
                | PromotionState::UnknownManual
        )
    }

    /// States in which no Git effect has been attempted yet.
    pub fn is_pre_effect(&self) -> bool {
        matches!(
            self,
            PromotionState::Queued | PromotionState::EvidenceValidating
        )
    }
}

/// Returns whether `from -> to` is one of the FSM's edges. A terminal state may
/// only be re-written to itself (idempotent durable update); nothing else.
pub fn transition_allowed(from: PromotionState, to: PromotionState) -> bool {
    use PromotionState::*;
    if from == to {
        return from.is_terminal();
    }
    matches!(
        (from, to),
        (Queued, EvidenceValidating)
            | (Queued, UnknownManual)
            | (EvidenceValidating, Rejected)
            | (EvidenceValidating, Checkpointing)
            | (EvidenceValidating, UnknownManual)
            | (Checkpointing, Checkpointed)
            | (Checkpointing, Rejected)
            | (Checkpointing, UnknownManual)
            | (Checkpointed, CanaryRunning)
            | (Checkpointed, UnknownManual)
            | (CanaryRunning, CanaryFailed)
            | (CanaryRunning, ReadyToAdvance)
            | (CanaryRunning, UnknownManual)
            | (ReadyToAdvance, Advancing)
            | (ReadyToAdvance, UnknownManual)
            | (Advancing, Promoted)
            | (Advancing, RolledBack)
            | (Advancing, UnknownManual)
    )
}

/// Validates one FSM edge.
pub fn validate_transition(from: PromotionState, to: PromotionState) -> Result<(), PromotionError> {
    if transition_allowed(from, to) {
        return Ok(());
    }
    Err(error(
        PromotionErrorCode::InvalidPromotionTransition,
        format!(
            "promotion transition {} -> {} is not allowed",
            from.as_str(),
            to.as_str()
        ),
    ))
}

// ---------------------------------------------------------------------------
// Effect intents and restart classification
// ---------------------------------------------------------------------------

/// Whether a single-use Git effect has been attempted. Written durably *before*
/// the effect so recovery always has a typed starting point (INV-7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectIntent {
    /// No intent was ever written for this effect.
    NotStarted,
    /// An intent is durable; the effect may or may not have happened.
    IntentRecorded,
    /// The effect is confirmed by a durable observation.
    Completed,
}

impl EffectIntent {
    pub fn as_str(&self) -> &'static str {
        match self {
            EffectIntent::NotStarted => "not_started",
            EffectIntent::IntentRecorded => "intent_recorded",
            EffectIntent::Completed => "completed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "not_started" => EffectIntent::NotStarted,
            "intent_recorded" => EffectIntent::IntentRecorded,
            "completed" => EffectIntent::Completed,
            _ => return None,
        })
    }
}

/// What the backend observed about the checkpoint ref of one promotion attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointObservation {
    /// The namespaced checkpoint ref does not exist.
    Absent,
    /// The ref exists and its commit carries this attempt's evidence bindings.
    PresentConsistent,
    /// The ref exists but does not prove this attempt (a foreign or drifted ref).
    PresentInconsistent,
}

impl CheckpointObservation {
    pub fn as_str(&self) -> &'static str {
        match self {
            CheckpointObservation::Absent => "absent",
            CheckpointObservation::PresentConsistent => "present_consistent",
            CheckpointObservation::PresentInconsistent => "present_inconsistent",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "absent" => CheckpointObservation::Absent,
            "present_consistent" => CheckpointObservation::PresentConsistent,
            "present_inconsistent" => CheckpointObservation::PresentInconsistent,
            _ => return None,
        })
    }
}

/// What the backend observed about the registered experiment branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchObservation {
    /// The branch still points at the expected old head.
    AtOld,
    /// The branch points at this attempt's checkpoint commit.
    AtCheckpoint,
    /// The branch points somewhere else: an external actor moved it.
    ThirdValue,
}

impl BranchObservation {
    pub fn as_str(&self) -> &'static str {
        match self {
            BranchObservation::AtOld => "at_old",
            BranchObservation::AtCheckpoint => "at_checkpoint",
            BranchObservation::ThirdValue => "third_value",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "at_old" => BranchObservation::AtOld,
            "at_checkpoint" => BranchObservation::AtCheckpoint,
            "third_value" => BranchObservation::ThirdValue,
            _ => return None,
        })
    }
}

/// The deterministic restart-recovery outcome for a durable promotion. This is
/// the only place that decides what a restart may do; every arm that could
/// replay an unknown effect resolves to `ParkUnknown` (INV-7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromotionRecoveryDecision {
    /// Terminal already: keep the recorded outcome, do no work.
    Terminal,
    /// Pure validation only; may re-run the evidence/policy gate.
    ResumeEvidence,
    /// A checkpoint is needed or was already adopted; no branch mutation.
    ResumeCheckpoint,
    /// The checkpoint exists; (re)run the idempotent paired canary.
    ResumeCanary,
    /// Attempt the branch CAS from the expected old head.
    Advance,
    /// The CAS provably happened; record the journal roll-forward.
    RollForward,
    /// No automatic action; a human must resolve the ambiguity.
    ParkUnknown,
}

impl PromotionRecoveryDecision {
    pub fn as_str(&self) -> &'static str {
        match self {
            PromotionRecoveryDecision::Terminal => "terminal",
            PromotionRecoveryDecision::ResumeEvidence => "resume_evidence",
            PromotionRecoveryDecision::ResumeCheckpoint => "resume_checkpoint",
            PromotionRecoveryDecision::ResumeCanary => "resume_canary",
            PromotionRecoveryDecision::Advance => "advance",
            PromotionRecoveryDecision::RollForward => "roll_forward",
            PromotionRecoveryDecision::ParkUnknown => "park_unknown",
        }
    }
}

/// Classifies what a restart may do, from typed durable state plus the two
/// authoritative Git observations.
///
/// The classification is deliberately conservative: a state that requires a
/// checkpoint the ref cannot prove, or a branch that moved to a third value,
/// always parks as `unknown_manual` instead of guessing (INV-7).
pub fn classify_promotion_recovery(
    state: PromotionState,
    checkpoint: CheckpointObservation,
    branch: BranchObservation,
    checkpoint_intent: EffectIntent,
) -> PromotionRecoveryDecision {
    use PromotionRecoveryDecision::*;
    if state.is_terminal() {
        return Terminal;
    }
    match state {
        // No effect was ever attempted for these states.
        PromotionState::Queued | PromotionState::EvidenceValidating => ResumeEvidence,
        PromotionState::Checkpointing => match (checkpoint_intent, checkpoint) {
            // Intent recorded but nothing is visible: the effect may still be
            // in flight or may never have run. Re-creating it under a fresh
            // lease is safe because the checkpoint ref is namespaced per
            // attempt and `git commit-tree` is deterministic, but an
            // *unrecorded* intent means the state itself is inconsistent.
            (EffectIntent::IntentRecorded, CheckpointObservation::Absent) => ResumeCheckpoint,
            (EffectIntent::IntentRecorded, CheckpointObservation::PresentConsistent) => {
                ResumeCheckpoint
            }
            (EffectIntent::IntentRecorded, CheckpointObservation::PresentInconsistent) => {
                ParkUnknown
            }
            (EffectIntent::NotStarted, CheckpointObservation::Absent) => ResumeCheckpoint,
            _ => ParkUnknown,
        },
        PromotionState::Checkpointed => match checkpoint {
            CheckpointObservation::PresentConsistent => ResumeCanary,
            // The row claims a checkpoint the repository cannot prove.
            _ => ParkUnknown,
        },
        PromotionState::CanaryRunning => match checkpoint {
            CheckpointObservation::PresentConsistent => ResumeCanary,
            _ => ParkUnknown,
        },
        PromotionState::ReadyToAdvance => match checkpoint {
            CheckpointObservation::PresentConsistent => match branch {
                BranchObservation::AtOld => Advance,
                BranchObservation::AtCheckpoint => RollForward,
                BranchObservation::ThirdValue => ParkUnknown,
            },
            _ => ParkUnknown,
        },
        PromotionState::Advancing => match (checkpoint, branch) {
            // The CAS never landed, or landed and was never recorded: both
            // resolve to the same expected-old CAS, which is idempotent.
            (CheckpointObservation::PresentConsistent, BranchObservation::AtOld) => Advance,
            (CheckpointObservation::PresentConsistent, BranchObservation::AtCheckpoint) => {
                RollForward
            }
            _ => ParkUnknown,
        },
        PromotionState::Rejected
        | PromotionState::CanaryFailed
        | PromotionState::Promoted
        | PromotionState::RolledBack
        | PromotionState::UnknownManual => Terminal,
    }
}

// ---------------------------------------------------------------------------
// Fenced ownership
// ---------------------------------------------------------------------------

/// Fenced ownership of a claimed promotion. Both the owner id and the lease
/// generation must match for any mutation; a stale generation can neither
/// advance the state, run the canary, mutate a ref nor clean up (INV-2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromotionFence {
    pub owner_id: String,
    pub lease_generation: i64,
}

impl PromotionFence {
    pub fn new(owner_id: impl Into<String>, lease_generation: i64) -> Self {
        Self {
            owner_id: owner_id.into(),
            lease_generation,
        }
    }

    /// The opaque ownership token hash bound to the promotion id, the owner and
    /// the generation. A token can never be replayed by a different owner or
    /// generation.
    pub fn token_hash(&self, promotion_id: &str) -> String {
        canonical_hash(
            PROMOTION_OWNER_TOKEN_DOMAIN,
            &serde_json::json!({
                "promotion_id": promotion_id,
                "owner_id": self.owner_id,
                "lease_generation": self.lease_generation,
            }),
        )
    }
}

// ---------------------------------------------------------------------------
// Minters and validators
// ---------------------------------------------------------------------------

/// Deterministic backend-minted promotion id. A caller can never supply one.
pub fn promotion_id_for(
    campaign_id: &str,
    candidate_key: &str,
    target_ref: &str,
    evidence_hash: &str,
) -> String {
    let digest = canonical_hash(
        PROMOTION_ID_DOMAIN,
        &serde_json::json!({
            "campaign_id": campaign_id,
            "candidate_key": candidate_key,
            "target_ref": target_ref,
            "evidence_hash": evidence_hash,
        }),
    );
    format!("promo-{}", &digest[..32])
}

/// Rejects a promotion id that is not a backend-minted promotion id.
pub fn validate_promotion_id(promotion_id: &str) -> Result<(), PromotionError> {
    let valid = promotion_id.len() == 38
        && promotion_id.starts_with("promo-")
        && promotion_id[6..]
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase());
    if valid {
        Ok(())
    } else {
        Err(error(
            PromotionErrorCode::UnknownPromotion,
            format!("'{promotion_id}' is not a promotion id"),
        ))
    }
}

/// Matches the 2F/2G key alphabet so keys stay comparable across contracts.
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

/// A full, absolute branch ref under `refs/heads/`. A relative name, a remote
/// tracking ref, a tag ref or a namespaced non-branch ref is rejected, so a
/// target can never name something a `git update-ref` would interpret
/// differently from what the operator registered (INV-4/INV-5).
pub fn is_full_branch_ref(value: &str) -> bool {
    let Some(name) = value.strip_prefix("refs/heads/") else {
        return false;
    };
    !name.is_empty()
        && !name.starts_with('/')
        && !name.ends_with('/')
        && !name.ends_with(".lock")
        && !name.contains("..")
        && !name.contains("//")
        && !name.contains("@{")
        && !name.chars().any(|c| {
            c.is_ascii_whitespace()
                || c.is_control()
                || matches!(c, '~' | '^' | ':' | '?' | '*' | '[' | '\\')
        })
        && name.split('/').all(|component| {
            !component.is_empty()
                && !component.starts_with('.')
                && !component.starts_with('-')
                && !component.ends_with(".lock")
        })
}

/// The backend-minted checkpoint ref for one promotion id.
pub fn checkpoint_ref_for(promotion_id: &str) -> String {
    format!("{CHECKPOINT_REF_PREFIX}{promotion_id}")
}

/// A finite number. `NaN` and the infinities have no canonical JSON
/// representation, so they can never enter a hash.
fn is_finite(value: f64) -> bool {
    value.is_finite()
}

// ---------------------------------------------------------------------------
// Promotion evidence
// ---------------------------------------------------------------------------

/// Structured per-metric aggregate of one arm. Deliberately aggregate: the raw
/// per-task transcript is never projected, only counts and means, so the audit
/// bundle stays free of holdout detail (INV-8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct PromotionMetricFactV1 {
    /// Metric key; resolved against the server policy, never a free-form label.
    pub metric: String,
    /// Number of independent samples behind the two means.
    pub samples: u32,
    /// Number of baseline samples that met the metric's own pass condition.
    pub baseline_passed: u32,
    /// Number of candidate samples that met the metric's own pass condition.
    pub candidate_passed: u32,
    /// Mean metric value over the baseline arm.
    pub baseline_mean: f64,
    /// Mean metric value over the candidate arm.
    pub candidate_mean: f64,
}

impl PromotionMetricFactV1 {
    fn validate(&self) -> Result<(), PromotionError> {
        if !is_valid_key(&self.metric) {
            return Err(error(
                PromotionErrorCode::InconsistentEvidence,
                format!("metric fact has an invalid metric key '{}'", self.metric),
            ));
        }
        if self.samples == 0 {
            return Err(error(
                PromotionErrorCode::InsufficientSamples,
                format!("metric '{}' declares no samples", self.metric),
            ));
        }
        if self.baseline_passed > self.samples || self.candidate_passed > self.samples {
            return Err(error(
                PromotionErrorCode::InconsistentEvidence,
                format!(
                    "metric '{}' declares more passing samples than samples",
                    self.metric
                ),
            ));
        }
        if !is_finite(self.baseline_mean) || !is_finite(self.candidate_mean) {
            return Err(error(
                PromotionErrorCode::InconsistentEvidence,
                format!("metric '{}' declares a non-finite mean", self.metric),
            ));
        }
        Ok(())
    }
}

/// The budget facts one arm must carry for the promotion gate. Only typed facts
/// and digests; never a ledger row, never a price table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct PromotionBudgetFactsV1 {
    /// `known` or `unknown`, mirroring the 2A artifact's own cost status.
    pub cost_status: String,
    /// Whether budget admission rejected any run of this arm.
    pub budget_rejected: bool,
    /// Committed cost in integer micro-units of the campaign currency.
    pub committed_micros: i64,
    /// Currency the micro-units are denominated in.
    pub currency: String,
}

impl PromotionBudgetFactsV1 {
    fn validate(&self) -> Result<(), PromotionError> {
        if !matches!(self.cost_status.as_str(), "known" | "unknown") {
            return Err(error(
                PromotionErrorCode::InconsistentEvidence,
                format!("unsupported cost_status '{}'", self.cost_status),
            ));
        }
        if self.committed_micros < 0 {
            return Err(error(
                PromotionErrorCode::InconsistentEvidence,
                "budget facts declare a negative committed cost",
            ));
        }
        if !is_valid_key(&self.currency) {
            return Err(error(
                PromotionErrorCode::InconsistentEvidence,
                "budget facts declare an invalid currency",
            ));
        }
        Ok(())
    }
}

/// The independent verifier identity that produced the bound 2E verdict. The
/// digest is the verifier contract digest, so a verdict can never be re-bound
/// to a different verifier after the fact (INV-3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct PromotionVerifierIdentityV1 {
    pub verifier_id: String,
    pub verifier_version: String,
    pub verifier_digest: String,
}

impl PromotionVerifierIdentityV1 {
    fn validate(&self) -> Result<(), PromotionError> {
        if self.verifier_id.trim().is_empty() || self.verifier_version.trim().is_empty() {
            return Err(error(
                PromotionErrorCode::InconsistentEvidence,
                "verifier identity is incomplete",
            ));
        }
        if !is_sha256_hex(&self.verifier_digest) {
            return Err(error(
                PromotionErrorCode::MalformedDigest,
                "verifier digest is not a sha256 hex digest",
            ));
        }
        Ok(())
    }
}

/// The strict, versioned evidence projection the CLI submits for one candidate.
///
/// It carries **verified facts and digests only**. It cannot name a branch, a
/// repository path, a command, a threshold, a policy or a secret: all of those
/// are resolved by the backend from its own registry (INV-4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct PromotionEvidenceV1 {
    pub schema_version: String,
    /// The durable campaign this candidate belongs to.
    pub campaign_id: String,
    pub candidate_key: String,
    /// Durable baseline/candidate job ids minted by the 2G scheduler.
    pub baseline_job_id: String,
    pub candidate_job_id: String,
    /// Run/session ids of the two arms, as recorded by the durable job rows.
    pub baseline_run_id: String,
    pub candidate_run_id: String,
    pub candidate_session_id: String,
    /// Digests of the immutable 2A artifacts of the two arms.
    pub baseline_artifact_hash: String,
    pub candidate_artifact_hash: String,
    /// Digests of the 2D evaluation sidecars of the two arms.
    pub baseline_evaluation_hash: String,
    pub candidate_evaluation_hash: String,
    /// Digests of the consumed 2E verdicts of the two arms.
    pub baseline_verdict_hash: String,
    pub candidate_verdict_hash: String,
    /// Digest-bound 2E fixture identity.
    pub fixture_ref: String,
    pub fixture_digest: String,
    pub task_id: String,
    pub suite: String,
    pub dataset_id: String,
    pub dataset_version: u32,
    pub split: String,
    /// The server-registered execution profile the arms ran under.
    pub execution_profile_ref: String,
    pub execution_profile_hash: String,
    /// The immutable candidate patch this promotion would checkpoint.
    pub patch_manifest_hash: String,
    pub patch_sha256: String,
    /// The base revision the patch was produced against.
    pub base_revision: String,
    /// The candidate arm's independent verdict facts.
    pub verdict_status: String,
    pub verdict_score: f64,
    pub verdict_safety_status: String,
    pub verdict_infra_status: String,
    pub verdict_cost_status: String,
    pub budget: PromotionBudgetFactsV1,
    pub verifier: PromotionVerifierIdentityV1,
    /// Per-metric aggregates for the two arms.
    pub metrics: Vec<PromotionMetricFactV1>,
}

impl PromotionEvidenceV1 {
    /// Validates the evidence projection in isolation.
    ///
    /// This is only a *structural* check: it proves the document is
    /// self-consistent. Every binding to the durable job rows, the artifact
    /// chain and the immutable patch is cross-checked by the backend (U-3)
    /// before any effect.
    pub fn validate(&self) -> Result<(), PromotionError> {
        if self.schema_version != PROMOTION_EVIDENCE_V1 {
            return Err(error(
                PromotionErrorCode::UnsupportedVersion,
                format!(
                    "unsupported promotion evidence schema_version '{}'",
                    self.schema_version
                ),
            ));
        }
        validate_campaign_id(&self.campaign_id).map_err(|_| {
            error(
                PromotionErrorCode::InvalidCampaignId,
                format!("'{}' is not a backend-minted campaign id", self.campaign_id),
            )
        })?;
        if !is_valid_key(&self.candidate_key) {
            return Err(error(
                PromotionErrorCode::InvalidCandidateKey,
                format!("'{}' is not a valid candidate key", self.candidate_key),
            ));
        }
        for (field, value) in [
            ("baseline_job_id", &self.baseline_job_id),
            ("candidate_job_id", &self.candidate_job_id),
            ("baseline_run_id", &self.baseline_run_id),
            ("candidate_run_id", &self.candidate_run_id),
            ("candidate_session_id", &self.candidate_session_id),
        ] {
            if value.trim().is_empty() {
                return Err(error(
                    PromotionErrorCode::InconsistentEvidence,
                    format!("evidence declares an empty {field}"),
                ));
            }
        }
        for (field, value) in [
            ("baseline_artifact_hash", &self.baseline_artifact_hash),
            ("candidate_artifact_hash", &self.candidate_artifact_hash),
            ("baseline_evaluation_hash", &self.baseline_evaluation_hash),
            ("candidate_evaluation_hash", &self.candidate_evaluation_hash),
            ("baseline_verdict_hash", &self.baseline_verdict_hash),
            ("candidate_verdict_hash", &self.candidate_verdict_hash),
            ("fixture_digest", &self.fixture_digest),
            ("execution_profile_hash", &self.execution_profile_hash),
            ("patch_manifest_hash", &self.patch_manifest_hash),
            ("patch_sha256", &self.patch_sha256),
        ] {
            if !is_sha256_hex(value) {
                return Err(error(
                    PromotionErrorCode::MalformedDigest,
                    format!("evidence field '{field}' is not a sha256 hex digest"),
                ));
            }
        }
        for (field, value) in [
            ("fixture_ref", &self.fixture_ref),
            ("task_id", &self.task_id),
            ("suite", &self.suite),
            ("dataset_id", &self.dataset_id),
            ("split", &self.split),
            ("execution_profile_ref", &self.execution_profile_ref),
        ] {
            if value.trim().is_empty() {
                return Err(error(
                    PromotionErrorCode::InconsistentEvidence,
                    format!("evidence declares an empty {field}"),
                ));
            }
        }
        if self.dataset_version == 0 {
            return Err(error(
                PromotionErrorCode::InconsistentEvidence,
                "evidence declares dataset_version 0",
            ));
        }
        if self.base_revision.trim().is_empty() {
            return Err(error(
                PromotionErrorCode::InconsistentEvidence,
                "evidence declares an empty base revision",
            ));
        }
        if !matches!(
            self.verdict_status.as_str(),
            "pass" | "fail" | "not_evaluable"
        ) {
            return Err(error(
                PromotionErrorCode::InconsistentEvidence,
                format!("unsupported verdict status '{}'", self.verdict_status),
            ));
        }
        if !is_finite(self.verdict_score) {
            return Err(error(
                PromotionErrorCode::InconsistentEvidence,
                "evidence declares a non-finite verdict score",
            ));
        }
        if !matches!(self.verdict_safety_status.as_str(), "pass" | "fail") {
            return Err(error(
                PromotionErrorCode::InconsistentEvidence,
                format!("unsupported safety status '{}'", self.verdict_safety_status),
            ));
        }
        if !matches!(self.verdict_infra_status.as_str(), "pass" | "fail") {
            return Err(error(
                PromotionErrorCode::InconsistentEvidence,
                format!("unsupported infra status '{}'", self.verdict_infra_status),
            ));
        }
        if !matches!(self.verdict_cost_status.as_str(), "known" | "unknown") {
            return Err(error(
                PromotionErrorCode::InconsistentEvidence,
                format!(
                    "unsupported verdict cost status '{}'",
                    self.verdict_cost_status
                ),
            ));
        }
        self.budget.validate()?;
        self.verifier.validate()?;
        if self.metrics.is_empty() {
            return Err(error(
                PromotionErrorCode::InconsistentEvidence,
                "evidence declares no metric facts",
            ));
        }
        let mut seen: Vec<&str> = Vec::with_capacity(self.metrics.len());
        for metric in &self.metrics {
            metric.validate()?;
            if seen.contains(&metric.metric.as_str()) {
                return Err(error(
                    PromotionErrorCode::InconsistentEvidence,
                    format!("evidence declares metric '{}' twice", metric.metric),
                ));
            }
            seen.push(&metric.metric);
        }
        Ok(())
    }

    /// Canonical, domain-separated identity of the whole evidence projection.
    pub fn evidence_hash(&self) -> String {
        canonical_hash(
            PROMOTION_EVIDENCE_HASH_DOMAIN,
            &serde_json::to_value(self).unwrap_or(Value::Null),
        )
    }

    /// The metric fact for one metric key, if the projection declares it.
    pub fn metric(&self, metric: &str) -> Option<&PromotionMetricFactV1> {
        self.metrics.iter().find(|fact| fact.metric == metric)
    }
}

/// The strict submission body for one promotion attempt.
///
/// The CLI names the campaign, the candidate and an **opaque** target
/// reference. It never names a branch, a repository path, a Git identity, a
/// command or a threshold.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct PromotionRequestV1 {
    pub schema_version: String,
    pub campaign_id: String,
    pub candidate_key: String,
    pub target_ref: String,
    pub evidence: PromotionEvidenceV1,
}

impl PromotionRequestV1 {
    /// Validates the request and the nested evidence document.
    pub fn validate(&self) -> Result<(), PromotionError> {
        if self.schema_version != PROMOTION_REQUEST_V1 {
            return Err(error(
                PromotionErrorCode::UnsupportedVersion,
                format!(
                    "unsupported promotion request schema_version '{}'",
                    self.schema_version
                ),
            ));
        }
        if !is_valid_key(&self.target_ref) {
            return Err(error(
                PromotionErrorCode::UnknownPromotionTarget,
                format!(
                    "'{0}' is not a valid promotion target reference",
                    self.target_ref
                ),
            ));
        }
        self.evidence.validate()?;
        if self.campaign_id != self.evidence.campaign_id {
            return Err(error(
                PromotionErrorCode::InconsistentEvidence,
                "request campaign_id does not match the evidence campaign_id",
            ));
        }
        if self.candidate_key != self.evidence.candidate_key {
            return Err(error(
                PromotionErrorCode::InconsistentEvidence,
                "request candidate_key does not match the evidence candidate_key",
            ));
        }
        Ok(())
    }

    /// The digest-bound identity of one submission. Excludes volatile fields so
    /// the same submission always yields the same promotion identity.
    pub fn request_hash(&self) -> String {
        canonical_hash(
            PROMOTION_REQUEST_HASH_DOMAIN,
            &serde_json::json!({
                "campaign_id": self.campaign_id,
                "candidate_key": self.candidate_key,
                "target_ref": self.target_ref,
                "evidence_hash": self.evidence.evidence_hash(),
            }),
        )
    }

    /// The backend-minted promotion id this submission maps to.
    pub fn promotion_id(&self) -> String {
        promotion_id_for(
            &self.campaign_id,
            &self.candidate_key,
            &self.target_ref,
            &self.evidence.evidence_hash(),
        )
    }
}

// ---------------------------------------------------------------------------
// Paired canary result
// ---------------------------------------------------------------------------

/// One stage's structured, paired outcome. The runner recomputes `status` from
/// the numbers; the programme's own declaration is only cross-checked, so a
/// candidate-authored programme can never assert its own pass (INV-3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct CanaryStageResultV1 {
    pub stage_id: String,
    pub metric: String,
    pub samples: u32,
    pub baseline_passed: u32,
    pub candidate_passed: u32,
    pub baseline_mean: f64,
    pub candidate_mean: f64,
    /// `pass` or `fail`, as declared by the programme. Verified by the runner.
    pub status: String,
}

impl CanaryStageResultV1 {
    fn validate(&self) -> Result<(), PromotionError> {
        if !is_valid_key(&self.stage_id) {
            return Err(error(
                PromotionErrorCode::CanaryResultInvalid,
                format!(
                    "canary stage result has an invalid stage_id '{}'",
                    self.stage_id
                ),
            ));
        }
        if !is_valid_key(&self.metric) {
            return Err(error(
                PromotionErrorCode::CanaryResultInvalid,
                format!(
                    "canary stage '{}' declares an invalid metric",
                    self.stage_id
                ),
            ));
        }
        if self.samples == 0 {
            return Err(error(
                PromotionErrorCode::CanaryResultInvalid,
                format!("canary stage '{}' declares no samples", self.stage_id),
            ));
        }
        if self.baseline_passed > self.samples || self.candidate_passed > self.samples {
            return Err(error(
                PromotionErrorCode::CanaryResultInvalid,
                format!(
                    "canary stage '{}' declares more passing samples than samples",
                    self.stage_id
                ),
            ));
        }
        if !is_finite(self.baseline_mean) || !is_finite(self.candidate_mean) {
            return Err(error(
                PromotionErrorCode::CanaryResultInvalid,
                format!(
                    "canary stage '{}' declares a non-finite mean",
                    self.stage_id
                ),
            ));
        }
        if !matches!(self.status.as_str(), "pass" | "fail") {
            return Err(error(
                PromotionErrorCode::CanaryResultInvalid,
                format!(
                    "canary stage '{}' declares an unsupported status '{}'",
                    self.stage_id, self.status
                ),
            ));
        }
        Ok(())
    }
}

/// The single strict document a canary programme writes to stdout. Nothing else
/// a programme prints is parsed: the runner reads the whole stream, caps it,
/// hashes it and rejects anything that is not exactly this document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct CanaryResultV1 {
    pub schema_version: String,
    pub stages: Vec<CanaryStageResultV1>,
    /// `pass` or `fail`, as declared by the programme. Verified by the runner.
    pub status: String,
}

impl CanaryResultV1 {
    /// Parses and structurally validates a canary result document.
    pub fn parse(bytes: &[u8]) -> Result<Self, PromotionError> {
        if bytes.len() as u64 > MAX_CANARY_OUTPUT_BYTES {
            return Err(error(
                PromotionErrorCode::CanaryResultInvalid,
                format!(
                    "canary output is {} bytes, above the {MAX_CANARY_OUTPUT_BYTES} byte cap",
                    bytes.len()
                ),
            ));
        }
        let result: CanaryResultV1 = serde_json::from_slice(bytes).map_err(|parse_error| {
            error(
                PromotionErrorCode::CanaryResultInvalid,
                format!("canary output is not a strict canary result document: {parse_error}"),
            )
        })?;
        result.validate()?;
        Ok(result)
    }

    /// Full structural validation, including the fact that the stage list is
    /// ordered, unique and within the declared cap.
    pub fn validate(&self) -> Result<(), PromotionError> {
        if self.schema_version != CANARY_RESULT_V1 {
            return Err(error(
                PromotionErrorCode::UnsupportedVersion,
                format!(
                    "unsupported canary result schema_version '{}'",
                    self.schema_version
                ),
            ));
        }
        if !matches!(self.status.as_str(), "pass" | "fail") {
            return Err(error(
                PromotionErrorCode::CanaryResultInvalid,
                format!(
                    "canary result declares an unsupported status '{}'",
                    self.status
                ),
            ));
        }
        if self.stages.is_empty() {
            return Err(error(
                PromotionErrorCode::CanaryResultInvalid,
                "canary result declares no stages",
            ));
        }
        if self.stages.len() > MAX_CANARY_STAGES {
            return Err(error(
                PromotionErrorCode::CanaryResultInvalid,
                format!(
                    "canary result declares {} stages, above the {MAX_CANARY_STAGES} stage cap",
                    self.stages.len()
                ),
            ));
        }
        let mut seen: Vec<&str> = Vec::with_capacity(self.stages.len());
        for stage in &self.stages {
            stage.validate()?;
            if seen.contains(&stage.stage_id.as_str()) {
                return Err(error(
                    PromotionErrorCode::CanaryResultInvalid,
                    format!("canary result declares stage '{}' twice", stage.stage_id),
                ));
            }
            seen.push(&stage.stage_id);
        }
        Ok(())
    }

    /// Canonical, domain-separated identity of the whole result.
    pub fn result_hash(&self) -> String {
        canonical_hash(
            CANARY_RESULT_HASH_DOMAIN,
            &serde_json::to_value(self).unwrap_or(Value::Null),
        )
    }

    /// The stage result for one stage id, if the document declares it.
    pub fn stage(&self, stage_id: &str) -> Option<&CanaryStageResultV1> {
        self.stages.iter().find(|stage| stage.stage_id == stage_id)
    }
}

/// The ordered journal stages recorded in the promotion journal. Each stage is
/// written *before* its effect completes, so compensation always has a typed
/// starting point (INV-7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromotionJournalStage {
    /// The submission was persisted.
    Submitted,
    /// A worker claimed or adopted the promotion.
    Claimed,
    /// The evidence/policy gate reached a decision.
    PolicyDecided,
    /// The evidence/policy gate rejected the candidate.
    Rejected,
    /// A checkpoint intent is durable; the effect may or may not have happened.
    CheckpointIntent,
    /// The checkpoint commit and ref exist.
    CheckpointCreated,
    /// A canary intent is durable.
    CanaryIntent,
    /// The paired canary reached a structured outcome.
    CanaryCompleted,
    /// A branch advance intent is durable.
    AdvanceIntent,
    /// The registered branch now points at the checkpoint.
    BranchAdvanced,
    /// A post-CAS failure was compensated back to the old head.
    BranchRolledBack,
    /// The durable state cannot be resolved without a human.
    ParkedUnknownManual,
}

impl PromotionJournalStage {
    pub fn as_str(&self) -> &'static str {
        match self {
            PromotionJournalStage::Submitted => "submitted",
            PromotionJournalStage::Claimed => "claimed",
            PromotionJournalStage::PolicyDecided => "policy_decided",
            PromotionJournalStage::Rejected => "rejected",
            PromotionJournalStage::CheckpointIntent => "checkpoint_intent",
            PromotionJournalStage::CheckpointCreated => "checkpoint_created",
            PromotionJournalStage::CanaryIntent => "canary_intent",
            PromotionJournalStage::CanaryCompleted => "canary_completed",
            PromotionJournalStage::AdvanceIntent => "advance_intent",
            PromotionJournalStage::BranchAdvanced => "branch_advanced",
            PromotionJournalStage::BranchRolledBack => "branch_rolled_back",
            PromotionJournalStage::ParkedUnknownManual => "parked_unknown_manual",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "submitted" => PromotionJournalStage::Submitted,
            "claimed" => PromotionJournalStage::Claimed,
            "policy_decided" => PromotionJournalStage::PolicyDecided,
            "rejected" => PromotionJournalStage::Rejected,
            "checkpoint_intent" => PromotionJournalStage::CheckpointIntent,
            "checkpoint_created" => PromotionJournalStage::CheckpointCreated,
            "canary_intent" => PromotionJournalStage::CanaryIntent,
            "canary_completed" => PromotionJournalStage::CanaryCompleted,
            "advance_intent" => PromotionJournalStage::AdvanceIntent,
            "branch_advanced" => PromotionJournalStage::BranchAdvanced,
            "branch_rolled_back" => PromotionJournalStage::BranchRolledBack,
            "parked_unknown_manual" => PromotionJournalStage::ParkedUnknownManual,
            _ => return None,
        })
    }
}

/// One durable journal entry, as stored. `sequence` is SQLite's monotonic
/// auto-increment, so the journal is ordered by database allocation rather than
/// by a wall clock.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct PromotionJournalEntryV1 {
    pub sequence: i64,
    pub stage: String,
    pub owner_id: Option<String>,
    pub lease_generation: i64,
    pub detail: Option<String>,
    pub created_at_ms: u64,
}

/// Canonical digest of an ordered journal. Two journals with the same entries in
/// the same sequence order hash identically, so the audit can prove the ordering
/// without re-reading the database.
pub fn journal_digest(entries: &[PromotionJournalEntryV1]) -> String {
    canonical_hash(
        PROMOTION_JOURNAL_HASH_DOMAIN,
        &serde_json::to_value(entries).unwrap_or(Value::Null),
    )
}

// ---------------------------------------------------------------------------
// Read-only projections (audit + operator surface)
// ---------------------------------------------------------------------------

/// Hash algorithm literal shared by every Phase 2I digest.
pub const HASH_ALGORITHM: &str = "sha256";

/// Hash domain of the audit bundle integrity digest.
pub const PROMOTION_AUDIT_HASH_DOMAIN: &str = "cs-promotion:audit";

/// One persisted canary stage, as projected for the audit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct PromotionStageProjectionV1 {
    pub stage_index: u32,
    pub stage_id: String,
    pub metric: String,
    pub samples: u32,
    pub baseline_passed: u32,
    pub candidate_passed: u32,
    pub baseline_mean: f64,
    pub candidate_mean: f64,
    pub declared_status: String,
    pub recomputed_status: String,
    pub output_sha256: String,
}

/// The recorded policy decision, projected for the audit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct PromotionDecisionProjectionV1 {
    pub outcome: String,
    pub code: String,
    pub detail: String,
}

/// The promotion status projection. It carries refs, digests and typed state
/// only: never a secret, a transcript or a raw stream (INV-8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct PromotionProjectionV1 {
    pub schema_version: String,
    pub promotion_id: String,
    pub campaign_id: String,
    pub candidate_key: String,
    pub target_ref: String,
    pub state: String,
    pub request_hash: String,
    pub evidence_hash: String,
    pub target_hash: String,
    pub policy_hash: String,
    pub base_revision: String,
    pub patch_sha256: String,
    pub patch_manifest_hash: String,
    pub expected_old_head: Option<String>,
    pub observed_head: Option<String>,
    pub checkpoint_commit: Option<String>,
    pub checkpoint_ref: Option<String>,
    pub checkpoint_intent: String,
    pub branch_intent: String,
    pub canary_result_hash: Option<String>,
    pub decision: Option<PromotionDecisionProjectionV1>,
    pub error_code: Option<String>,
    pub lease_generation: i64,
    pub attempt: u32,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub stages: Vec<PromotionStageProjectionV1>,
}

/// The evidence-only reconcile projection: what the pure classifier would do
/// with this row, plus the journal that proves what already happened.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct PromotionReconcileV1 {
    pub schema_version: String,
    pub promotion: PromotionProjectionV1,
    pub recovery: String,
    pub journal: Vec<PromotionJournalEntryV1>,
    pub journal_digest: String,
}

/// The audit bundle's own integrity block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct AuditIntegrityV1 {
    pub algorithm: String,
    pub audit_hash: String,
}

/// The offline-verifiable audit bundle document.
///
/// It contains everything needed to reconstruct
/// `verdict → policy → checkpoint → canary → branch transition` without the
/// database, the daemon or any container: the immutable evidence projection,
/// the target and policy identities, the policy decision, the checkpoint
/// ref/commit, the paired canary stages, the branch transition and the ordered
/// journal. `integrity.audit_hash` covers every field except `created_at`, so a
/// tampered bundle is detectable offline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct PromotionAuditV1 {
    pub schema_version: String,
    pub promotion: PromotionProjectionV1,
    pub evidence: PromotionEvidenceV1,
    pub decision: Option<PromotionDecisionProjectionV1>,
    pub branch_ref: Option<String>,
    pub journal: Vec<PromotionJournalEntryV1>,
    pub journal_digest: String,
    pub integrity: AuditIntegrityV1,
    pub created_at: String,
}

/// Recomputes the audit bundle's canonical digest over every field except
/// `created_at` and `integrity` (neither is part of the claim).
pub fn audit_hash(document: &Value) -> String {
    let mut content = document.clone();
    if let Some(map) = content.as_object_mut() {
        map.remove("created_at");
        map.remove("integrity");
    }
    canonical_hash(PROMOTION_AUDIT_HASH_DOMAIN, &content)
}

/// Verifies an audit document offline: the schema version and shape, then the
/// integrity digest and the journal ordering. A mismatch means the bundle was
/// tampered with.
pub fn verify_audit_document(document: &Value) -> Result<PromotionAuditV1, PromotionError> {
    let audit: PromotionAuditV1 =
        serde_json::from_value(document.clone()).map_err(|parse_error| {
            error(
                PromotionErrorCode::InvalidPromotionState,
                format!("the audit bundle is not a strict document: {parse_error}"),
            )
        })?;
    if audit.schema_version != PROMOTION_AUDIT_V1 {
        return Err(error(
            PromotionErrorCode::UnsupportedVersion,
            format!(
                "unsupported audit schema_version '{}'",
                audit.schema_version
            ),
        ));
    }
    if audit.integrity.algorithm != HASH_ALGORITHM {
        return Err(error(
            PromotionErrorCode::InvalidPromotionState,
            format!(
                "unsupported audit hash algorithm '{}'",
                audit.integrity.algorithm
            ),
        ));
    }
    if audit_hash(document) != audit.integrity.audit_hash {
        return Err(error(
            PromotionErrorCode::InvalidPromotionState,
            "the audit bundle does not match its own integrity digest",
        ));
    }
    // The journal digest is re-derived from the entries, so the ordering claim is
    // checked, not trusted.
    if journal_digest(&audit.journal) != audit.journal_digest {
        return Err(error(
            PromotionErrorCode::InvalidPromotionState,
            "the audit bundle's journal digest does not match its journal",
        ));
    }
    audit.evidence.validate()?;
    Ok(audit)
}

/// The single-arm measurement a canary programme emits for **one** arm and
/// **one** stage.
///
/// The programme never sees, and never declares, the paired comparison: the
/// runner asks each arm separately, then assembles the pair itself. That is what
/// makes a candidate-authored programme unable to assert its own pass (INV-3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct CanaryArmSampleV1 {
    pub schema_version: String,
    pub stage_id: String,
    pub metric: String,
    /// Independent samples this arm produced for the stage.
    pub samples: u32,
    /// How many of them met the stage's own pass condition.
    pub passed: u32,
    /// Mean metric value over this arm's samples.
    pub mean: f64,
}

impl CanaryArmSampleV1 {
    /// Parses and structurally validates one arm sample, with the same output
    /// cap as a full result document.
    pub fn parse(bytes: &[u8]) -> Result<Self, PromotionError> {
        if bytes.len() as u64 > MAX_CANARY_OUTPUT_BYTES {
            return Err(error(
                PromotionErrorCode::CanaryResultInvalid,
                format!(
                    "canary arm output is {} bytes, above the {MAX_CANARY_OUTPUT_BYTES} byte cap",
                    bytes.len()
                ),
            ));
        }
        let sample: CanaryArmSampleV1 = serde_json::from_slice(bytes).map_err(|parse_error| {
            error(
                PromotionErrorCode::CanaryResultInvalid,
                format!("canary arm output is not a strict arm sample document: {parse_error}"),
            )
        })?;
        sample.validate()?;
        Ok(sample)
    }

    pub fn validate(&self) -> Result<(), PromotionError> {
        if self.schema_version != CANARY_ARM_SAMPLE_V1 {
            return Err(error(
                PromotionErrorCode::UnsupportedVersion,
                format!(
                    "unsupported canary arm sample schema_version '{}'",
                    self.schema_version
                ),
            ));
        }
        if !is_valid_key(&self.stage_id) {
            return Err(error(
                PromotionErrorCode::CanaryResultInvalid,
                format!(
                    "canary arm sample has an invalid stage_id '{}'",
                    self.stage_id
                ),
            ));
        }
        if !is_valid_key(&self.metric) {
            return Err(error(
                PromotionErrorCode::CanaryResultInvalid,
                format!(
                    "canary arm sample for '{}' has an invalid metric",
                    self.stage_id
                ),
            ));
        }
        if self.samples == 0 {
            return Err(error(
                PromotionErrorCode::CanaryResultInvalid,
                format!(
                    "canary arm sample for '{}' declares no samples",
                    self.stage_id
                ),
            ));
        }
        if self.passed > self.samples {
            return Err(error(
                PromotionErrorCode::CanaryResultInvalid,
                format!(
                    "canary arm sample for '{}' declares more passing samples than samples",
                    self.stage_id
                ),
            ));
        }
        if !is_finite(self.mean) {
            return Err(error(
                PromotionErrorCode::CanaryResultInvalid,
                format!(
                    "canary arm sample for '{}' declares a non-finite mean",
                    self.stage_id
                ),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn metric(metric: &str, samples: u32, baseline: f64, candidate: f64) -> PromotionMetricFactV1 {
        PromotionMetricFactV1 {
            metric: metric.to_string(),
            samples,
            baseline_passed: samples / 2,
            candidate_passed: samples,
            baseline_mean: baseline,
            candidate_mean: candidate,
        }
    }

    fn evidence() -> PromotionEvidenceV1 {
        PromotionEvidenceV1 {
            schema_version: PROMOTION_EVIDENCE_V1.to_string(),
            campaign_id: "camp-0123456789abcdef0123456789abcdef".to_string(),
            candidate_key: "prompt-a".to_string(),
            baseline_job_id: "job-baseline".to_string(),
            candidate_job_id: "job-candidate".to_string(),
            baseline_run_id: "run-baseline".to_string(),
            candidate_run_id: "run-candidate".to_string(),
            candidate_session_id: "session-candidate".to_string(),
            baseline_artifact_hash: "a".repeat(64),
            candidate_artifact_hash: "b".repeat(64),
            baseline_evaluation_hash: "c".repeat(64),
            candidate_evaluation_hash: "d".repeat(64),
            baseline_verdict_hash: "e".repeat(64),
            candidate_verdict_hash: "f".repeat(64),
            fixture_ref: "smoke-tools".to_string(),
            fixture_digest: "1".repeat(64),
            task_id: "task-a".to_string(),
            suite: "chatspeed-smoke".to_string(),
            dataset_id: "chatspeed-smoke".to_string(),
            dataset_version: 2,
            split: "smoke".to_string(),
            execution_profile_ref: "smoke-local".to_string(),
            execution_profile_hash: "2".repeat(64),
            patch_manifest_hash: "3".repeat(64),
            patch_sha256: "4".repeat(64),
            base_revision: "refs/heads/main".to_string(),
            verdict_status: "pass".to_string(),
            verdict_score: 1.0,
            verdict_safety_status: "pass".to_string(),
            verdict_infra_status: "pass".to_string(),
            verdict_cost_status: "known".to_string(),
            budget: PromotionBudgetFactsV1 {
                cost_status: "known".to_string(),
                budget_rejected: false,
                committed_micros: 0,
                currency: "usd".to_string(),
            },
            verifier: PromotionVerifierIdentityV1 {
                verifier_id: "chatspeed-smoke".to_string(),
                verifier_version: "2".to_string(),
                verifier_digest: "5".repeat(64),
            },
            metrics: vec![metric("verdict_score", 4, 0.5, 1.0)],
        }
    }

    #[test]
    fn a_valid_evidence_projection_round_trips_strictly() {
        let evidence = evidence();
        evidence.validate().expect("valid");
        let text = serde_json::to_string(&evidence).expect("serialize");
        let parsed: PromotionEvidenceV1 = serde_json::from_str(&text).expect("deserialize");
        assert_eq!(parsed, evidence);
        assert_eq!(parsed.evidence_hash(), evidence.evidence_hash());
        assert!(is_sha256_hex(&evidence.evidence_hash()));
    }

    #[test]
    fn evidence_hash_is_key_order_independent_but_value_sensitive() {
        let evidence = evidence();
        let mut reordered = serde_json::to_value(&evidence).expect("value");
        reordered = json!({
            "metrics": reordered["metrics"].clone(),
            "verifier": reordered["verifier"].clone(),
            "budget": reordered["budget"].clone(),
            "schema_version": reordered["schema_version"].clone(),
            "campaign_id": reordered["campaign_id"].clone(),
            "candidate_key": reordered["candidate_key"].clone(),
        });
        // Only a strict subset of keys: the canonical hash of a partial
        // document must differ from the full one, which proves the hash covers
        // every field rather than a fixed prefix.
        assert_ne!(
            canonical_hash(PROMOTION_EVIDENCE_HASH_DOMAIN, &reordered),
            evidence.evidence_hash()
        );

        let mut tampered = evidence.clone();
        tampered.candidate_artifact_hash = "9".repeat(64);
        assert_ne!(tampered.evidence_hash(), evidence.evidence_hash());
    }

    #[test]
    fn an_unknown_field_in_the_evidence_is_rejected() {
        let mut value = serde_json::to_value(evidence()).expect("value");
        value["promotion_policy"] = json!({ "min_samples": 1 });
        let parse_error = serde_json::from_value::<PromotionEvidenceV1>(value)
            .expect_err("unknown field must be rejected");
        assert!(parse_error.to_string().contains("unknown field"));
    }

    #[test]
    fn a_caller_minted_campaign_id_is_rejected() {
        let mut evidence = evidence();
        evidence.campaign_id = "campaign-1".to_string();
        let rejected = evidence.validate().expect_err("caller-minted id");
        assert_eq!(rejected.code, PromotionErrorCode::InvalidCampaignId);
    }

    #[test]
    fn malformed_digests_and_non_finite_means_are_rejected() {
        let mut document = evidence();
        document.patch_sha256 = "NOT-A-DIGEST".to_string();
        assert_eq!(
            document.validate().expect_err("digest").code,
            PromotionErrorCode::MalformedDigest
        );

        let mut document = evidence();
        document.metrics = vec![metric("verdict_score", 4, f64::NAN, 1.0)];
        assert_eq!(
            document.validate().expect_err("nan").code,
            PromotionErrorCode::InconsistentEvidence
        );

        let mut document = evidence();
        document.metrics = vec![metric("verdict_score", 0, 0.0, 1.0)];
        assert_eq!(
            document.validate().expect_err("no samples").code,
            PromotionErrorCode::InsufficientSamples
        );

        let mut document = evidence();
        document.metrics = vec![
            metric("verdict_score", 4, 0.5, 1.0),
            metric("verdict_score", 4, 0.5, 1.0),
        ];
        assert_eq!(
            document.validate().expect_err("duplicate").code,
            PromotionErrorCode::InconsistentEvidence
        );
    }

    #[test]
    fn an_incomplete_or_mismatched_request_is_rejected() {
        let request = PromotionRequestV1 {
            schema_version: PROMOTION_REQUEST_V1.to_string(),
            campaign_id: "camp-0123456789abcdef0123456789abcdef".to_string(),
            candidate_key: "prompt-a".to_string(),
            target_ref: "local-dev".to_string(),
            evidence: evidence(),
        };
        request.validate().expect("valid request");
        assert!(is_sha256_hex(&request.request_hash()));
        assert!(request.promotion_id().starts_with("promo-"));
        validate_promotion_id(&request.promotion_id()).expect("minted id");

        let mut mismatched = request.clone();
        mismatched.candidate_key = "prompt-b".to_string();
        assert_eq!(
            mismatched.validate().expect_err("mismatch").code,
            PromotionErrorCode::InconsistentEvidence
        );

        let mut injected = request.clone();
        injected.target_ref = "../escape".to_string();
        assert_eq!(
            injected.validate().expect_err("target ref").code,
            PromotionErrorCode::UnknownPromotionTarget
        );
    }

    #[test]
    fn the_promotion_id_is_deterministic_and_evidence_bound() {
        let request = PromotionRequestV1 {
            schema_version: PROMOTION_REQUEST_V1.to_string(),
            campaign_id: "camp-0123456789abcdef0123456789abcdef".to_string(),
            candidate_key: "prompt-a".to_string(),
            target_ref: "local-dev".to_string(),
            evidence: evidence(),
        };
        let mut other = request.clone();
        other.evidence.base_revision = "refs/heads/dev".to_string();
        assert_eq!(request.promotion_id(), request.promotion_id());
        assert_ne!(request.promotion_id(), other.promotion_id());
        assert_eq!(
            checkpoint_ref_for(&request.promotion_id()),
            format!("refs/chatspeed/checkpoints/{}", request.promotion_id())
        );
    }

    #[test]
    fn the_fsm_only_allows_the_documented_edges() {
        use PromotionState::*;
        assert!(transition_allowed(Queued, EvidenceValidating));
        assert!(transition_allowed(EvidenceValidating, Rejected));
        assert!(transition_allowed(EvidenceValidating, Checkpointing));
        assert!(transition_allowed(Checkpointing, Checkpointed));
        assert!(transition_allowed(Checkpointed, CanaryRunning));
        assert!(transition_allowed(CanaryRunning, CanaryFailed));
        assert!(transition_allowed(CanaryRunning, ReadyToAdvance));
        assert!(transition_allowed(ReadyToAdvance, Advancing));
        assert!(transition_allowed(Advancing, Promoted));
        assert!(transition_allowed(Advancing, RolledBack));

        // A failed canary can never reach the branch CAS.
        assert!(!transition_allowed(CanaryFailed, ReadyToAdvance));
        assert!(!transition_allowed(Rejected, Checkpointing));
        assert!(!transition_allowed(CanaryFailed, Advancing));
        // A checkpoint can never skip the canary.
        assert!(!transition_allowed(Checkpointed, Advancing));
        assert!(!transition_allowed(Checkpointed, ReadyToAdvance));
        // Terminal states only re-write themselves.
        assert!(transition_allowed(Promoted, Promoted));
        assert!(!transition_allowed(Promoted, Queued));
        assert_eq!(
            validate_transition(Promoted, Queued)
                .expect_err("illegal")
                .code,
            PromotionErrorCode::InvalidPromotionTransition
        );
    }

    #[test]
    fn every_state_string_round_trips() {
        for state in [
            PromotionState::Queued,
            PromotionState::EvidenceValidating,
            PromotionState::Rejected,
            PromotionState::Checkpointing,
            PromotionState::Checkpointed,
            PromotionState::CanaryRunning,
            PromotionState::CanaryFailed,
            PromotionState::ReadyToAdvance,
            PromotionState::Advancing,
            PromotionState::Promoted,
            PromotionState::RolledBack,
            PromotionState::UnknownManual,
        ] {
            assert_eq!(PromotionState::parse(state.as_str()), Some(state));
        }
        assert_eq!(PromotionState::parse("nonsense"), None);
        for intent in [
            EffectIntent::NotStarted,
            EffectIntent::IntentRecorded,
            EffectIntent::Completed,
        ] {
            assert_eq!(EffectIntent::parse(intent.as_str()), Some(intent));
        }
        for observation in [
            CheckpointObservation::Absent,
            CheckpointObservation::PresentConsistent,
            CheckpointObservation::PresentInconsistent,
        ] {
            assert_eq!(
                CheckpointObservation::parse(observation.as_str()),
                Some(observation)
            );
        }
        for observation in [
            BranchObservation::AtOld,
            BranchObservation::AtCheckpoint,
            BranchObservation::ThirdValue,
        ] {
            assert_eq!(
                BranchObservation::parse(observation.as_str()),
                Some(observation)
            );
        }
    }

    #[test]
    fn restart_recovery_never_replays_an_unknown_effect() {
        use CheckpointObservation as Co;
        use EffectIntent as Ei;
        use PromotionRecoveryDecision as D;
        use PromotionState as S;

        assert_eq!(
            classify_promotion_recovery(
                S::Promoted,
                Co::PresentConsistent,
                BranchObservation::AtCheckpoint,
                Ei::Completed
            ),
            D::Terminal
        );
        assert_eq!(
            classify_promotion_recovery(
                S::Rejected,
                Co::Absent,
                BranchObservation::AtOld,
                Ei::NotStarted
            ),
            D::Terminal
        );
        assert_eq!(
            classify_promotion_recovery(
                S::Queued,
                Co::Absent,
                BranchObservation::AtOld,
                Ei::NotStarted
            ),
            D::ResumeEvidence
        );
        // Checkpoint intent without a visible ref: safe to recreate.
        assert_eq!(
            classify_promotion_recovery(
                S::Checkpointing,
                Co::Absent,
                BranchObservation::AtOld,
                Ei::IntentRecorded
            ),
            D::ResumeCheckpoint
        );
        // A foreign checkpoint ref is never adopted.
        assert_eq!(
            classify_promotion_recovery(
                S::Checkpointing,
                Co::PresentInconsistent,
                BranchObservation::AtOld,
                Ei::IntentRecorded
            ),
            D::ParkUnknown
        );
        assert_eq!(
            classify_promotion_recovery(
                S::Checkpointed,
                Co::PresentConsistent,
                BranchObservation::AtOld,
                Ei::Completed
            ),
            D::ResumeCanary
        );
        // The row claims a checkpoint the repository cannot prove.
        assert_eq!(
            classify_promotion_recovery(
                S::Checkpointed,
                Co::Absent,
                BranchObservation::AtOld,
                Ei::Completed
            ),
            D::ParkUnknown
        );
        assert_eq!(
            classify_promotion_recovery(
                S::ReadyToAdvance,
                Co::PresentConsistent,
                BranchObservation::AtOld,
                Ei::Completed
            ),
            D::Advance
        );
        assert_eq!(
            classify_promotion_recovery(
                S::Advancing,
                Co::PresentConsistent,
                BranchObservation::AtCheckpoint,
                Ei::IntentRecorded
            ),
            D::RollForward
        );
        // A third value means an external actor moved the branch: never overwrite.
        assert_eq!(
            classify_promotion_recovery(
                S::Advancing,
                Co::PresentConsistent,
                BranchObservation::ThirdValue,
                Ei::IntentRecorded
            ),
            D::ParkUnknown
        );
        assert_eq!(
            classify_promotion_recovery(
                S::ReadyToAdvance,
                Co::PresentConsistent,
                BranchObservation::ThirdValue,
                Ei::Completed
            ),
            D::ParkUnknown
        );
    }

    #[test]
    fn the_owner_token_binds_promotion_owner_and_generation() {
        let fence = PromotionFence::new("headless", 3);
        let token = fence.token_hash("promo-0123456789abcdef0123456789abcdef");
        assert_eq!(
            token,
            PromotionFence::new("headless", 3).token_hash("promo-0123456789abcdef0123456789abcdef")
        );
        assert_ne!(
            token,
            PromotionFence::new("headless", 4).token_hash("promo-0123456789abcdef0123456789abcdef")
        );
        assert_ne!(
            token,
            fence.token_hash("promo-ffffffffffffffffffffffffffffffff")
        );
    }

    #[test]
    fn branch_refs_are_strict() {
        assert!(is_full_branch_ref("refs/heads/main"));
        assert!(is_full_branch_ref("refs/heads/experiment/2i-promotion"));
        assert!(!is_full_branch_ref("main"));
        assert!(!is_full_branch_ref("refs/remotes/origin/main"));
        assert!(!is_full_branch_ref("refs/tags/v1"));
        assert!(!is_full_branch_ref("refs/heads/../escape"));
        assert!(!is_full_branch_ref("refs/heads/main.lock"));
        assert!(!is_full_branch_ref("refs/heads/"));
        assert!(!is_full_branch_ref("refs/heads/-flag"));
        assert!(!is_full_branch_ref("refs/heads/a b"));
        assert!(!is_full_branch_ref("refs/heads/a~b"));
        assert!(!is_full_branch_ref("refs/heads/a:b"));
    }

    #[test]
    fn a_canary_result_round_trips_and_rejects_bad_documents() {
        let result = CanaryResultV1 {
            schema_version: CANARY_RESULT_V1.to_string(),
            stages: vec![CanaryStageResultV1 {
                stage_id: "stage-1".to_string(),
                metric: "verdict_score".to_string(),
                samples: 4,
                baseline_passed: 2,
                candidate_passed: 4,
                baseline_mean: 0.5,
                candidate_mean: 1.0,
                status: "pass".to_string(),
            }],
            status: "pass".to_string(),
        };
        let bytes = serde_json::to_vec(&result).expect("serialize");
        let parsed = CanaryResultV1::parse(&bytes).expect("parse");
        assert_eq!(parsed, result);
        assert!(is_sha256_hex(&parsed.result_hash()));

        // Malformed JSON, unknown fields, oversized output and an unsupported
        // status are all rejected before any comparison.
        assert_eq!(
            CanaryResultV1::parse(b"{ not json }")
                .expect_err("malformed")
                .code,
            PromotionErrorCode::CanaryResultInvalid
        );
        let oversized = vec![b' '; (MAX_CANARY_OUTPUT_BYTES + 1) as usize];
        assert_eq!(
            CanaryResultV1::parse(&oversized)
                .expect_err("oversize")
                .code,
            PromotionErrorCode::CanaryResultInvalid
        );

        let mut value = serde_json::to_value(&result).expect("value");
        value["extra"] = json!(true);
        assert!(CanaryResultV1::parse(&serde_json::to_vec(&value).expect("bytes")).is_err());

        let mut value = serde_json::to_value(&result).expect("value");
        value["stages"][0]["status"] = json!("maybe");
        assert!(CanaryResultV1::parse(&serde_json::to_vec(&value).expect("bytes")).is_err());

        let mut value = serde_json::to_value(&result).expect("value");
        value["schema_version"] = json!("canary_result.v2");
        assert!(CanaryResultV1::parse(&serde_json::to_vec(&value).expect("bytes")).is_err());

        let mut value = serde_json::to_value(&result).expect("value");
        value["stages"] = json!([]);
        assert!(CanaryResultV1::parse(&serde_json::to_vec(&value).expect("bytes")).is_err());

        let mut duplicated = result.clone();
        duplicated.stages.push(duplicated.stages[0].clone());
        assert_eq!(
            duplicated.validate().expect_err("duplicate").code,
            PromotionErrorCode::CanaryResultInvalid
        );
    }
}
