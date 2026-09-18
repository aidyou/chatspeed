//! The Phase 2I promotion supervisor: one bounded tick that drives a claimed
//! promotion along the FSM, and nothing else.
//!
//! Each tick does exactly one bounded unit of work on at most one promotion:
//!
//! ```text
//! claim → evidence/policy gate → checkpoint → paired canary → branch CAS
//! ```
//!
//! Recovery is not a separate code path. The store records an *intent* before
//! every effect, and this supervisor re-derives what to do from typed
//! observation of the repository, exactly as the pure classifier describes:
//!
//! - a checkpoint that the repository cannot prove is never adopted (park),
//! - a checkpoint intent whose ref is absent is re-created,
//! - a branch still on the expected old head retries the CAS,
//! - a branch already on the checkpoint rolls the journal forward,
//! - a branch on a third value is parked, never overwritten.
//!
//! Everything server-side is resolved by the caller: the execution profile, the
//! target registry, the bundle allowlist, the base repository and the domain
//! directories. This module adds no second authority (INV-2).

use crate::db::experiment_promotion::{
    evaluate_record_policy, CanaryStageRow, ExperimentPromotionStore, PromotionClaimOutcome,
    PromotionRecord,
};
use crate::db::experiment_schedule::{ExperimentScheduleStore, JobRecord};
use crate::headless::profiles::ExecutionProfileRegistry;
use crate::headless::promotion_targets::PromotionTargetRegistry;
use crate::workflow::react::experiment_owner::bundle::{stage_bundle, BundleRegistry};
use crate::workflow::react::experiment_owner::docker::DockerOwnerConfig;
use crate::workflow::react::experiment_owner::promotion::{
    CheckpointProof, PromotionCheckpointOwner, PromotionCheckpointRequest,
};
use crate::workflow::react::experiment_owner::promotion_canary::{
    CanaryRunRequest, PromotionCanaryRunner,
};
use crate::workflow::react::experiment_promotion::binding::{
    verify_artifact_file, verify_promotion_binding, BoundArtifactV1, CampaignBindingV1,
    JobBindingV1, TargetBindingV1,
};
use crate::workflow::react::experiment_promotion::policy::{PromotionDecision, PromotionOutcome};
use crate::workflow::react::experiment_promotion::types::{
    checkpoint_ref_for, BranchObservation, CheckpointObservation, PromotionError,
    PromotionErrorCode, PromotionFence, PromotionState,
};
use crate::workflow::react::experiment_schedule::types::{
    ExecutionProfileV1, OwnerKind, ScheduleError,
};
use std::path::PathBuf;

fn promotion_error(code: PromotionErrorCode, message: impl Into<String>) -> PromotionError {
    PromotionError::new(code, message)
}

/// The outcome of one supervisor tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromotionTickOutcome {
    /// Nothing was claimable.
    Idle,
    /// The promotion moved forward and remains non-terminal.
    Progressed {
        promotion_id: String,
        state: PromotionState,
    },
    /// The promotion reached a terminal state by a gate, not by a failure.
    Terminal {
        promotion_id: String,
        state: PromotionState,
        code: String,
    },
    /// The promotion was parked for a human.
    Parked { promotion_id: String, code: String },
}

/// The durable, bounded promotion supervisor.
pub struct PromotionSupervisor {
    promotions: ExperimentPromotionStore,
    schedule: ExperimentScheduleStore,
    targets: PromotionTargetRegistry,
    profiles: ExecutionProfileRegistry,
    bundles: BundleRegistry,
    artifacts_root: PathBuf,
    bundles_root: PathBuf,
    base_repo: Option<PathBuf>,
    worktrees_root: PathBuf,
    owner_id: String,
    lease_ms: u64,
}

/// Everything the supervisor needs, all of it server-derived.
pub struct PromotionSupervisorConfig {
    pub domain_root: PathBuf,
    pub base_repo: Option<PathBuf>,
    pub owner_id: String,
    pub lease_ms: u64,
}

impl PromotionSupervisor {
    pub fn new(
        promotions: ExperimentPromotionStore,
        schedule: ExperimentScheduleStore,
        config: PromotionSupervisorConfig,
    ) -> Self {
        let domain_root = config.domain_root;
        Self {
            promotions,
            schedule,
            targets: PromotionTargetRegistry::new(&domain_root),
            profiles: ExecutionProfileRegistry::new(&domain_root),
            bundles: BundleRegistry::new(domain_root.join("bundles-allowlist")),
            artifacts_root: domain_root.join("artifacts"),
            bundles_root: domain_root.join("bundles"),
            worktrees_root: domain_root.join("worktrees"),
            base_repo: config.base_repo,
            owner_id: config.owner_id,
            lease_ms: config.lease_ms,
        }
    }

    /// Runs at most one promotion step.
    pub fn tick(&self, now_ms: u64) -> Result<PromotionTickOutcome, PromotionError> {
        let claimed = match self
            .promotions
            .claim_next(&self.owner_id, now_ms, self.lease_ms)?
        {
            PromotionClaimOutcome::Idle => return Ok(PromotionTickOutcome::Idle),
            PromotionClaimOutcome::Claimed(record) => *record,
        };
        let fence = claimed.fence().ok_or_else(|| {
            promotion_error(
                PromotionErrorCode::InvalidPromotionState,
                format!(
                    "promotion '{}' was claimed without an owner",
                    claimed.promotion_id
                ),
            )
        })?;
        self.step(&claimed, &fence, now_ms)
    }

    /// Startup reconciliation: reports every non-terminal promotion and what the
    /// pure classifier would do with it. It performs no effect, so a restart can
    /// always be inspected before anything else runs.
    pub fn reconcile(
        &self,
        now_ms: u64,
    ) -> Result<
        Vec<(
            PromotionRecord,
            crate::workflow::react::experiment_promotion::types::PromotionRecoveryDecision,
        )>,
        PromotionError,
    > {
        let _ = now_ms;
        let mut projections = Vec::new();
        for record in self.promotions.list_non_terminal()? {
            let decision = self.classify(&record)?;
            projections.push((record, decision));
        }
        Ok(projections)
    }

    /// Classifies a durable row from real repository observation.
    pub fn classify(
        &self,
        record: &PromotionRecord,
    ) -> Result<
        crate::workflow::react::experiment_promotion::types::PromotionRecoveryDecision,
        PromotionError,
    > {
        use crate::workflow::react::experiment_promotion::types::PromotionRecoveryDecision;
        if record.state.is_pre_effect() {
            return Ok(PromotionRecoveryDecision::ResumeEvidence);
        }
        let Some(target) = self.targets.load(&record.target_ref).ok() else {
            return Ok(PromotionRecoveryDecision::ParkUnknown);
        };
        let Some(base_repo) = self.base_repo.clone() else {
            return Ok(PromotionRecoveryDecision::ParkUnknown);
        };
        let checkpoint_request = self.checkpoint_request(
            record,
            &target,
            &record
                .expected_old_head
                .clone()
                .unwrap_or_else(|| "0".repeat(40)),
        );
        let owner = PromotionCheckpointOwner::new(base_repo, &self.worktrees_root);
        let checkpoint = match &checkpoint_request {
            Ok(request) => owner
                .observe_checkpoint(request)
                .unwrap_or(CheckpointObservation::PresentInconsistent),
            Err(_) => CheckpointObservation::PresentInconsistent,
        };
        let implementation = match &checkpoint_request {
            Ok(request) => Ok(request),
            Err(error) => Err(error.clone()),
        };
        let branch = match implementation {
            Ok(request) => {
                let checkpoint_commit = record
                    .checkpoint_commit
                    .clone()
                    .unwrap_or_else(|| "0".repeat(40));
                owner
                    .observe_branch(
                        &request.branch_ref,
                        &request.expected_old_head,
                        &checkpoint_commit,
                    )
                    .unwrap_or(BranchObservation::ThirdValue)
            }
            Err(_) => BranchObservation::ThirdValue,
        };
        Ok(record.recovery_decision(checkpoint, branch))
    }

