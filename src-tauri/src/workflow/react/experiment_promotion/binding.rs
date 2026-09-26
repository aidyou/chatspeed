//! Phase 2I evidence binding: the backend's independent cross-check of a
//! caller's promotion projection against the durable campaign, the durable job
//! rows and the immutable patch manifest.
//!
//! The projection the CLI submits is **not** the only evidence (A-3/INV-2). The
//! backend re-derives every durable fact it can and refuses the promotion
//! *before* any effect when a single one disagrees:
//!
//! - the durable campaign exists, is not cancelled, and owns both jobs;
//! - both jobs reached `succeeded`, belong to the campaign, and carry the run
//!   and session identity the projection claims;
//! - the candidate key is a member of the campaign;
//! - the fixture identity (ref, digest, task, suite, dataset, split) matches the
//!   campaign's durable fixture set and the job row;
//! - the execution profile the arms ran under is the campaign's profile;
//! - the immutable patch artifacts the scheduler itself recorded exist and
//!   carry exactly the digests the projection advertises, and their
//!   `base_revision` equals the projected base revision.
//!
//! What the backend **cannot** re-derive from its own durable state is the
//! offline 2D/2E sidecar chain, which lives in the operator's local campaign
//! output. Those digests are therefore carried as *facts* of the operator
//! adapter: they are structurally validated (strict schema, digests, statuses)
//! and the durable half they hang off is fully cross-checked, but the backend
//! does not claim to re-run the verifier. This boundary is deliberate and is
//! recorded in the Phase 2I Implementation Record.
//!
//! The module is pure over its typed inputs plus one explicit filesystem check
//! ([`verify_artifact_file`]); it never opens a database, a Git repository or a
//! provider.

use crate::workflow::react::experiment_owner::patch::{
    digest_hex, is_safe_relative_path, PATCH_FILE_NAME, PATCH_MANIFEST_FILE_NAME,
};
use crate::workflow::react::experiment_promotion::types::{
    is_sha256_hex, PromotionError, PromotionErrorCode, PromotionEvidenceV1,
};
use std::path::Path;

/// The durable half of the campaign a promotion belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampaignBindingV1 {
    pub campaign_id: String,
    /// `active`, `closed` or `cancelled`.
    pub campaign_status: String,
    pub plan_hash: String,
    pub execution_profile_ref: String,
    pub execution_profile_hash: Option<String>,
    pub job_ids: Vec<String>,
    /// The `manifest_digest` of every fixture ref the campaign was scheduled
    /// with.
    pub fixture_digests: Vec<String>,
}

impl CampaignBindingV1 {
    pub fn accepts_new_work(&self) -> bool {
        self.campaign_status != "cancelled"
    }
}

/// One durable artifact row, as recorded by the 2G scheduler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundArtifactV1 {
    pub kind: String,
    pub relative_path: String,
    pub sha256: String,
    pub base_revision: Option<String>,
}

impl BoundArtifactV1 {
    fn find<'a>(artifacts: &'a [BoundArtifactV1], kind: &str) -> Option<&'a BoundArtifactV1> {
        artifacts.iter().find(|artifact| artifact.kind == kind)
    }
}

/// The durable half of one arm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobBindingV1 {
    pub job_id: String,
    pub campaign_id: String,
    pub candidate_key: String,
    /// `succeeded` and the other durable job states.
    pub state: String,
    pub run_id: Option<String>,
    pub session_id: Option<String>,
    pub execution_profile_ref: String,
    pub execution_profile_hash: Option<String>,
    pub fixture_digest: String,
    pub task_id: String,
    pub suite: String,
    pub dataset_id: String,
    pub dataset_version: u32,
    pub split: String,
    pub artifacts: Vec<BoundArtifactV1>,
}

/// The server-owned target facts the projection is bound to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetBindingV1 {
    pub target_ref: String,
    pub target_hash: String,
    pub policy_hash: String,
    /// The base revision the registered target expects the patch to sit on.
    pub base_revision: String,
}

