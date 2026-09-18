//! ChatSpeed headless runtime (Phase 2H).
//!
//! This module owns everything that is specific to running the ChatSpeed
//! workflow runtime *without* a window: the experiment data-domain layout and
//! its fail-closed guard, and (from U-3) the transport-neutral bootstrap that
//! assembles the single runtime authority for a headless process.
//!
//! Contract summary (AC-1, INV-1, INV-9):
//!
//! - A headless instance runs only inside an explicit data directory. It never
//!   falls back to `:memory:` and never opens the desktop database.
//! - A database may be adopted only when it does not yet exist or already
//!   carries the `experiment.v1` marker. An existing unmarked database — most
//!   importantly a desktop database — is refused.
//! - Exactly one live headless instance may hold the domain lease; a second
//!   concurrent instance fails closed instead of racing the first.

pub mod bootstrap;
pub mod domain;
pub mod logging;
pub mod profiles;
pub mod promotion_runtime;
pub mod promotion_targets;
pub mod scheduler_runtime;

/// Re-exported for the `chatspeed-headless` binary's `--config-category` flag.
/// The runtime types themselves stay crate-internal: the binary drives the
/// instance through [`HeadlessRuntime`]'s accessors, so the private persistence
/// and workflow internals never become part of a public surface.
pub use crate::db::config_transfer::ConfigCategory;
pub use crate::db::experiment_promotion::{
    decision_metrics, evaluate_record_policy, promotion_error, CanaryStageRow,
    ExperimentPromotionStore, PromotionClaimOutcome, PromotionDecisionRecord, PromotionRecord,
    SubmitOutcome,
};
pub use crate::db::experiment_schedule::{
    classify_recovery, job_digests_are_well_formed, job_transition_allowed, owner_fence,
    persistence_error, store_error, validate_job_row, validate_job_transition, verify_job_fixture,
    CampaignRecord, CampaignStatus, ClaimOutcome, ExperimentScheduleStore, JobRecord,
    RecoveryRecord, ScheduleOutcome, TransitionRequest,
};
pub use crate::workflow::react::experiment_promotion::binding::{
    verify_artifact_file, verify_promotion_binding, BoundArtifactV1, CampaignBindingV1,
    JobBindingV1, TargetBindingV1, VerifiedPromotionBinding,
};
pub use bootstrap::{start, HeadlessError, HeadlessOptions, HeadlessRuntime};
pub use domain::{
    ExperimentDomain, ExperimentDomainLease, ExperimentDomainPaths, DOMAIN_DIRECTORIES,
};
pub use profiles::{ExecutionProfileRegistry, EXECUTION_PROFILE_DIR};
pub use promotion_runtime::{
    server_promotion_supervisor, spawn_promotion_supervisor, PromotionSupervisorHandle,
    DEFAULT_PROMOTION_POLL_MS, MIN_PROMOTION_POLL_MS, PROMOTION_LEASE_MS,
};
pub use promotion_targets::{PromotionTargetRegistry, PROMOTION_TARGET_DIR};