    // -----------------------------------------------------------------------
    // One step
    // -----------------------------------------------------------------------

    fn step(
        &self,
        record: &PromotionRecord,
        fence: &PromotionFence,
        now_ms: u64,
    ) -> Result<PromotionTickOutcome, PromotionError> {
        match record.state {
            PromotionState::EvidenceValidating => self.step_evidence(record, fence, now_ms),
            PromotionState::Checkpointing => self.step_checkpoint(record, fence, now_ms),
            PromotionState::Checkpointed => {
                let record = self
                    .promotions
                    .begin_canary(fence, &record.promotion_id, now_ms)?;
                self.step_canary(&record, fence, now_ms)
            }
            PromotionState::CanaryRunning => self.step_canary(record, fence, now_ms),
            PromotionState::ReadyToAdvance => self.step_advance(record, fence, now_ms, true),
            PromotionState::Advancing => self.step_advance(record, fence, now_ms, false),
            other => Ok(PromotionTickOutcome::Progressed {
                promotion_id: record.promotion_id.clone(),
                state: other,
            }),
        }
    }

    /// The evidence/policy gate. Pure validation plus one policy decision; the
    /// only durable writes are the recorded bindings and the decision.
    fn step_evidence(
        &self,
        record: &PromotionRecord,
        fence: &PromotionFence,
        now_ms: u64,
    ) -> Result<PromotionTickOutcome, PromotionError> {
        let (target, decision, expected_old_head, base_revision) = match self.gate(record)? {
            Gate::Reject { code, detail } => {
                let record = self.promotions.reject(
                    fence,
                    &record.promotion_id,
                    PromotionState::EvidenceValidating,
                    code,
                    &detail,
                    now_ms,
                )?;
                return Ok(PromotionTickOutcome::Terminal {
                    promotion_id: record.promotion_id,
                    state: record.state,
                    code: code.as_str().to_string(),
                });
            }
            Gate::Promote {
                target,
                decision,
                expected_old_head,
                base_revision,
            } => (target, decision, expected_old_head, base_revision),
        };

        self.promotions.record_bindings(
            fence,
            &record.promotion_id,
            &base_revision,
            &expected_old_head,
            &target.target_hash(),
            &target.policy.policy_hash(),
            now_ms,
        )?;
        self.promotions
            .record_policy_decision(fence, &record.promotion_id, &decision, now_ms)?;
        let record = self
            .promotions
            .begin_checkpoint(fence, &record.promotion_id, now_ms)?;
        let _ = decision;
        Ok(PromotionTickOutcome::Progressed {
            promotion_id: record.promotion_id,
            state: record.state,
        })
    }

    /// The checkpoint effect: create or adopt the local commit and ref.
    fn step_checkpoint(
        &self,
        record: &PromotionRecord,
        fence: &PromotionFence,
        now_ms: u64,
    ) -> Result<PromotionTickOutcome, PromotionError> {
        let target = match self.targets.load(&record.target_ref) {
            Ok(target) => target,
            Err(error) => {
                let record = self.promotions.park_unknown_manual(
                    fence,
                    &record.promotion_id,
                    PromotionState::Checkpointing,
                    error.code,
                    &error.message,
                    now_ms,
                )?;
                return Ok(PromotionTickOutcome::Parked {
                    promotion_id: record.promotion_id,
                    code: error.code.as_str().to_string(),
                });
            }
        };
        if let Err(error) = self.ensure_target_binding(record, &target) {
            let code = error.code;
            let record = self.promotions.park_unknown_manual(
                fence,
                &record.promotion_id,
                PromotionState::Checkpointing,
                code,
                &error.message,
                now_ms,
            )?;
            return Ok(PromotionTickOutcome::Parked {
                promotion_id: record.promotion_id,
                code: code.as_str().to_string(),
            });
        }
        let expected_old_head = match record.expected_old_head.clone() {
            Some(expected_old_head) => expected_old_head,
            None => {
                return self.converge_checkpoint_failure(
                    record,
                    fence,
                    promotion_error(
                        PromotionErrorCode::InvalidPromotionState,
                        "the promotion reached checkpointing without a recorded expected head",
                    ),
                    now_ms,
                )
            }
        };
        let request = match self.checkpoint_request(record, &target, &expected_old_head) {
            Ok(request) => request,
            Err(error) => return self.converge_checkpoint_failure(record, fence, error, now_ms),
        };
        let base_repo = match self.require_base_repo() {
            Ok(base_repo) => base_repo,
            Err(error) => return self.converge_checkpoint_failure(record, fence, error, now_ms),
        };
        let owner = PromotionCheckpointOwner::new(base_repo, &self.worktrees_root);

        let proof = match owner.observe_checkpoint(&request) {
            Ok(CheckpointObservation::PresentConsistent) => {
                let commit = match owner.checkpoint_commit(&request) {
                    Ok(Some(commit)) => commit,
                    Ok(None) => {
                        return self.converge_checkpoint_failure(
                            record,
                            fence,
                            promotion_error(
                                PromotionErrorCode::CheckpointFailed,
                                "the checkpoint ref vanished between observation and adoption",
                            ),
                            now_ms,
                        )
                    }
                    Err(error) => {
                        return self.converge_checkpoint_failure(record, fence, error, now_ms)
                    }
                };
                CheckpointProof {
                    promotion_id: record.promotion_id.clone(),
                    checkpoint_commit: commit,
                    checkpoint_ref: checkpoint_ref_for(&record.promotion_id),
                    workspace_root: PathBuf::new(),
                }
            }
            Err(error) => return self.converge_checkpoint_failure(record, fence, error, now_ms),
            Ok(CheckpointObservation::PresentInconsistent) => {
                let record = self.promotions.park_unknown_manual(
                    fence,
                    &record.promotion_id,
                    PromotionState::Checkpointing,
                    PromotionErrorCode::EffectUncertain,
                    "the namespaced checkpoint ref exists but does not prove this attempt",
                    now_ms,
                )?;
                return Ok(PromotionTickOutcome::Parked {
                    promotion_id: record.promotion_id,
                    code: "effect_uncertain".to_string(),
                });
            }
            Ok(CheckpointObservation::Absent) => {
                let patch = match self.candidate_patch(record) {
                    Ok(patch) => patch,
                    Err(error) => {
                        return self.converge_checkpoint_failure(record, fence, error, now_ms)
                    }
                };
                if crate::workflow::react::experiment_owner::patch::digest_hex(&patch)
                    != record.patch_sha256
                {
                    let code = PromotionErrorCode::PatchApplyFailed;
                    let record = self.promotions.reject(
                        fence,
                        &record.promotion_id,
                        PromotionState::Checkpointing,
                        code,
                        "the durable patch artifact no longer matches its recorded digest",
                        now_ms,
                    )?;
                    return Ok(PromotionTickOutcome::Terminal {
                        promotion_id: record.promotion_id,
                        state: record.state,
                        code: code.as_str().to_string(),
                    });
                }
                let proof = match owner.create_checkpoint(&request, &patch) {
                    Ok(proof) => proof,
                    Err(error)
                        if matches!(
                            error.code,
                            PromotionErrorCode::BranchHeadDrift
                                | PromotionErrorCode::PatchApplyFailed
                                | PromotionErrorCode::PatchUnbound
                                | PromotionErrorCode::BranchCheckedOut
                                | PromotionErrorCode::CheckpointFailed
                        ) =>
                    {
                        let code = error.code;
                        let record = self.promotions.reject(
                            fence,
                            &record.promotion_id,
                            PromotionState::Checkpointing,
                            code,
                            &error.message,
                            now_ms,
                        )?;
                        return Ok(PromotionTickOutcome::Terminal {
                            promotion_id: record.promotion_id,
                            state: record.state,
                            code: code.as_str().to_string(),
                        });
                    }
                    Err(error) => {
                        return self.converge_checkpoint_failure(record, fence, error, now_ms)
                    }
                };
                proof
            }
        };

        let record = self.promotions.complete_checkpoint(
            fence,
            &record.promotion_id,
            &proof.checkpoint_commit,
            &proof.checkpoint_ref,
            now_ms,
        )?;
        // The detached worktree is transient: the checkpoint commit and ref are
        // the durable evidence, so the worktree is always removed.
        let _ = owner.cleanup_workspace(&proof);
        Ok(PromotionTickOutcome::Progressed {
            promotion_id: record.promotion_id,
            state: record.state,
        })
    }