/// The result of a successful binding check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedPromotionBinding {
    pub base_revision: String,
    pub patch_sha256: String,
    pub patch_manifest_hash: String,
    /// The relative path of the immutable patch inside the artifacts root.
    pub patch_relative_path: String,
}

fn reject(code: PromotionErrorCode, message: impl Into<String>) -> PromotionError {
    PromotionError::new(code, message)
}

/// Re-verifies one artifact file against the digest the scheduler recorded.
///
/// A missing file, an unsafe relative path or a digest drift is a
/// pre-effect rejection: a promotion is never created from an artifact that
/// cannot be re-read byte-for-byte.
pub fn verify_artifact_file(
    artifacts_root: &Path,
    relative_path: &str,
    expected_sha256: &str,
    expected_size: Option<u64>,
) -> Result<Vec<u8>, PromotionError> {
    if !is_safe_relative_path(relative_path) {
        return Err(reject(
            PromotionErrorCode::ArtifactUnbound,
            format!("artifact path '{relative_path}' is not a safe relative path"),
        ));
    }
    let path = artifacts_root.join(relative_path);
    let bytes = std::fs::read(&path).map_err(|error| {
        reject(
            PromotionErrorCode::ArtifactUnbound,
            format!("artifact '{relative_path}' cannot be read: {error}"),
        )
    })?;
    if let Some(expected) = expected_size {
        if bytes.len() as u64 != expected {
            return Err(reject(
                PromotionErrorCode::ArtifactUnbound,
                format!(
                    "artifact '{relative_path}' is {} bytes, expected {expected}",
                    bytes.len()
                ),
            ));
        }
    }
    let actual = digest_hex(&bytes);
    if actual != expected_sha256 {
        return Err(reject(
            PromotionErrorCode::ArtifactUnbound,
            format!("artifact '{relative_path}' does not match its recorded digest"),
        ));
    }
    Ok(bytes)
}