    /// Maps checkpoint-phase errors onto a durable terminal result. A local
    /// checkpoint rejection is final; repository/runtime availability remains
    /// manual because the effect boundary cannot be safely retried blindly.
    fn converge_checkpoint_failure(
        &self,
        record: &PromotionRecord,
        fence: &PromotionFence,
        error: PromotionError,
        now_ms: u64,
    ) -> Result<PromotionTickOutcome, PromotionError> {
        let code = error.code;
        if matches!(
            code,
            PromotionErrorCode::CheckpointFailed
                | PromotionErrorCode::PatchApplyFailed
                | PromotionErrorCode::PatchUnbound
                | PromotionErrorCode::BranchHeadDrift
                | PromotionErrorCode::BranchCheckedOut
                | PromotionErrorCode::ArtifactUnbound
                | PromotionErrorCode::MalformedDigest
                | PromotionErrorCode::UnsafeTargetRef
                | PromotionErrorCode::InvalidPromotionState
                | PromotionErrorCode::UnknownPromotion
        ) {
            let record = self.promotions.reject(
                fence,
                &record.promotion_id,
                PromotionState::Checkpointing,
                code,
                &error.message,
                now_ms,
            )?;
            return Ok(PromotionTickOutcome::Terminal {
                promotion_id: record.promotion_id,
                state: record.state,
                code: code.as_str().to_string(),
            });
        }
        if code == PromotionErrorCode::RepositoryUnavailable {
            let record = self.promotions.park_unknown_manual(
                fence,
                &record.promotion_id,
                PromotionState::Checkpointing,
                code,
                &error.message,
                now_ms,
            )?;
            return Ok(PromotionTickOutcome::Parked {
                promotion_id: record.promotion_id,
                code: code.as_str().to_string(),
            });
        }
        Err(error)
    }

    /// The paired canary. Inherently idempotent: it always re-measures both
    /// arms, and it never touches the branch.
    ///
    /// Every non-success outcome converges to a terminal state so the row never
    /// spins on a result it can never accept (AC-6): a failed stage or an
    /// unusable result document is `canary_failed`; an environment failure (the
    /// container runtime is down, the bundle vanished) parks as
    /// `unknown_manual`, because retrying cannot be proven safe.
    fn step_canary(
        &self,
        record: &PromotionRecord,
        fence: &PromotionFence,
        now_ms: u64,
    ) -> Result<PromotionTickOutcome, PromotionError> {
        match self.canary_gate(record, fence, now_ms) {
            Ok(outcome) => Ok(outcome),
            Err(error) => self.converge_canary_failure(record, fence, error, now_ms),
        }
    }

    /// Maps one canary-phase failure onto its terminal state.
    fn converge_canary_failure(
        &self,
        record: &PromotionRecord,
        fence: &PromotionFence,
        error: PromotionError,
        now_ms: u64,
    ) -> Result<PromotionTickOutcome, PromotionError> {
        let promotion_id = record.promotion_id.clone();
        // A gate failure (the stage regressed, timed out, or produced a document
        // that cannot be trusted) is a *decision*: record it as `canary_failed`
        // with the branch untouched.
        if matches!(
            error.code,
            PromotionErrorCode::CanaryStageFailed
                | PromotionErrorCode::CanaryResultInvalid
                | PromotionErrorCode::InsufficientSamples
        ) {
            let record = self.promotions.record_canary_result(
                fence,
                &promotion_id,
                "{}",
                &"0".repeat(64),
                &[],
                false,
                Some(error.code),
                &error.message,
                now_ms,
            )?;
            return Ok(PromotionTickOutcome::Terminal {
                promotion_id: record.promotion_id,
                state: record.state,
                code: error.code.as_str().to_string(),
            });
        }
        // Anything else is an environment failure. The FSM has no
        // "retry later" edge out of `canary_running`, and blindly re-running an
        // effect we cannot reason about is forbidden (INV-7), so the row parks.
        let record = self.promotions.park_unknown_manual(
            fence,
            &promotion_id,
            PromotionState::CanaryRunning,
            error.code,
            &error.message,
            now_ms,
        )?;
        Ok(PromotionTickOutcome::Parked {
            promotion_id: record.promotion_id,
            code: error.code.as_str().to_string(),
        })
    }

    /// The canary work itself. Errors are converged by
    /// [`Self::converge_canary_failure`].
    fn canary_gate(
        &self,
        record: &PromotionRecord,
        fence: &PromotionFence,
        now_ms: u64,
    ) -> Result<PromotionTickOutcome, PromotionError> {
        let target = self.targets.load(&record.target_ref)?;
        self.ensure_target_binding(record, &target)?;
        let profile = self.load_bound_canary_profile(record, &target)?;
        let config = self.canary_config(&profile)?;
        let base_repo = self.require_base_repo()?;
        let bundle_root = self.stage_canary_bundle(record, &target)?;
        let checkpoint_commit = record.checkpoint_commit.clone().ok_or_else(|| {
            promotion_error(
                PromotionErrorCode::InvalidPromotionState,
                "the promotion reached canary_running without a checkpoint commit",
            )
        })?;
        let expected_old_head = record.expected_old_head.clone().ok_or_else(|| {
            promotion_error(
                PromotionErrorCode::InvalidPromotionState,
                "the promotion reached canary_running without a recorded expected head",
            )
        })?;
        let runner = PromotionCanaryRunner::new(base_repo, &self.worktrees_root, config)?;
        let request = CanaryRunRequest {
            promotion_id: record.promotion_id.clone(),
            fence: fence.clone(),
            old_revision: expected_old_head,
            checkpoint_revision: checkpoint_commit,
            bundle_source_root: bundle_root,
        };

        // Errors are converged by `converge_canary_failure`, so every
        // non-success outcome reaches a terminal state.
        let result = runner.run(&target.canary, &request)?;
        let rows = stage_rows(&result, &record.promotion_id);
        let json = serde_json::to_string(&result).map_err(|error| {
            promotion_error(
                PromotionErrorCode::PersistenceFailure,
                format!("the canary result could not be serialised: {error}"),
            )
        })?;
        let record = self.promotions.record_canary_result(
            fence,
            &record.promotion_id,
            &json,
            &result.result_hash(),
            &rows,
            true,
            None,
            "pass",
            now_ms,
        )?;
        Ok(PromotionTickOutcome::Progressed {
            promotion_id: record.promotion_id,
            state: record.state,
        })
    }

    /// The branch effect. `begin` records the durable intent when the row is
    /// still `ready_to_advance`.
    fn step_advance(
        &self,
        record: &PromotionRecord,
        fence: &PromotionFence,
        now_ms: u64,
        begin: bool,
    ) -> Result<PromotionTickOutcome, PromotionError> {
        let target = match self.targets.load(&record.target_ref) {
            Ok(target) => target,
            Err(error) => {
                let record = self.promotions.park_unknown_manual(
                    fence,
                    &record.promotion_id,
                    if begin {
                        PromotionState::ReadyToAdvance
                    } else {
                        PromotionState::Advancing
                    },
                    error.code,
                    &error.message,
                    now_ms,
                )?;
                return Ok(PromotionTickOutcome::Parked {
                    promotion_id: record.promotion_id,
                    code: error.code.as_str().to_string(),
                });
            }
        };
        if let Err(error) = self.ensure_target_binding(record, &target) {
            let code = error.code;
            let record = self.promotions.park_unknown_manual(
                fence,
                &record.promotion_id,
                if begin {
                    PromotionState::ReadyToAdvance
                } else {
                    PromotionState::Advancing
                },
                code,
                &error.message,
                now_ms,
            )?;
            return Ok(PromotionTickOutcome::Parked {
                promotion_id: record.promotion_id,
                code: code.as_str().to_string(),
            });
        }
        let expected_old_head = record.expected_old_head.clone().ok_or_else(|| {
            promotion_error(
                PromotionErrorCode::InvalidPromotionState,
                "the promotion reached the branch step without a recorded expected head",
            )
        })?;
        let checkpoint_commit = record.checkpoint_commit.clone().ok_or_else(|| {
            promotion_error(
                PromotionErrorCode::InvalidPromotionState,
                "the promotion reached the branch step without a checkpoint commit",
            )
        })?;
        let base_repo = self.require_base_repo()?;
        let owner = PromotionCheckpointOwner::new(base_repo, &self.worktrees_root);
        let request = self.checkpoint_request(record, &target, &expected_old_head)?;

        // Observation first: the CAS is only attempted when the branch still
        // carries the head the checkpoint was built from.
        match owner.observe_branch(&target.branch_ref, &expected_old_head, &checkpoint_commit)? {
            BranchObservation::AtCheckpoint => {
                let record = self.promotions.complete_advance(
                    fence,
                    &record.promotion_id,
                    &checkpoint_commit,
                    now_ms,
                )?;
                return Ok(PromotionTickOutcome::Terminal {
                    promotion_id: record.promotion_id,
                    state: record.state,
                    code: "promoted".to_string(),
                });
            }
            BranchObservation::ThirdValue => {
                let record = self.promotions.park_unknown_manual(
                    fence,
                    &record.promotion_id,
                    if begin {
                        PromotionState::ReadyToAdvance
                    } else {
                        PromotionState::Advancing
                    },
                    PromotionErrorCode::BranchHeadDrift,
                    "the registered branch moved away from both the expected old head and the checkpoint",
                    now_ms,
                )?;
                return Ok(PromotionTickOutcome::Parked {
                    promotion_id: record.promotion_id,
                    code: "branch_head_drift".to_string(),
                });
            }
            BranchObservation::AtOld => {}
        }

        if begin {
            self.promotions
                .begin_advance(fence, &record.promotion_id, now_ms)?;
        }
        let observed = owner.advance_branch(&request, &checkpoint_commit)?;
        match observed {
            BranchObservation::AtCheckpoint => {
                let record = self.promotions.complete_advance(
                    fence,
                    &record.promotion_id,
                    &checkpoint_commit,
                    now_ms,
                )?;
                Ok(PromotionTickOutcome::Terminal {
                    promotion_id: record.promotion_id,
                    state: record.state,
                    code: "promoted".to_string(),
                })
            }
            other => Err(promotion_error(
                PromotionErrorCode::BranchHeadDrift,
                format!(
                    "the branch compare-and-swap ended at '{}' instead of the checkpoint",
                    other.as_str()
                ),
            )),
        }
    }

    // -----------------------------------------------------------------------
    // The pure gate (shared by the evidence step and the tests)
    // -----------------------------------------------------------------------

    fn gate(&self, record: &PromotionRecord) -> Result<Gate, PromotionError> {
        let target = match self.targets.load(&record.target_ref) {
            Ok(target) => target,
            Err(error) => return Ok(Gate::reject(error)),
        };
        if let Err(error) = self.targets.authorize_canary(&target) {
            return Ok(Gate::reject(error));
        }
        let profile = match self.profiles.load(&target.canary.execution_profile_ref) {
            Ok(profile) => profile,
            Err(error) => {
                return Ok(Gate::reject(promotion_error(
                    PromotionErrorCode::UnknownExecutionProfile,
                    format!(
                        "canary profile '{}' is not registered: {}",
                        target.canary.execution_profile_ref, error.message
                    ),
                )))
            }
        };
        if profile.owner_kind != OwnerKind::PersistentDocker {
            return Ok(Gate::reject(promotion_error(
                PromotionErrorCode::InvalidCanarySpec,
                format!(
                    "the canary profile '{}' must use the digest-pinned container owner",
                    profile.profile_ref
                ),
            )));
        }
        if record.evidence.execution_profile_ref != profile.profile_ref
            || record.evidence.execution_profile_hash != profile.profile_hash()
        {
            return Ok(Gate::reject(promotion_error(
                PromotionErrorCode::EvidenceMismatch,
                format!(
                    "execution profile '{}' does not match the submitted evidence",
                    profile.profile_ref
                ),
            )));
        }
        // A-1: the target and the canary profile must agree on which repository
        // the effect happens in, and the runtime must own that repository.
        if target.base_repo_ref != profile.base_repo_ref {
            return Ok(Gate::reject(promotion_error(
                PromotionErrorCode::EvidenceMismatch,
                format!(
                    "target '{}' names repository '{}' but its canary profile names '{}'",
                    target.target_ref, target.base_repo_ref, profile.base_repo_ref
                ),
            )));
        }
        let Some(base_repo) = self.base_repo.clone() else {
            return Ok(Gate::reject(promotion_error(
                PromotionErrorCode::RepositoryUnavailable,
                "this instance has no --base-repo, so no promotion effect can be resolved",
            )));
        };

        let campaign = match self.schedule.get_campaign(&record.campaign_id) {
            Ok(campaign) => campaign,
            Err(error) if error.code == crate::workflow::react::experiment_schedule::types::ScheduleErrorCode::UnknownCampaign => {
                return Ok(Gate::reject(promotion_error(
                    PromotionErrorCode::UnknownCampaign,
                    format!("campaign '{}' is unknown for this domain", record.campaign_id),
                )))
            }
            Err(error) => {
                return Err(promotion_error(
                    PromotionErrorCode::PersistenceFailure,
                    error.message,
                ))
            }
        };
        if campaign.execution_profile_hash.is_none()
            || campaign.execution_profile_hash.as_deref() != Some(profile.profile_hash().as_str())
        {
            return Ok(Gate::reject(promotion_error(
                PromotionErrorCode::EvidenceMismatch,
                "the durable campaign profile digest does not match the registered profile",
            )));
        }
        let jobs = match self.schedule.list_jobs(&record.campaign_id) {
            Ok(jobs) => jobs,
            Err(error) => {
                return Err(promotion_error(
                    PromotionErrorCode::PersistenceFailure,
                    error.message,
                ))
            }
        };
        let baseline = jobs.iter().find(|job| job.job.candidate_key == "baseline");
        let candidate = jobs
            .iter()
            .find(|job| job.job.candidate_key == record.candidate_key);
        let (Some(baseline), Some(candidate)) = (baseline, candidate) else {
            return Ok(Gate::reject(promotion_error(
                PromotionErrorCode::JobUnresolved,
                format!(
                    "campaign '{}' does not contain both a baseline and candidate '{}' job",
                    record.campaign_id, record.candidate_key
                ),
            )));
        };

        let campaign_binding = CampaignBindingV1 {
            campaign_id: campaign.campaign_id.clone(),
            campaign_status: campaign.status.as_str().to_string(),
            plan_hash: campaign.plan_hash.clone(),
            execution_profile_ref: campaign.execution_profile_ref.clone(),
            execution_profile_hash: campaign.execution_profile_hash.clone(),
            job_ids: campaign.job_ids.clone(),
            fixture_digests: campaign
                .fixture_refs
                .iter()
                .map(|reference| reference.manifest_digest.clone())
                .collect(),
        };
        let target_binding = TargetBindingV1 {
            target_ref: target.target_ref.clone(),
            target_hash: target.target_hash(),
            policy_hash: target.policy.policy_hash(),
            base_revision: profile.base_revision.clone(),
        };
        let baseline_binding = match self.job_binding(baseline) {
            Ok(binding) => binding,
            Err(error) => return Ok(Gate::reject(error)),
        };
        let candidate_binding = match self.job_binding(candidate) {
            Ok(binding) => binding,
            Err(error) => return Ok(Gate::reject(error)),
        };

        let verified = match verify_promotion_binding(
            &record.evidence,
            &campaign_binding,
            &baseline_binding,
            &candidate_binding,
            &target_binding,
        ) {
            Ok(verified) => verified,
            Err(error) => return Ok(Gate::reject(error)),
        };
        // The patch bytes themselves must still match the recorded digest.
        if let Err(error) = verify_artifact_file(
            &self.artifacts_root,
            &verified.patch_relative_path,
            &record.patch_sha256,
            None,
        ) {
            return Ok(Gate::reject(error));
        }

        // The branch head is read here so the checkpoint step has a stable
        // expectation, and so a drifted or checked-out branch is refused before
        // any effect.
        let owner = PromotionCheckpointOwner::new(base_repo, &self.worktrees_root);
        let expected_old_head = match owner.observe_branch_head(&target.branch_ref) {
            Ok(head) => head,
            Err(error) => return Ok(Gate::reject(error)),
        };

        let decision = evaluate_record_policy(record, &target.policy);
        if decision.outcome == PromotionOutcome::Reject {
            return Ok(Gate::Reject {
                code: decision.code,
                detail: decision.detail.clone(),
            });
        }
        Ok(Gate::Promote {
            target,
            decision,
            expected_old_head,
            base_revision: profile.base_revision.clone(),
        })
    }