/// Cross-checks a promotion projection against the durable campaign, the two
/// durable jobs and the server-owned target.
///
/// Everything the backend can re-derive is re-derived here. The first
/// disagreement decides, so the recorded machine code is deterministic.
pub fn verify_promotion_binding(
    evidence: &PromotionEvidenceV1,
    campaign: &CampaignBindingV1,
    baseline: &JobBindingV1,
    candidate: &JobBindingV1,
    target: &TargetBindingV1,
) -> Result<VerifiedPromotionBinding, PromotionError> {
    if campaign.campaign_id != evidence.campaign_id {
        return Err(reject(
            PromotionErrorCode::EvidenceMismatch,
            format!(
                "the projection names campaign '{}' but the durable campaign is '{}'",
                evidence.campaign_id, campaign.campaign_id
            ),
        ));
    }
    if !campaign.accepts_new_work() {
        return Err(reject(
            PromotionErrorCode::CampaignNotActive,
            format!(
                "campaign '{}' is '{}'",
                campaign.campaign_id, campaign.campaign_status
            ),
        ));
    }
    if baseline.job_id != evidence.baseline_job_id || candidate.job_id != evidence.candidate_job_id
    {
        return Err(reject(
            PromotionErrorCode::JobUnresolved,
            "the projection does not name the durable baseline/candidate jobs",
        ));
    }
    for job in [baseline, candidate] {
        if job.campaign_id != campaign.campaign_id {
            return Err(reject(
                PromotionErrorCode::JobUnresolved,
                format!(
                    "job '{}' belongs to campaign '{}', not '{}'",
                    job.job_id, job.campaign_id, campaign.campaign_id
                ),
            ));
        }
        if !campaign.job_ids.iter().any(|id| id == &job.job_id) {
            return Err(reject(
                PromotionErrorCode::JobUnresolved,
                format!(
                    "job '{}' is not a member of campaign '{}'",
                    job.job_id, campaign.campaign_id
                ),
            ));
        }
        if job.state != "succeeded" {
            return Err(reject(
                PromotionErrorCode::JobNotSucceeded,
                format!("job '{}' is '{}', not 'succeeded'", job.job_id, job.state),
            ));
        }
    }
    if candidate.candidate_key != evidence.candidate_key {
        return Err(reject(
            PromotionErrorCode::CandidateNotInCampaign,
            format!(
                "the projection names candidate '{}' but the durable job carries '{}'",
                evidence.candidate_key, candidate.candidate_key
            ),
        ));
    }
    if baseline.candidate_key == candidate.candidate_key {
        return Err(reject(
            PromotionErrorCode::CandidateNotInCampaign,
            "the baseline and candidate arms resolve to the same durable candidate",
        ));
    }

    // Run/session identity.
    if baseline.run_id.as_deref() != Some(evidence.baseline_run_id.as_str())
        || candidate.run_id.as_deref() != Some(evidence.candidate_run_id.as_str())
        || candidate.session_id.as_deref() != Some(evidence.candidate_session_id.as_str())
    {
        return Err(reject(
            PromotionErrorCode::EvidenceMismatch,
            "the projected run/session identity does not match the durable job rows",
        ));
    }

    // Execution profile and fixture identity.
    let Some(campaign_profile_hash) = campaign.execution_profile_hash.as_deref() else {
        return Err(reject(
            PromotionErrorCode::EvidenceMismatch,
            "the durable campaign has no historical execution profile digest",
        ));
    };
    if !is_sha256_hex(campaign_profile_hash)
        || campaign_profile_hash != evidence.execution_profile_hash
    {
        return Err(reject(
            PromotionErrorCode::EvidenceMismatch,
            "the promotion evidence does not match the durable campaign profile digest",
        ));
    }
    for job in [baseline, candidate] {
        if job.execution_profile_ref != evidence.execution_profile_ref
            || job.execution_profile_ref != campaign.execution_profile_ref
            || job.execution_profile_hash.as_deref() != Some(campaign_profile_hash)
        {
            return Err(reject(
                PromotionErrorCode::EvidenceMismatch,
                format!(
                    "job '{}' ran under profile '{}', not '{}'",
                    job.job_id, job.execution_profile_ref, evidence.execution_profile_ref
                ),
            ));
        }
    }
    if candidate.suite != evidence.suite
        || candidate.dataset_id != evidence.dataset_id
        || candidate.dataset_version != evidence.dataset_version
        || candidate.split != evidence.split
        || candidate.task_id != evidence.task_id
        || candidate.fixture_digest != evidence.fixture_digest
    {
        return Err(reject(
            PromotionErrorCode::EvidenceMismatch,
            "the projected fixture identity does not match the durable job row",
        ));
    }
    if !campaign
        .fixture_digests
        .iter()
        .any(|digest| digest == &evidence.fixture_digest)
    {
        return Err(reject(
            PromotionErrorCode::EvidenceMismatch,
            "the projected fixture digest is not part of the durable campaign",
        ));
    }
    if baseline.fixture_digest != candidate.fixture_digest {
        return Err(reject(
            PromotionErrorCode::EvidenceMismatch,
            "the two arms did not run the same fixture",
        ));
    }

    // The 2A artifact digests the scheduler recorded for each arm.
    let baseline_summary =
        BoundArtifactV1::find(&baseline.artifacts, "job_summary").ok_or_else(|| {
            reject(
                PromotionErrorCode::ArtifactUnbound,
                format!(
                    "baseline job '{}' has no recorded job_summary artifact",
                    baseline.job_id
                ),
            )
        })?;
    let candidate_summary =
        BoundArtifactV1::find(&candidate.artifacts, "job_summary").ok_or_else(|| {
            reject(
                PromotionErrorCode::ArtifactUnbound,
                format!(
                    "candidate job '{}' has no recorded job_summary artifact",
                    candidate.job_id
                ),
            )
        })?;
    if baseline_summary.sha256 != evidence.baseline_artifact_hash {
        return Err(reject(
            PromotionErrorCode::ArtifactUnbound,
            "the projected baseline artifact digest does not match the durable artifact row",
        ));
    }
    if candidate_summary.sha256 != evidence.candidate_artifact_hash {
        return Err(reject(
            PromotionErrorCode::ArtifactUnbound,
            "the projected candidate artifact digest does not match the durable artifact row",
        ));
    }

    // The immutable patch the candidate produced.
    let manifest_row =
        BoundArtifactV1::find(&candidate.artifacts, "patch_manifest").ok_or_else(|| {
            reject(
                PromotionErrorCode::PatchUnbound,
                format!(
                    "candidate job '{}' has no recorded patch manifest",
                    candidate.job_id
                ),
            )
        })?;
    let patch_row =
        BoundArtifactV1::find(&candidate.artifacts, "output_patch").ok_or_else(|| {
            reject(
                PromotionErrorCode::PatchUnbound,
                format!(
                    "candidate job '{}' has no recorded output patch",
                    candidate.job_id
                ),
            )
        })?;
    if !is_sha256_hex(&manifest_row.sha256) || !is_sha256_hex(&patch_row.sha256) {
        return Err(reject(
            PromotionErrorCode::ArtifactUnbound,
            "a recorded patch artifact digest is malformed",
        ));
    }
    if manifest_row.sha256 != evidence.patch_manifest_hash {
        return Err(reject(
            PromotionErrorCode::PatchUnbound,
            "the projected patch manifest digest does not match the durable artifact row",
        ));
    }
    if patch_row.sha256 != evidence.patch_sha256 {
        return Err(reject(
            PromotionErrorCode::PatchUnbound,
            "the projected patch digest does not match the durable artifact row",
        ));
    }
    if !patch_row.relative_path.ends_with(PATCH_FILE_NAME)
        || !manifest_row
            .relative_path
            .ends_with(PATCH_MANIFEST_FILE_NAME)
    {
        return Err(reject(
            PromotionErrorCode::PatchUnbound,
            "the recorded patch artifacts do not use the published file names",
        ));
    }
    if manifest_row.base_revision.as_deref() != Some(evidence.base_revision.as_str()) {
        return Err(reject(
            PromotionErrorCode::EvidenceMismatch,
            format!(
                "the patch manifest was produced against '{}', the projection claims '{}'",
                manifest_row.base_revision.as_deref().unwrap_or("<absent>"),
                evidence.base_revision
            ),
        ));
    }
    if target.base_revision != evidence.base_revision {
        return Err(reject(
            PromotionErrorCode::EvidenceMismatch,
            format!(
                "the registered target expects base revision '{}', the projection claims '{}'",
                target.base_revision, evidence.base_revision
            ),
        ));
    }
    if target.target_hash.is_empty() || target.policy_hash.is_empty() {
        return Err(reject(
            PromotionErrorCode::EvidenceMismatch,
            "the registered target is missing its identity digests",
        ));
    }

    Ok(VerifiedPromotionBinding {
        base_revision: evidence.base_revision.clone(),
        patch_sha256: evidence.patch_sha256.clone(),
        patch_manifest_hash: evidence.patch_manifest_hash.clone(),
        patch_relative_path: patch_row.relative_path.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::react::experiment_promotion::types::{
        PromotionBudgetFactsV1, PromotionMetricFactV1, PromotionVerifierIdentityV1,
        PROMOTION_EVIDENCE_V1,
    };
    use tempfile::tempdir;

    const BASE: &str = "refs/heads/main";

    fn artifact(kind: &str, relative_path: &str, sha256: &str) -> BoundArtifactV1 {
        BoundArtifactV1 {
            kind: kind.to_string(),
            relative_path: relative_path.to_string(),
            sha256: sha256.to_string(),
            base_revision: Some(BASE.to_string()),
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
            execution_profile_ref: "smoke-local".to_string(),
            execution_profile_hash: "4".repeat(64),
            patch_manifest_hash: "5".repeat(64),
            patch_sha256: "6".repeat(64),
            base_revision: BASE.to_string(),
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
                baseline_mean: 0.5,
                candidate_mean: 1.0,
            }],
        }
    }

    fn campaign() -> CampaignBindingV1 {
        CampaignBindingV1 {
            campaign_id: "camp-0123456789abcdef0123456789abcdef".to_string(),
            campaign_status: "closed".to_string(),
            plan_hash: "8".repeat(64),
            execution_profile_ref: "smoke-local".to_string(),
            execution_profile_hash: Some("4".repeat(64)),
            job_ids: vec!["job-baseline".to_string(), "job-candidate".to_string()],
            fixture_digests: vec!["3".repeat(64)],
        }
    }

    fn job(
        job_id: &str,
        candidate_key: &str,
        run_id: &str,
        artifacts: Vec<BoundArtifactV1>,
    ) -> JobBindingV1 {
        JobBindingV1 {
            job_id: job_id.to_string(),
            campaign_id: "camp-0123456789abcdef0123456789abcdef".to_string(),
            candidate_key: candidate_key.to_string(),
            state: "succeeded".to_string(),
            run_id: Some(run_id.to_string()),
            session_id: Some(format!("session-{run_id}")),
            execution_profile_ref: "smoke-local".to_string(),
            execution_profile_hash: Some("4".repeat(64)),
            fixture_digest: "3".repeat(64),
            task_id: "task-a".to_string(),
            suite: "chatspeed-smoke".to_string(),
            dataset_id: "chatspeed-smoke".to_string(),
            dataset_version: 2,
            split: "smoke".to_string(),
            artifacts,
        }
    }

    fn baseline() -> JobBindingV1 {
        JobBindingV1 {
            session_id: Some("session-baseline".to_string()),
            ..job(
                "job-baseline",
                "baseline",
                "run-baseline",
                vec![artifact(
                    "job_summary",
                    "jobs/job-baseline/job-summary.json",
                    &"1".repeat(64),
                )],
            )
        }
    }

    fn candidate() -> JobBindingV1 {
        JobBindingV1 {
            session_id: Some("session-candidate".to_string()),
            ..job(
                "job-candidate",
                "prompt-a",
                "run-candidate",
                vec![
                    artifact(
                        "job_summary",
                        "jobs/job-candidate/job-summary.json",
                        &"2".repeat(64),
                    ),
                    artifact(
                        "patch_manifest",
                        "jobs/job-candidate/patch-manifest.json",
                        &"5".repeat(64),
                    ),
                    artifact(
                        "output_patch",
                        "jobs/job-candidate/patch.diff",
                        &"6".repeat(64),
                    ),
                ],
            )
        }
    }

    fn target() -> TargetBindingV1 {
        TargetBindingV1 {
            target_ref: "local-dev".to_string(),
            target_hash: "9".repeat(64),
            policy_hash: "a".repeat(64),
            base_revision: BASE.to_string(),
        }
    }

    #[test]
    fn a_consistent_projection_binds() {
        let binding = verify_promotion_binding(
            &evidence(),
            &campaign(),
            &baseline(),
            &candidate(),
            &target(),
        )
        .expect("consistent");
        assert_eq!(binding.base_revision, BASE);
        assert_eq!(binding.patch_sha256, "6".repeat(64));
        assert_eq!(binding.patch_relative_path, "jobs/job-candidate/patch.diff");
    }

    #[test]
    fn every_durable_field_is_cross_checked() {
        let base = candidate();
        let mutation = |mutate: &dyn Fn(&mut JobBindingV1)| {
            let mut job = base.clone();
            mutate(&mut job);
            job
        };

        // A job that did not succeed never promotes.
        let failed = mutation(&|job| job.state = "failed".to_string());
        assert_eq!(
            verify_promotion_binding(&evidence(), &campaign(), &baseline(), &failed, &target())
                .expect_err("state")
                .code,
            PromotionErrorCode::JobNotSucceeded
        );

        // A job from another campaign never promotes.
        let foreign =
            mutation(&|job| job.campaign_id = "camp-ffffffffffffffffffffffffffffffff".to_string());
        assert_eq!(
            verify_promotion_binding(&evidence(), &campaign(), &baseline(), &foreign, &target())
                .expect_err("campaign")
                .code,
            PromotionErrorCode::JobUnresolved
        );

        // A candidate the campaign does not contain never promotes.
        let other_candidate = mutation(&|job| job.candidate_key = "prompt-b".to_string());
        assert_eq!(
            verify_promotion_binding(
                &evidence(),
                &campaign(),
                &baseline(),
                &other_candidate,
                &target()
            )
            .expect_err("candidate")
            .code,
            PromotionErrorCode::CandidateNotInCampaign
        );

        // A different run identity never promotes.
        let drifted_run = mutation(&|job| job.run_id = Some("run-other".to_string()));
        assert_eq!(
            verify_promotion_binding(
                &evidence(),
                &campaign(),
                &baseline(),
                &drifted_run,
                &target()
            )
            .expect_err("run")
            .code,
            PromotionErrorCode::EvidenceMismatch
        );

        let mut legacy_campaign = campaign();
        legacy_campaign.execution_profile_hash = None;
        assert_eq!(
            verify_promotion_binding(
                &evidence(),
                &legacy_campaign,
                &baseline(),
                &candidate(),
                &target()
            )
            .expect_err("legacy profile binding")
            .code,
            PromotionErrorCode::EvidenceMismatch
        );

        // A durable job whose historical profile digest differs never promotes.
        let drifted_hash = mutation(&|job| job.execution_profile_hash = Some("5".repeat(64)));
        assert_eq!(
            verify_promotion_binding(
                &evidence(),
                &campaign(),
                &baseline(),
                &drifted_hash,
                &target()
            )
            .expect_err("profile digest")
            .code,
            PromotionErrorCode::EvidenceMismatch
        );

        // A different execution profile never promotes.
        let other_profile = mutation(&|job| job.execution_profile_ref = "other".to_string());
        assert_eq!(
            verify_promotion_binding(
                &evidence(),
                &campaign(),
                &baseline(),
                &other_profile,
                &target()
            )
            .expect_err("profile")
            .code,
            PromotionErrorCode::EvidenceMismatch
        );

        // A drifted fixture never promotes.
        let other_fixture = mutation(&|job| job.fixture_digest = "b".repeat(64));
        assert_eq!(
            verify_promotion_binding(
                &evidence(),
                &campaign(),
                &baseline(),
                &other_fixture,
                &target()
            )
            .expect_err("fixture")
            .code,
            PromotionErrorCode::EvidenceMismatch
        );

        // A tampered artifact digest never promotes.
        let tampered_artifact = mutation(&|job| {
            job.artifacts[0].sha256 = "b".repeat(64);
        });
        assert_eq!(
            verify_promotion_binding(
                &evidence(),
                &campaign(),
                &baseline(),
                &tampered_artifact,
                &target()
            )
            .expect_err("artifact")
            .code,
            PromotionErrorCode::ArtifactUnbound
        );

        // A tampered patch digest never promotes.
        let tampered_patch = mutation(&|job| {
            job.artifacts[2].sha256 = "b".repeat(64);
        });
        assert_eq!(
            verify_promotion_binding(
                &evidence(),
                &campaign(),
                &baseline(),
                &tampered_patch,
                &target()
            )
            .expect_err("patch")
            .code,
            PromotionErrorCode::PatchUnbound
        );

        // A missing patch artifact never promotes.
        let missing_patch = mutation(&|job| job.artifacts.truncate(2));
        assert_eq!(
            verify_promotion_binding(
                &evidence(),
                &campaign(),
                &baseline(),
                &missing_patch,
                &target()
            )
            .expect_err("missing patch")
            .code,
            PromotionErrorCode::PatchUnbound
        );

        // A patch produced against another base never promotes.
        let drifted_base = mutation(&|job| {
            job.artifacts[1].base_revision = Some("refs/heads/dev".to_string());
        });
        assert_eq!(
            verify_promotion_binding(
                &evidence(),
                &campaign(),
                &baseline(),
                &drifted_base,
                &target()
            )
            .expect_err("base")
            .code,
            PromotionErrorCode::EvidenceMismatch
        );

        // A target that expects a different base never promotes.
        let mut other_target = target();
        other_target.base_revision = "refs/heads/dev".to_string();
        assert_eq!(
            verify_promotion_binding(
                &evidence(),
                &campaign(),
                &baseline(),
                &candidate(),
                &other_target
            )
            .expect_err("target base")
            .code,
            PromotionErrorCode::EvidenceMismatch
        );

        // A cancelled campaign never promotes.
        let mut cancelled = campaign();
        cancelled.campaign_status = "cancelled".to_string();
        assert_eq!(
            verify_promotion_binding(
                &evidence(),
                &cancelled,
                &baseline(),
                &candidate(),
                &target()
            )
            .expect_err("cancelled")
            .code,
            PromotionErrorCode::CampaignNotActive
        );

        // A projection naming another campaign never promotes.
        let mut foreign_campaign = campaign();
        foreign_campaign.campaign_id = "camp-ffffffffffffffffffffffffffffffff".to_string();
        assert_eq!(
            verify_promotion_binding(
                &evidence(),
                &foreign_campaign,
                &baseline(),
                &candidate(),
                &target()
            )
            .expect_err("campaign id")
            .code,
            PromotionErrorCode::EvidenceMismatch
        );

        // The two arms must share a fixture.
        let mut different_arm_fixture = baseline();
        different_arm_fixture.fixture_digest = "c".repeat(64);
        assert_eq!(
            verify_promotion_binding(
                &evidence(),
                &campaign(),
                &different_arm_fixture,
                &candidate(),
                &target()
            )
            .expect_err("arm fixture")
            .code,
            PromotionErrorCode::EvidenceMismatch
        );
    }

    #[test]
    fn a_patch_file_is_verified_byte_for_byte() {
        let directory = tempdir().expect("tempdir");
        let root = directory.path();
        std::fs::create_dir_all(root.join("jobs/job-candidate")).expect("mkdir");
        let body = b"diff --git a/x b/x\n";
        std::fs::write(root.join("jobs/job-candidate/patch.diff"), body).expect("write");
        let digest = digest_hex(body);
        verify_artifact_file(
            root,
            "jobs/job-candidate/patch.diff",
            &digest,
            Some(body.len() as u64),
        )
        .expect("verified");

        // A digest drift, a size drift, an escaping path and a missing file are
        // all pre-effect rejections.
        assert_eq!(
            verify_artifact_file(root, "jobs/job-candidate/patch.diff", &"0".repeat(64), None)
                .expect_err("drift")
                .code,
            PromotionErrorCode::ArtifactUnbound
        );
        assert_eq!(
            verify_artifact_file(root, "jobs/job-candidate/patch.diff", &digest, Some(1))
                .expect_err("size")
                .code,
            PromotionErrorCode::ArtifactUnbound
        );
        assert_eq!(
            verify_artifact_file(root, "../escape", &digest, None)
                .expect_err("escape")
                .code,
            PromotionErrorCode::ArtifactUnbound
        );
        assert_eq!(
            verify_artifact_file(root, "jobs/missing/patch.diff", &digest, None)
                .expect_err("missing")
                .code,
            PromotionErrorCode::ArtifactUnbound
        );
    }
}