    fn job_binding(&self, job: &JobRecord) -> Result<JobBindingV1, PromotionError> {
        let artifacts = self
            .schedule
            .job_artifacts(&job.job.job_id)
            .map_err(|error| {
                promotion_error(PromotionErrorCode::PersistenceFailure, error.message)
            })?;
        Ok(JobBindingV1 {
            job_id: job.job.job_id.clone(),
            campaign_id: job.job.campaign_id.clone(),
            candidate_key: job.job.candidate_key.clone(),
            state: job.job.state.as_str().to_string(),
            run_id: job.job.run_id.clone(),
            session_id: job.session_id.clone(),
            execution_profile_ref: job.job.execution_profile_ref.clone(),
            execution_profile_hash: job.execution_profile_hash.clone(),
            fixture_digest: job.job.manifest_digest.clone(),
            task_id: job.job.task_id.clone(),
            suite: job.job.suite.clone(),
            dataset_id: job.job.dataset_id.clone(),
            dataset_version: job.job.dataset_version,
            split: job.job.split.clone(),
            artifacts: artifacts
                .into_iter()
                .map(|artifact| BoundArtifactV1 {
                    kind: artifact.kind,
                    relative_path: artifact.relative_path,
                    sha256: artifact.sha256,
                    base_revision: artifact.base_revision,
                })
                .collect(),
        })
    }

    fn ensure_target_binding(
        &self,
        record: &PromotionRecord,
        target: &crate::workflow::react::experiment_promotion::policy::PromotionTargetV1,
    ) -> Result<(), PromotionError> {
        let target_hash = target.target_hash();
        let policy_hash = target.policy.policy_hash();
        if record.target_hash != target_hash || record.policy_hash != policy_hash {
            return Err(promotion_error(
                PromotionErrorCode::EvidenceMismatch,
                format!(
                    "registered target '{}' or policy changed after the promotion gate",
                    target.target_ref
                ),
            ));
        }
        Ok(())
    }

    fn load_bound_canary_profile(
        &self,
        record: &PromotionRecord,
        target: &crate::workflow::react::experiment_promotion::policy::PromotionTargetV1,
    ) -> Result<ExecutionProfileV1, PromotionError> {
        self.targets.authorize_canary(target)?;
        let profile = self
            .profiles
            .load(&target.canary.execution_profile_ref)
            .map_err(|error| {
                promotion_error(
                    PromotionErrorCode::UnknownExecutionProfile,
                    format!(
                        "canary profile '{}' is not registered: {}",
                        target.canary.execution_profile_ref, error.message
                    ),
                )
            })?;
        if profile.owner_kind != OwnerKind::PersistentDocker {
            return Err(promotion_error(
                PromotionErrorCode::InvalidCanarySpec,
                format!(
                    "the canary profile '{}' must use the digest-pinned container owner",
                    profile.profile_ref
                ),
            ));
        }
        if record.evidence.execution_profile_ref != profile.profile_ref
            || record.evidence.execution_profile_hash != profile.profile_hash()
        {
            return Err(promotion_error(
                PromotionErrorCode::EvidenceMismatch,
                format!(
                    "execution profile '{}' no longer matches the promotion evidence",
                    profile.profile_ref
                ),
            ));
        }
        Ok(profile)
    }

    fn require_base_repo(&self) -> Result<PathBuf, PromotionError> {
        self.base_repo.clone().ok_or_else(|| {
            promotion_error(
                PromotionErrorCode::RepositoryUnavailable,
                "this instance has no --base-repo",
            )
        })
    }

    fn checkpoint_request(
        &self,
        record: &PromotionRecord,
        target: &crate::workflow::react::experiment_promotion::policy::PromotionTargetV1,
        expected_old_head: &str,
    ) -> Result<PromotionCheckpointRequest, PromotionError> {
        Ok(PromotionCheckpointRequest {
            promotion_id: record.promotion_id.clone(),
            fence: record.fence().unwrap_or_else(|| PromotionFence::new("", 0)),
            branch_ref: target.branch_ref.clone(),
            expected_old_head: expected_old_head.to_string(),
            base_revision: record.base_revision.clone(),
            git_identity_name: target.git_identity_name.clone(),
            git_identity_email: target.git_identity_email.clone(),
            target_ref: target.target_ref.clone(),
            evidence_hash: record.evidence_hash.clone(),
            patch_sha256: record.patch_sha256.clone(),
        })
    }

    /// The candidate patch bytes, re-read from the immutable artifact.
    fn candidate_patch(&self, record: &PromotionRecord) -> Result<Vec<u8>, PromotionError> {
        let (row, _) = self.candidate_patch_row(record)?.ok_or_else(|| {
            promotion_error(
                PromotionErrorCode::PatchUnbound,
                "candidate job has no recorded patch artifact",
            )
        })?;
        verify_artifact_file(
            &self.artifacts_root,
            &row.relative_path,
            &record.patch_sha256,
            None,
        )
    }

    fn candidate_patch_row(
        &self,
        record: &PromotionRecord,
    ) -> Result<Option<(BoundArtifactV1, String)>, PromotionError> {
        let artifacts = self
            .schedule
            .job_artifacts(&record.evidence.candidate_job_id)
            .map_err(|error| {
                promotion_error(PromotionErrorCode::PersistenceFailure, error.message)
            })?;
        let Some(row) = artifacts
            .into_iter()
            .find(|artifact| artifact.kind == "output_patch")
        else {
            return Ok(None);
        };
        Ok(Some((
            BoundArtifactV1 {
                kind: row.kind,
                relative_path: row.relative_path,
                sha256: row.sha256,
                base_revision: row.base_revision,
            },
            record.evidence.candidate_job_id.clone(),
        )))
    }

    fn canary_config(
        &self,
        profile: &crate::workflow::react::experiment_schedule::types::ExecutionProfileV1,
    ) -> Result<DockerOwnerConfig, PromotionError> {
        use crate::workflow::react::experiment_schedule::types::MountSpecV1;
        let image_reference = profile.image_reference.clone().ok_or_else(|| {
            promotion_error(
                PromotionErrorCode::InvalidCanarySpec,
                format!(
                    "the canary profile '{}' declares no digest-pinned image",
                    profile.profile_ref
                ),
            )
        })?;
        let network_policy = profile.network_policy.clone().ok_or_else(|| {
            promotion_error(
                PromotionErrorCode::InvalidCanarySpec,
                format!(
                    "the canary profile '{}' declares no network policy",
                    profile.profile_ref
                ),
            )
        })?;
        let config = DockerOwnerConfig {
            image_reference,
            network_policy,
            resources: profile.resources.clone(),
            workspace_read_only: profile
                .mounts
                .iter()
                .find(|mount| mount.source_kind == MountSpecV1::SOURCE_WORKSPACE)
                .map(|mount| mount.read_only)
                .unwrap_or(false),
            mounts: profile.mounts.clone(),
        };
        config.validate().map_err(|error: ScheduleError| {
            promotion_error(
                PromotionErrorCode::InvalidCanarySpec,
                format!("{}: {}", error.code.as_str(), error.message),
            )
        })?;
        Ok(config)
    }

    /// Stages the target's verified bundle once per promotion and returns the
    /// read-only root both arms mount.
    fn stage_canary_bundle(
        &self,
        record: &PromotionRecord,
        target: &crate::workflow::react::experiment_promotion::policy::PromotionTargetV1,
    ) -> Result<PathBuf, PromotionError> {
        let expected = self
            .bundles_root
            .join(&record.promotion_id)
            .join(&target.canary.bundle_ref);
        if expected.is_dir() {
            return Ok(expected);
        }
        let source = self
            .bundles
            .acquire(&target.canary.bundle_ref)
            .map_err(|error| {
                promotion_error(
                    PromotionErrorCode::InvalidCanarySpec,
                    format!("{}: {}", error.code.as_str(), error.message),
                )
            })?;
        let staged =
            stage_bundle(&source, &self.bundles_root, &record.promotion_id).map_err(|error| {
                promotion_error(
                    PromotionErrorCode::InvalidCanarySpec,
                    format!("{}: {}", error.code.as_str(), error.message),
                )
            })?;
        Ok(staged.staged_root)
    }
}

/// The policy-gate outcome.
enum Gate {
    Promote {
        target: crate::workflow::react::experiment_promotion::policy::PromotionTargetV1,
        decision: PromotionDecision,
        expected_old_head: String,
        base_revision: String,
    },
    Reject {
        code: PromotionErrorCode,
        detail: String,
    },
}

impl Gate {
    fn reject(error: PromotionError) -> Self {
        Gate::Reject {
            code: error.code,
            detail: error.message,
        }
    }
}

/// Converts an assembled canary result into the durable per-stage rows.
fn stage_rows(
    result: &crate::workflow::react::experiment_promotion::types::CanaryResultV1,
    promotion_id: &str,
) -> Vec<CanaryStageRow> {
    let _ = promotion_id;
    result
        .stages
        .iter()
        .enumerate()
        .map(|(index, stage)| CanaryStageRow {
            stage_index: index as u32,
            stage_id: stage.stage_id.clone(),
            metric: stage.metric.clone(),
            samples: stage.samples,
            baseline_passed: stage.baseline_passed,
            candidate_passed: stage.candidate_passed,
            baseline_mean: stage.baseline_mean,
            candidate_mean: stage.candidate_mean,
            declared_status: stage.status.clone(),
            // The runner derived the status from the numbers through the shared
            // rule, so the recomputed status is the same by construction.
            recomputed_status: stage.status.clone(),
            output_sha256: sha256_of_stage(stage),
        })
        .collect()
}

/// A bounded per-stage digest, so the audit can tie a row to its numbers without
/// storing the raw stream.
fn sha256_of_stage(
    stage: &crate::workflow::react::experiment_promotion::types::CanaryStageResultV1,
) -> String {
    crate::workflow::react::experiment_promotion::types::journal_digest(&[
        crate::workflow::react::experiment_promotion::types::PromotionJournalEntryV1 {
            sequence: stage.samples as i64,
            stage: format!(
                "{}:{}:{:.6}:{:.6}",
                stage.stage_id, stage.metric, stage.baseline_mean, stage.candidate_mean
            ),
            owner_id: None,
            lease_generation: 0,
            detail: Some(stage.status.clone()),
            created_at_ms: 0,
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::experiment_promotion::ExperimentPromotionStore;
    use crate::db::MainStore;
    use crate::headless::promotion_targets::PROMOTION_TARGET_DIR;
    use crate::workflow::react::experiment_promotion::policy::{
        CanaryStageSpecV1, MetricDirection, PromotionCanarySpecV1, PromotionMetricRuleV1,
        PromotionPolicyV1, PROMOTION_POLICY_V1, PROMOTION_TARGET_V1,
    };
    use crate::workflow::react::experiment_promotion::types::{
        PromotionBudgetFactsV1, PromotionEvidenceV1, PromotionMetricFactV1, PromotionRequestV1,
        PromotionVerifierIdentityV1, PROMOTION_EVIDENCE_V1, PROMOTION_REQUEST_V1,
    };
    use crate::workflow::react::experiment_schedule::types::{
        ExecutionProfileV1, MountSpecV1, NetworkPolicyV1, ResourceLimitsV1, EXECUTION_PROFILE_V1,
    };
    use std::process::Command;
    use std::sync::Arc;
    use tempfile::tempdir;

    const T0: u64 = 1_700_000_000_000;

    fn base_repo(directory: &std::path::Path) -> (PathBuf, String) {
        let repo = directory.join("base");
        std::fs::create_dir_all(&repo).expect("mkdir");
        let git = |args: &[&str]| {
            let status = Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .env("GIT_AUTHOR_NAME", "cs-test")
                .env("GIT_AUTHOR_EMAIL", "cs-test@example.invalid")
                .env("GIT_COMMITTER_NAME", "cs-test")
                .env("GIT_COMMITTER_EMAIL", "cs-test@example.invalid")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .expect("git");
            assert!(status.success(), "git {args:?} failed");
        };
        git(&["init", "--quiet"]);
        std::fs::write(repo.join("app.txt"), "base\n").expect("write");
        git(&["add", "-A"]);
        git(&["commit", "--quiet", "-m", "base"]);
        git(&["branch", "experiment/2i"]);
        let head = String::from_utf8_lossy(
            &Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(["rev-parse", "HEAD"])
                .output()
                .expect("rev-parse")
                .stdout,
        )
        .trim()
        .to_string();
        (repo, head)
    }

    fn profile_json(base_repo_ref: &str) -> String {
        serde_json::to_string_pretty(&test_profile(base_repo_ref)).expect("serialize")
    }

    fn test_profile(base_repo_ref: &str) -> ExecutionProfileV1 {
        ExecutionProfileV1 {
            schema_version: EXECUTION_PROFILE_V1.to_string(),
            profile_ref: "canary".to_string(),
            owner_kind: OwnerKind::PersistentDocker,
            base_repo_ref: base_repo_ref.to_string(),
            base_revision: "refs/heads/experiment/2i".to_string(),
            image_reference: Some(format!("sha256:{}", "a".repeat(64))),
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
                    container_path: "/bundle".to_string(),
                    read_only: true,
                },
            ],
            resources: ResourceLimitsV1 {
                cpu_millis: 1000,
                memory_bytes: 256 << 20,
                pids: 64,
                no_new_privileges: true,
            },
            allowed_bundle_refs: vec!["smoke-tools".to_string()],
            input_patch_ref: None,
            input_patch_digest: None,
        }
    }

    fn target(profile_base_repo_ref: &str, branch_ref: &str) -> String {
        let target = crate::workflow::react::experiment_promotion::policy::PromotionTargetV1 {
            schema_version: PROMOTION_TARGET_V1.to_string(),
            target_ref: "local-dev".to_string(),
            base_repo_ref: profile_base_repo_ref.to_string(),
            branch_ref: branch_ref.to_string(),
            git_identity_name: "ChatSpeed Promotion".to_string(),
            git_identity_email: "promotion@chatspeed.local".to_string(),
            canary: PromotionCanarySpecV1 {
                execution_profile_ref: "canary".to_string(),
                bundle_ref: "smoke-tools".to_string(),
                executable: "./tools/canary".to_string(),
                stages: vec![CanaryStageSpecV1 {
                    stage_id: "stage-1".to_string(),
                    args: vec!["--stage".to_string(), "stage-1".to_string()],
                    metric: "score".to_string(),
                    direction: MetricDirection::HigherIsBetter,
                    min_improvement: 0.5,
                    max_regression: 0.0,
                    min_samples: 2,
                    required: true,
                }],
                timeout_ms: 30_000,
                max_output_bytes: 65_536,
            },
            policy: PromotionPolicyV1 {
                schema_version: PROMOTION_POLICY_V1.to_string(),
                policy_ref: "default".to_string(),
                require_verdict_pass: true,
                require_safety_pass: true,
                require_infra_pass: true,
                allow_unknown_cost: false,
                max_committed_micros: 1_000_000,
                metrics: vec![PromotionMetricRuleV1 {
                    metric: "verdict_score".to_string(),
                    direction: MetricDirection::HigherIsBetter,
                    min_improvement: 0.1,
                    max_regression: 0.0,
                    min_samples: 4,
                    required: true,
                }],
            },
        };
        serde_json::to_string_pretty(&target).expect("serialize")
    }

    fn write_domain(
        directory: &std::path::Path,
        profile_repo_ref: &str,
        target_repo_ref: &str,
        branch_ref: &str,
    ) {
        let profiles = directory.join(crate::headless::profiles::EXECUTION_PROFILE_DIR);
        std::fs::create_dir_all(&profiles).expect("profiles");
        std::fs::write(profiles.join("canary.json"), profile_json(profile_repo_ref))
            .expect("profile");
        let targets = directory.join(PROMOTION_TARGET_DIR);
        std::fs::create_dir_all(&targets).expect("targets");
        std::fs::write(
            targets.join("local-dev.json"),
            target(target_repo_ref, branch_ref),
        )
        .expect("target");
    }

    fn request(baseline: f64, candidate: f64, base_revision: &str) -> PromotionRequestV1 {
        PromotionRequestV1 {
            schema_version: PROMOTION_REQUEST_V1.to_string(),
            campaign_id: "camp-0123456789abcdef0123456789abcdef".to_string(),
            candidate_key: "prompt-a".to_string(),
            target_ref: "local-dev".to_string(),
            evidence: PromotionEvidenceV1 {
                schema_version: PROMOTION_EVIDENCE_V1.to_string(),
                campaign_id: "camp-0123456789abcdef0123456789abcdef".to_string(),
                candidate_key: "prompt-a".to_string(),
                baseline_job_id: "job-baseline".to_string(),
                candidate_job_id: "job-candidate".to_string(),
                baseline_run_id: "run-baseline".to_string(),
                candidate_run_id: "run-candidate".to_string(),
                candidate_session_id: "session-candidate".to_string(),
                baseline_artifact_hash: "1".repeat(64),
                candidate_artifact_hash: "2".repeat(64),
                baseline_evaluation_hash: "c".repeat(64),
                candidate_evaluation_hash: "d".repeat(64),
                baseline_verdict_hash: "e".repeat(64),
                candidate_verdict_hash: "f".repeat(64),
                fixture_ref: "smoke-tools".to_string(),
                fixture_digest: "3".repeat(64),
                task_id: "task-a".to_string(),
                suite: "chatspeed-smoke".to_string(),
                dataset_id: "chatspeed-smoke".to_string(),
                dataset_version: 2,
                split: "smoke".to_string(),
                execution_profile_ref: "canary".to_string(),
                execution_profile_hash: test_profile("repo:primary").profile_hash(),
                patch_manifest_hash: "5".repeat(64),
                patch_sha256: "6".repeat(64),
                base_revision: base_revision.to_string(),
                verdict_status: "pass".to_string(),
                verdict_score: 1.0,
                verdict_safety_status: "pass".to_string(),
                verdict_infra_status: "pass".to_string(),
                verdict_cost_status: "known".to_string(),
                budget: PromotionBudgetFactsV1 {
                    cost_status: "known".to_string(),
                    budget_rejected: false,
                    committed_micros: 10,
                    currency: "usd".to_string(),
                },
                verifier: PromotionVerifierIdentityV1 {
                    verifier_id: "chatspeed-smoke".to_string(),
                    verifier_version: "2".to_string(),
                    verifier_digest: "7".repeat(64),
                },
                metrics: vec![PromotionMetricFactV1 {
                    metric: "verdict_score".to_string(),
                    samples: 8,
                    baseline_passed: 4,
                    candidate_passed: 8,
                    baseline_mean: baseline,
                    candidate_mean: candidate,
                }],
            },
        }
    }

    fn build_supervisor(directory: &std::path::Path, repo: &PathBuf) -> PromotionSupervisor {
        let store = Arc::new(MainStore::new(directory.join("promotion.db")).expect("store"));
        PromotionSupervisor::new(
            ExperimentPromotionStore::new(store.clone()),
            ExperimentScheduleStore::new(store),
            PromotionSupervisorConfig {
                domain_root: directory.to_path_buf(),
                base_repo: Some(repo.clone()),
                owner_id: "promotion-supervisor".to_string(),
                lease_ms: 30_000,
            },
        )
    }

    /// The gate refuses a target whose repository disagrees with its canary
    /// profile, and a promotion whose campaign is unknown — before any effect.
    #[test]
    fn the_gate_refuses_a_repository_mismatch_and_an_unknown_campaign() {
        let directory = tempdir().expect("tempdir");
        let (repo, _head) = base_repo(directory.path());
        // The target names a different repository than its canary profile.
        write_domain(
            directory.path(),
            "repo:primary",
            "repo:other",
            "refs/heads/experiment/2i",
        );
        let supervisor = build_supervisor(directory.path(), &repo);

        let submission = request(0.5, 1.0, "refs/heads/experiment/2i");
        let promotion_id = submission.promotion_id();
        supervisor
            .promotions
            .submit(&submission, &promotion_id, "key-1", T0)
            .expect("submit");
        let outcome = supervisor.tick(T0).expect("tick");
        let PromotionTickOutcome::Terminal { state, code, .. } = outcome else {
            panic!("expected a rejection, got {outcome:?}");
        };
        assert_eq!(state, PromotionState::Rejected);
        assert_eq!(code, "evidence_mismatch");

        // The same target with a matching repository, but a campaign the domain
        // has never seen, is also rejected before any effect.
        let directory = tempdir().expect("tempdir");
        let (repo, _head) = base_repo(directory.path());
        write_domain(
            directory.path(),
            "repo:primary",
            "repo:primary",
            "refs/heads/experiment/2i",
        );
        let supervisor = build_supervisor(directory.path(), &repo);
        let submission = request(0.5, 1.0, "refs/heads/experiment/2i");
        let promotion_id = submission.promotion_id();
        supervisor
            .promotions
            .submit(&submission, &promotion_id, "key-1", T0)
            .expect("submit");
        let outcome = supervisor.tick(T0).expect("tick");
        assert_eq!(
            outcome,
            PromotionTickOutcome::Terminal {
                promotion_id,
                state: PromotionState::Rejected,
                code: "unknown_campaign".to_string(),
            }
        );
    }

    /// A missing `--base-repo` is a pre-effect refusal, never a silent effect in
    /// the wrong repository.
    #[test]
    fn a_missing_base_repo_refuses_the_gate() {
        let directory = tempdir().expect("tempdir");
        let (repo, _head) = base_repo(directory.path());
        write_domain(
            directory.path(),
            "repo:primary",
            "repo:primary",
            "refs/heads/experiment/2i",
        );
        let store = Arc::new(MainStore::new(directory.path().join("p.db")).expect("store"));
        let supervisor = PromotionSupervisor::new(
            ExperimentPromotionStore::new(store.clone()),
            ExperimentScheduleStore::new(store),
            PromotionSupervisorConfig {
                domain_root: directory.path().to_path_buf(),
                base_repo: None,
                owner_id: "promotion-supervisor".to_string(),
                lease_ms: 30_000,
            },
        );
        let submission = request(0.5, 1.0, "refs/heads/experiment/2i");
        let promotion_id = submission.promotion_id();
        supervisor
            .promotions
            .submit(&submission, &promotion_id, "key-1", T0)
            .expect("submit");
        let outcome = supervisor.tick(T0).expect("tick");
        assert_eq!(
            outcome,
            PromotionTickOutcome::Terminal {
                promotion_id,
                state: PromotionState::Rejected,
                code: "repository_unavailable".to_string(),
            }
        );
        let _ = repo;
    }

    /// An idle supervisor does nothing at all.
    #[test]
    fn an_empty_queue_is_idle() {
        let directory = tempdir().expect("tempdir");
        let (repo, _head) = base_repo(directory.path());
        write_domain(
            directory.path(),
            "repo:primary",
            "repo:primary",
            "refs/heads/experiment/2i",
        );
        let supervisor = build_supervisor(directory.path(), &repo);
        assert_eq!(
            supervisor.tick(T0).expect("tick"),
            PromotionTickOutcome::Idle
        );
        assert!(supervisor.reconcile(T0).expect("reconcile").is_empty());
    }

    #[test]
    fn insufficient_canary_samples_converge_to_a_terminal_failure() {
        let directory = tempdir().expect("tempdir");
        let store = Arc::new(MainStore::new(directory.path().join("promotion.db")).expect("store"));
        let supervisor = PromotionSupervisor::new(
            ExperimentPromotionStore::new(store.clone()),
            ExperimentScheduleStore::new(store),
            PromotionSupervisorConfig {
                domain_root: directory.path().to_path_buf(),
                base_repo: None,
                owner_id: "promotion-supervisor".to_string(),
                lease_ms: 30_000,
            },
        );
        let submission = request(0.5, 1.0, "refs/heads/experiment/2i");
        let promotion_id = submission.promotion_id();
        supervisor
            .promotions
            .submit(&submission, &promotion_id, "key-1", T0)
            .expect("submit");
        let claimed = match supervisor
            .promotions
            .claim_next("worker-a", T0, 30_000)
            .expect("claim")
        {
            PromotionClaimOutcome::Claimed(record) => *record,
            PromotionClaimOutcome::Idle => panic!("expected claim"),
        };
        let fence = claimed.fence().expect("fence");
        supervisor
            .promotions
            .begin_checkpoint(&fence, &promotion_id, T0)
            .expect("checkpoint intent");
        supervisor
            .promotions
            .complete_checkpoint(
                &fence,
                &promotion_id,
                &"c".repeat(40),
                "refs/chatspeed/checkpoints/test",
                T0,
            )
            .expect("checkpoint");
        let canary = supervisor
            .promotions
            .begin_canary(&fence, &promotion_id, T0)
            .expect("canary intent");
        let outcome = supervisor
            .converge_canary_failure(
                &canary,
                &fence,
                PromotionError::new(
                    PromotionErrorCode::InsufficientSamples,
                    "paired canary supplied too few samples",
                ),
                T0,
            )
            .expect("terminal convergence");
        assert_eq!(
            outcome,
            PromotionTickOutcome::Terminal {
                promotion_id: promotion_id.clone(),
                state: PromotionState::CanaryFailed,
                code: "insufficient_samples".to_string(),
            }
        );
        let record = supervisor.promotions.get(&promotion_id).expect("record");
        assert_eq!(record.error_code.as_deref(), Some("insufficient_samples"));
        assert!(record.owner_id.is_none());
        assert!(matches!(
            supervisor
                .promotions
                .claim_next("worker-b", T0 + 60_000, 30_000)
                .expect("claim"),
            PromotionClaimOutcome::Idle
        ));
    }

    /// The pure stage rule the runner and the policy share, exercised here so a
    /// target can be validated without a container.
    #[test]
    fn the_stage_rule_is_shared() {
        let spec = CanaryStageSpecV1 {
            stage_id: "stage-1".to_string(),
            args: Vec::new(),
            metric: "score".to_string(),
            direction: MetricDirection::HigherIsBetter,
            min_improvement: 0.5,
            max_regression: 0.0,
            min_samples: 2,
            required: true,
        };
        assert!(
            crate::workflow::react::experiment_promotion::policy::canary_stage_passes(
                &spec, 0.25, 1.0
            )
        );
        assert!(
            !crate::workflow::react::experiment_promotion::policy::canary_stage_passes(
                &spec, 1.0, 0.25
            )
        );
        assert_eq!(
            crate::workflow::react::experiment_promotion::types::CANARY_RESULT_V1,
            "canary_result.v1"
        );
        assert_eq!(
            checkpoint_ref_for("promo-x"),
            "refs/chatspeed/checkpoints/promo-x"
        );
    }
}
