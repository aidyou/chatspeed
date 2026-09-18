//! The Phase 2I paired staged canary runner.
//!
//! The canary is the last gate before a branch may move. For every stage the
//! target declares, the runner asks **both** arms the same question — the
//! expected old head and this attempt's checkpoint commit — under the same
//! digest-pinned container profile, with the same verified bundle, `--network
//! none` and the target's own wall-clock/output caps. Only then does it compare
//! the two structured measurements itself.
//!
//! Design rules enforced here (AC-4/AC-6/AC-7; INV-3/4/7/8/9):
//!
//! - The programme never declares the comparison. Each arm emits one
//!   [`CanaryArmSampleV1`] for one stage, and the *runner* assembles the paired
//!   [`CanaryStageResultV1`] and recomputes pass/fail. A programme therefore
//!   cannot assert its own success (INV-3).
//! - The image, the network policy, the resources and the mount layout all come
//!   from the server-registered execution profile. This runner refuses a profile
//!   whose network policy is not `none`: a canary that could reach the network
//!   is not the deterministic verifier this phase promises (INV-9).
//! - A stage that fails, times out, floods its output or returns a document that
//!   does not match the declared stage stops the run immediately. The branch is
//!   never touched by this module at all.
//! - Both arm environments are removed on every exit path, so a failure leaves
//!   no orphan container or worktree (INV-8). The checkpoint ref itself is not
//!   managed here and is therefore never at risk.

use crate::workflow::react::experiment_owner::docker::{
    BoundedExec, DockerOwnerConfig, PersistentDockerOwner, CONTAINER_NAME_PREFIX,
    WORKSPACE_MOUNT_PATH,
};
use crate::workflow::react::experiment_owner::worktree::HostWorktreeOwner;
use crate::workflow::react::experiment_owner::{
    ExecutionOwner, OwnerAcquireRequest, PreparedWorkspace,
};
use crate::workflow::react::experiment_promotion::policy::{
    canary_stage_passes, evaluate_canary_stage, verify_canary_result, CanaryStageSpecV1,
    PromotionCanarySpecV1,
};
use crate::workflow::react::experiment_promotion::types::{
    is_valid_key, CanaryArmSampleV1, CanaryResultV1, CanaryStageResultV1, PromotionError,
    PromotionErrorCode, PromotionFence, CANARY_RESULT_V1,
};
use crate::workflow::react::experiment_schedule::types::{
    MountSpecV1, NetworkPolicyV1, OwnerFence, ScheduleError,
};
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn promotion_error(code: PromotionErrorCode, message: impl Into<String>) -> PromotionError {
    PromotionError::new(code, message)
}

/// Maps an owner/execution failure onto the promotion error surface without
/// losing the owner's machine code.
fn map_owner_error(code: PromotionErrorCode, error: ScheduleError) -> PromotionError {
    promotion_error(code, format!("{}: {}", error.code.as_str(), error.message))
}

/// The scope of one paired canary run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanaryRunRequest {
    pub promotion_id: String,
    pub fence: PromotionFence,
    /// The expected old head: the baseline arm's revision.
    pub old_revision: String,
    /// This attempt's checkpoint commit: the candidate arm's revision.
    pub checkpoint_revision: String,
    /// The staged, verified bundle root on the host. It is mounted read-only.
    pub bundle_source_root: PathBuf,
}

impl CanaryRunRequest {
    /// The deterministic owner job id of one arm. Derived from the promotion id
    /// (never caller supplied), so the worktree and container names of both arms
    /// are reproducible across restarts.
    pub fn arm_job_id(&self, arm: &str) -> String {
        format!("{}-{arm}", self.promotion_id)
    }

    /// The owner fence of one arm. The promotion's own generation is reused, so
    /// a superseded worker's arm resources are never adopted by its successor.
    pub fn arm_fence(&self) -> OwnerFence {
        OwnerFence::new(self.fence.owner_id.clone(), self.fence.lease_generation)
    }
}

/// The paired staged canary runner.
#[derive(Debug, Clone)]
pub struct PromotionCanaryRunner {
    base_repo: PathBuf,
    worktrees_root: PathBuf,
    config: DockerOwnerConfig,
}

impl PromotionCanaryRunner {
    /// Binds the runner to the server-side base repository, the experiment
    /// domain's worktrees root and the canary execution profile's config.
    pub fn new(
        base_repo: impl Into<PathBuf>,
        worktrees_root: impl Into<PathBuf>,
        config: DockerOwnerConfig,
    ) -> Result<Self, PromotionError> {
        // The no-network requirement is checked first so the diagnosis is the
        // semantically right one: a canary that could reach the network is not a
        // deterministic verifier at all (INV-9).
        if config.network_policy.mode != NetworkPolicyV1::MODE_NONE {
            return Err(promotion_error(
                PromotionErrorCode::CanaryEffectForbidden,
                format!(
                    "a promotion canary must run with network policy 'none', not '{}'",
                    config.network_policy.mode
                ),
            ));
        }
        config
            .validate()
            .map_err(|error| map_owner_error(PromotionErrorCode::InvalidCanarySpec, error))?;
        Ok(Self {
            base_repo: base_repo.into(),
            worktrees_root: worktrees_root.into(),
            config,
        })
    }

    /// The container path of the canary programme inside the verified bundle.
    ///
    /// The bundle mount is required: a canary programme may only ever come from
    /// the verified, allowlisted bundle the target names, never from the
    /// candidate's own workspace.
    fn program_path(&self, spec: &PromotionCanarySpecV1) -> Result<String, PromotionError> {
        let mount = self
            .config
            .mounts
            .iter()
            .find(|mount| mount.source_kind == MountSpecV1::SOURCE_BUNDLE && mount.read_only)
            .ok_or_else(|| {
                promotion_error(
                    PromotionErrorCode::InvalidCanarySpec,
                    "the canary profile must declare exactly one read-only bundle mount",
                )
            })?;
        let relative = canary_relative_program(&spec.executable).ok_or_else(|| {
            promotion_error(
                PromotionErrorCode::InvalidCanarySpec,
                format!(
                    "canary executable '{}' is not a safe bundle-relative program",
                    spec.executable
                ),
            )
        })?;
        Ok(format!(
            "{}/{}",
            mount.container_path.trim_end_matches('/'),
            relative
        ))
    }

    /// Runs every declared stage against both arms, in the declared order.
    ///
    /// The first stage that fails stops the run: the returned error carries
    /// `canary_stage_failed` (or the malformed-document/oversize/timeout code)
    /// and the caller must not advance the branch.
    pub fn run(
        &self,
        spec: &PromotionCanarySpecV1,
        request: &CanaryRunRequest,
    ) -> Result<CanaryResultV1, PromotionError> {
        if !is_valid_key(&request.promotion_id) || !request.promotion_id.starts_with("promo-") {
            return Err(promotion_error(
                PromotionErrorCode::InvalidPromotionState,
                "'{request.promotion_id}' is not a promotion id",
            ));
        }
        // Re-validate at run time, not only at load time (TOCTOU defence).
        self.config
            .validate()
            .map_err(|error| map_owner_error(PromotionErrorCode::InvalidCanarySpec, error))?;
        spec.validate()?;
        if !request.bundle_source_root.is_dir() {
            return Err(promotion_error(
                PromotionErrorCode::InvalidCanarySpec,
                format!(
                    "the verified bundle root '{}' does not exist",
                    request.bundle_source_root.display()
                ),
            ));
        }
        let program = self.program_path(spec)?;
        // A previous attempt that died mid-canary leaves its arms behind: the
        // container/worktree names embed the promotion id (never a caller
        // string), so they are provably ours and a restart must reclaim them
        // before acquiring fresh ones, or every crash would leak resources.
        self.cleanup_stale_arms(request);

        let baseline = self.acquire_arm(request, "old", &request.old_revision)?;
        let candidate = match self.acquire_arm(request, "cp", &request.checkpoint_revision) {
            Ok(arm) => arm,
            Err(error) => {
                self.cleanup_arm(&baseline);
                return Err(error);
            }
        };

        let outcome = self.run_stages(spec, request, &program, &baseline, &candidate);
        self.cleanup_arm(&baseline);
        self.cleanup_arm(&candidate);
        outcome
    }

    /// Removes every arm resource a previous generation of this promotion left
    /// behind: its containers (`cs-run-<promotion>-<arm>-g*`), its registered
    /// worktrees and any worktree directory a hard kill left unregistered.
    ///
    /// The namespace is provably ours — the arm job ids are derived from the
    /// backend-minted promotion id and live inside the domain's own worktrees
    /// root — so this reclamation is ownership-proven cleanup, not a destructive
    /// guess (INV-7/INV-8).
    fn cleanup_stale_arms(&self, request: &CanaryRunRequest) {
        let reclaim =
            crate::workflow::react::experiment_owner::promotion::PromotionCheckpointOwner::new(
                self.base_repo.clone(),
                self.worktrees_root.clone(),
            );
        for arm in ["old", "cp"] {
            let job_id = request.arm_job_id(arm);
            // Containers from any generation of this arm. The name filter is a
            // prefix of the backend-minted arm id, and the names themselves come
            // from docker's own listing — never from caller input.
            let prefix = format!("{CONTAINER_NAME_PREFIX}-{job_id}-g");
            if let Ok(listing) = Command::new("docker")
                .args([
                    "ps",
                    "-a",
                    "--filter",
                    &format!("name={prefix}"),
                    "--format",
                    "{{.Names}}",
                ])
                .stdin(Stdio::null())
                .output()
            {
                if listing.status.success() {
                    for name in String::from_utf8_lossy(&listing.stdout).lines() {
                        let name = name.trim();
                        if name.starts_with(&prefix) {
                            let _ = Command::new("docker")
                                .args(["rm", "-f", name])
                                .stdin(Stdio::null())
                                .output();
                        }
                    }
                }
            }
            // Registered worktrees of this arm, across generations, through the
            // checkpoint owner's own allowlisted git path.
            let _ = reclaim.remove_stale_worktrees(&format!("{job_id}-g"));
            // Directories a hard kill left behind that git no longer tracks.
            if self.worktrees_root.is_dir() {
                let entries = match std::fs::read_dir(&self.worktrees_root) {
                    Ok(entries) => entries,
                    Err(_) => continue,
                };
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.starts_with(&format!("{job_id}-g")) {
                        let _ = std::fs::remove_dir_all(entry.path());
                    }
                }
            }
        }
    }

    fn acquire_arm(
        &self,
        request: &CanaryRunRequest,
        arm: &str,
        revision: &str,
    ) -> Result<CanaryArm, PromotionError> {
        let job_id = request.arm_job_id(arm);
        let worktree = HostWorktreeOwner::new(self.base_repo.clone(), self.worktrees_root.clone());
        let owner = PersistentDockerOwner::new(worktree, self.config.clone());
        // The bundle must be permitted by the profile and the mount must exist;
        // both were validated above, so a missing root here is a real failure.
        owner
            .preflight()
            .map_err(|error| map_owner_error(PromotionErrorCode::ExecutorUnavailable, error))?;
        let acquire_request = OwnerAcquireRequest {
            job_id: job_id.clone(),
            fence: request.arm_fence(),
            base_revision: revision.to_string(),
            input_patch: None,
            bundle_source_root: Some(request.bundle_source_root.clone()),
        };
        let workspace = owner
            .acquire(&acquire_request)
            .map_err(|error| map_owner_error(PromotionErrorCode::ExecutorUnavailable, error))?;
        let container = workspace
            .container
            .as_ref()
            .map(|handle| handle.name.clone())
            .ok_or_else(|| {
                promotion_error(
                    PromotionErrorCode::ExecutorUnavailable,
                    "the canary owner produced no container",
                )
            })?;
        if workspace
            .container
            .as_ref()
            .and_then(|h| h.bundle_mount.as_ref())
            .is_none()
        {
            let _ = owner.cleanup(&workspace);
            return Err(promotion_error(
                PromotionErrorCode::InvalidCanarySpec,
                "the canary profile's bundle mount was not applied to the arm container",
            ));
        }
        let _ = WORKSPACE_MOUNT_PATH;
        Ok(CanaryArm {
            owner,
            workspace,
            container,
        })
    }

    fn run_stages(
        &self,
        spec: &PromotionCanarySpecV1,
        request: &CanaryRunRequest,
        program: &str,
        baseline: &CanaryArm,
        candidate: &CanaryArm,
    ) -> Result<CanaryResultV1, PromotionError> {
        let mut stage_results = Vec::with_capacity(spec.stages.len());
        for stage in &spec.stages {
            // Baseline first, then the checkpoint, so the ordering of the two
            // measurements is deterministic and reproducible in the audit.
            let baseline_sample = exec_arm_sample(
                &baseline.owner,
                &baseline.container,
                program,
                &stage.args,
                spec.timeout_ms,
                spec.max_output_bytes,
                stage,
            )?;
            let candidate_sample = exec_arm_sample(
                &candidate.owner,
                &candidate.container,
                program,
                &stage.args,
                spec.timeout_ms,
                spec.max_output_bytes,
                stage,
            )?;
            let (result, passed) =
                assemble_stage_result(stage, &baseline_sample, &candidate_sample)?;
            if !passed {
                return Err(promotion_error(
                    PromotionErrorCode::CanaryStageFailed,
                    format!(
                        "canary stage '{}' did not reach its required improvement: baseline mean {:.6}, checkpoint mean {:.6}",
                        stage.stage_id, result.baseline_mean, result.candidate_mean
                    ),
                ));
            }
            stage_results.push(result);
        }

        let result = CanaryResultV1 {
            schema_version: CANARY_RESULT_V1.to_string(),
            stages: stage_results,
            status: "pass".to_string(),
        };
        // Final self-check against the target's own declaration: the assembled
        // document must be exactly what the target demanded.
        verify_canary_result(spec, &result)?;
        let _ = request;
        Ok(result)
    }

    fn cleanup_arm(&self, arm: &CanaryArm) {
        // Best effort: a failure to remove must never turn a completed gate into
        // an error, and the checkpoint ref is not managed here.
        let _ = arm.owner.cleanup(&arm.workspace);
    }
}

/// One acquired arm: its owner, its workspace proof and its container handle.
struct CanaryArm {
    owner: PersistentDockerOwner,
    workspace: PreparedWorkspace,
    container: String,
}

/// The bundle-relative part of a canary programme path (`./tools/canary` →
/// `tools/canary`).
pub fn canary_relative_program(executable: &str) -> Option<&str> {
    let rest = executable.strip_prefix("./")?;
    if rest.is_empty() || rest.ends_with('/') {
        return None;
    }
    if rest
        .split('/')
        .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return None;
    }
    Some(rest)
}

/// Assembles the paired stage result from the two arm samples and recomputes
/// whether the stage passed.
///
/// This is where INV-3 is enforced mechanically: the samples carry no status at
/// all, and the `status` written into the result is derived from the numbers by
/// [`evaluate_canary_stage`].
pub fn assemble_stage_result(
    stage: &CanaryStageSpecV1,
    baseline: &CanaryArmSampleV1,
    candidate: &CanaryArmSampleV1,
) -> Result<(CanaryStageResultV1, bool), PromotionError> {
    for sample in [baseline, candidate] {
        if sample.stage_id != stage.stage_id {
            return Err(promotion_error(
                PromotionErrorCode::CanaryResultInvalid,
                format!(
                    "an arm reported stage '{}' where '{}' was expected",
                    sample.stage_id, stage.stage_id
                ),
            ));
        }
        if sample.metric != stage.metric {
            return Err(promotion_error(
                PromotionErrorCode::CanaryResultInvalid,
                format!(
                    "an arm reported metric '{}' for stage '{}', expected '{}'",
                    sample.metric, stage.stage_id, stage.metric
                ),
            ));
        }
    }
    let samples = baseline.samples.min(candidate.samples);
    let passed = canary_stage_passes(stage, baseline.mean, candidate.mean);
    let result = CanaryStageResultV1 {
        stage_id: stage.stage_id.clone(),
        metric: stage.metric.clone(),
        samples,
        baseline_passed: baseline.passed,
        candidate_passed: candidate.passed,
        baseline_mean: baseline.mean,
        candidate_mean: candidate.mean,
        // Derived from the numbers by the one shared rule; never taken from a
        // programme (the arm sample has no status field at all).
        status: if passed { "pass" } else { "fail" }.to_string(),
    };
    // Cross-check through the shared evaluator, which also enforces the sample
    // floor, so the runner and the policy can never disagree.
    let passed = evaluate_canary_stage(stage, &result)?;
    Ok((result, passed))
}

/// Runs the bundle programme for one arm and one stage, then parses its single
/// strict measurement document.
#[allow(clippy::too_many_arguments)]
fn exec_arm_sample(
    owner: &PersistentDockerOwner,
    container: &str,
    program: &str,
    stage_args: &[String],
    timeout_ms: u64,
    max_output_bytes: u64,
    stage: &CanaryStageSpecV1,
) -> Result<CanaryArmSampleV1, PromotionError> {
    let mut argv: Vec<&str> = Vec::with_capacity(stage_args.len() + 1);
    argv.push(program);
    argv.extend(stage_args.iter().map(String::as_str));
    let exec: BoundedExec = owner
        .exec_capture_bounded(container, &argv, timeout_ms, max_output_bytes)
        .map_err(|error| map_owner_error(PromotionErrorCode::ExecutorUnavailable, error))?;
    if exec.timed_out {
        return Err(promotion_error(
            PromotionErrorCode::CanaryStageFailed,
            format!(
                "canary stage '{}' exceeded its {} ms limit",
                stage.stage_id, timeout_ms
            ),
        ));
    }
    if exec.truncated {
        return Err(promotion_error(
            PromotionErrorCode::CanaryResultInvalid,
            format!(
                "canary stage '{}' produced more than {max_output_bytes} bytes of output",
                stage.stage_id
            ),
        ));
    }
    if exec.exit_code != Some(0) {
        return Err(promotion_error(
            PromotionErrorCode::CanaryStageFailed,
            format!(
                "canary stage '{}' failed to run: {}",
                stage.stage_id,
                exec.diagnostic()
            ),
        ));
    }
    let sample = CanaryArmSampleV1::parse(&exec.stdout)?;
    if sample.stage_id != stage.stage_id || sample.metric != stage.metric {
        return Err(promotion_error(
            PromotionErrorCode::CanaryResultInvalid,
            format!(
                "canary stage '{}' returned a document for stage '{}'/metric '{}'",
                stage.stage_id, sample.stage_id, sample.metric
            ),
        ));
    }
    Ok(sample)
}

/// A helper that builds the schema-correct arm sample document. Test-only: a
/// real programme writes the document itself.
#[cfg(test)]
pub fn arm_sample_document(
    stage_id: &str,
    metric: &str,
    samples: u32,
    passed: u32,
    mean: f64,
) -> CanaryArmSampleV1 {
    CanaryArmSampleV1 {
        schema_version: crate::workflow::react::experiment_promotion::types::CANARY_ARM_SAMPLE_V1
            .to_string(),
        stage_id: stage_id.to_string(),
        metric: metric.to_string(),
        samples,
        passed,
        mean,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::react::experiment_owner::docker::local_image_pin;
    use crate::workflow::react::experiment_schedule::types::ResourceLimitsV1;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use tempfile::tempdir;

    /// Each container test uses its own promotion id, so the two never contend
    /// for the same arm container names: the arm identity is derived from the
    /// promotion id and the lease generation.
    const PROMOTION_A: &str = "promo-0123456789abcdef0123456789abcdef";
    const PROMOTION_B: &str = "promo-fedcba9876543210fedcba9876543210";

    fn stage_spec(stage_id: &str, min_improvement: f64) -> CanaryStageSpecV1 {
        CanaryStageSpecV1 {
            stage_id: stage_id.to_string(),
            args: vec!["--stage".to_string(), stage_id.to_string()],
            metric: "score".to_string(),
            direction: crate::workflow::react::experiment_promotion::policy::MetricDirection::HigherIsBetter,
            min_improvement,
            max_regression: 0.0,
            min_samples: 2,
            required: true,
        }
    }

    fn spec(stages: Vec<CanaryStageSpecV1>) -> PromotionCanarySpecV1 {
        PromotionCanarySpecV1 {
            execution_profile_ref: "canary".to_string(),
            bundle_ref: "smoke-tools".to_string(),
            executable: "./tools/canary".to_string(),
            stages,
            timeout_ms: 30_000,
            max_output_bytes: 65_536,
        }
    }

    fn config(image: &str) -> DockerOwnerConfig {
        DockerOwnerConfig {
            image_reference: image.to_string(),
            network_policy: NetworkPolicyV1 {
                mode: NetworkPolicyV1::MODE_NONE.to_string(),
                allow_hosts: Vec::new(),
            },
            resources: ResourceLimitsV1 {
                cpu_millis: 1000,
                memory_bytes: 256 << 20,
                pids: 64,
                no_new_privileges: true,
            },
            workspace_read_only: false,
            mounts: vec![
                MountSpecV1 {
                    source_kind: MountSpecV1::SOURCE_WORKSPACE.to_string(),
                    container_path: WORKSPACE_MOUNT_PATH.to_string(),
                    read_only: false,
                },
                MountSpecV1 {
                    source_kind: MountSpecV1::SOURCE_BUNDLE.to_string(),
                    container_path: "/bundle".to_string(),
                    read_only: true,
                },
            ],
        }
    }

    #[test]
    fn a_programme_can_never_declare_its_own_pass() {
        let stage = stage_spec("stage-1", 0.5);
        let baseline = arm_sample_document("stage-1", "score", 4, 1, 0.25);
        let candidate = arm_sample_document("stage-1", "score", 4, 4, 1.0);
        let (result, passed) = assemble_stage_result(&stage, &baseline, &candidate).expect("pair");
        assert!(passed);
        assert_eq!(result.status, "pass");
        assert_eq!(result.baseline_mean, 0.25);
        assert_eq!(result.candidate_mean, 1.0);
        assert_eq!(result.samples, 4);

        // The arm document has no status field at all: a programme that tries to
        // declare one produces a document the runner rejects outright.
        let forged = br#"{"schema_version":"canary_arm_sample.v1","stage_id":"stage-1","metric":"score","samples":4,"passed":4,"mean":1.0,"status":"pass"}"#;
        assert_eq!(
            CanaryArmSampleV1::parse(forged).expect_err("forged").code,
            PromotionErrorCode::CanaryResultInvalid
        );

        // A regression is a structured `false`, never an error.
        let worse = arm_sample_document("stage-1", "score", 4, 0, 0.0);
        let (result, passed) = assemble_stage_result(&stage, &baseline, &worse).expect("pair");
        assert!(!passed);
        assert_eq!(result.status, "fail");

        // A stage that regresses *harder* than the allowance is still reported as
        // a failed stage, not a malformed document.
        let mut tolerant = stage_spec("stage-1", 0.1);
        tolerant.max_regression = 0.5;
        let slight = arm_sample_document("stage-1", "score", 4, 1, 0.2);
        let (_result, passed) = assemble_stage_result(&tolerant, &baseline, &slight).expect("pair");
        assert!(!passed, "no improvement means the stage does not pass");
    }

    #[test]
    fn mismatched_or_thin_arm_documents_are_rejected() {
        let stage = stage_spec("stage-1", 0.5);
        let baseline = arm_sample_document("stage-1", "score", 4, 1, 0.25);

        let wrong_stage = arm_sample_document("stage-2", "score", 4, 4, 1.0);
        assert_eq!(
            assemble_stage_result(&stage, &baseline, &wrong_stage)
                .expect_err("stage id")
                .code,
            PromotionErrorCode::CanaryResultInvalid
        );

        let wrong_metric = arm_sample_document("stage-1", "latency", 4, 4, 1.0);
        assert_eq!(
            assemble_stage_result(&stage, &baseline, &wrong_metric)
                .expect_err("metric")
                .code,
            PromotionErrorCode::CanaryResultInvalid
        );

        // Too few samples is a structured rejection, not a promotion.
        let thin = arm_sample_document("stage-1", "score", 1, 1, 1.0);
        assert_eq!(
            assemble_stage_result(&stage, &baseline, &thin)
                .expect_err("samples")
                .code,
            PromotionErrorCode::InsufficientSamples
        );

        // A document that names an unsupported schema or an impossible sample
        // count never parses.
        let bad_version = br#"{"schema_version":"canary_arm_sample.v2","stage_id":"stage-1","metric":"score","samples":4,"passed":1,"mean":0.0}"#;
        assert_eq!(
            CanaryArmSampleV1::parse(bad_version)
                .expect_err("version")
                .code,
            PromotionErrorCode::UnsupportedVersion
        );
        let impossible = br#"{"schema_version":"canary_arm_sample.v1","stage_id":"stage-1","metric":"score","samples":1,"passed":4,"mean":0.0}"#;
        assert_eq!(
            CanaryArmSampleV1::parse(impossible)
                .expect_err("passed")
                .code,
            PromotionErrorCode::CanaryResultInvalid
        );
        let oversized = vec![
            b' ';
            (crate::workflow::react::experiment_promotion::types::MAX_CANARY_OUTPUT_BYTES + 1)
                as usize
        ];
        assert_eq!(
            CanaryArmSampleV1::parse(&oversized)
                .expect_err("oversize")
                .code,
            PromotionErrorCode::CanaryResultInvalid
        );
    }

    #[test]
    fn the_runner_refuses_a_networked_or_unpinned_profile() {
        let directory = tempdir().expect("tempdir");
        let pinned = format!("sha256:{}", "a".repeat(64));
        PromotionCanaryRunner::new(directory.path(), directory.path(), config(&pinned))
            .expect("a pinned, network-less profile is accepted");

        // A profile that could reach the network is never a canary.
        let mut networked = config(&pinned);
        networked.network_policy = NetworkPolicyV1 {
            mode: NetworkPolicyV1::MODE_EGRESS_ALLOWLIST.to_string(),
            allow_hosts: Vec::new(),
        };
        assert_eq!(
            PromotionCanaryRunner::new(directory.path(), directory.path(), networked)
                .expect_err("network")
                .code,
            PromotionErrorCode::CanaryEffectForbidden
        );

        // An unpinned image is refused by the shared profile validation.
        assert_eq!(
            PromotionCanaryRunner::new(
                directory.path(),
                directory.path(),
                config("chatspeed/runner:latest")
            )
            .expect_err("unpinned")
            .code,
            PromotionErrorCode::InvalidCanarySpec
        );

        // A profile without the read-only bundle mount cannot supply a programme.
        let mut no_bundle = config(&pinned);
        no_bundle
            .mounts
            .retain(|mount| mount.source_kind != MountSpecV1::SOURCE_BUNDLE);
        no_bundle.workspace_read_only = false;
        let runner = PromotionCanaryRunner::new(directory.path(), directory.path(), no_bundle)
            .expect("valid profile");
        assert_eq!(
            runner
                .program_path(&spec(vec![stage_spec("stage-1", 0.5)]))
                .expect_err("no bundle mount")
                .code,
            PromotionErrorCode::InvalidCanarySpec
        );

        // A relative-programme helper that escapes or is absolute is refused.
        assert!(canary_relative_program("./tools/canary").is_some());
        assert!(canary_relative_program("/usr/bin/canary").is_none());
        assert!(canary_relative_program("./../canary").is_none());
        assert!(canary_relative_program("tools/canary").is_none());

        // A missing verified bundle root is a pre-effect rejection.
        let runner =
            PromotionCanaryRunner::new(directory.path(), directory.path(), config(&pinned))
                .expect("runner");
        let request = CanaryRunRequest {
            promotion_id: "promo-0123456789abcdef0123456789abcdef".to_string(),
            fence: PromotionFence::new("promotion-owner", 1),
            old_revision: "HEAD".to_string(),
            checkpoint_revision: "HEAD".to_string(),
            bundle_source_root: directory.path().join("absent-bundle"),
        };
        assert_eq!(
            runner
                .run(&spec(vec![stage_spec("stage-1", 0.5)]), &request)
                .expect_err("missing bundle")
                .code,
            PromotionErrorCode::InvalidCanarySpec
        );
    }

    #[test]
    fn arm_identities_are_deterministic() {
        let request = CanaryRunRequest {
            promotion_id: "promo-0123456789abcdef0123456789abcdef".to_string(),
            fence: PromotionFence::new("promotion-owner", 7),
            old_revision: "a".repeat(40),
            checkpoint_revision: "b".repeat(40),
            bundle_source_root: PathBuf::from("/bundle"),
        };
        assert_eq!(
            request.arm_job_id("old"),
            "promo-0123456789abcdef0123456789abcdef-old"
        );
        assert_eq!(
            request.arm_job_id("cp"),
            "promo-0123456789abcdef0123456789abcdef-cp"
        );
        assert_eq!(request.arm_fence().lease_generation, 7);
    }

    // -----------------------------------------------------------------------
    // Container gates. They run only where a digest-pinned local image and a
    // Docker daemon exist; otherwise they skip loudly instead of pretending.
    // -----------------------------------------------------------------------

    fn docker_available() -> bool {
        Command::new("docker")
            .args(["version", "--format", "{{.Server.Version}}"])
            .stdin(Stdio::null())
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    fn available_image() -> Option<String> {
        for reference in [
            "busybox:latest",
            "alpine:latest",
            "bash:latest",
            "ubuntu:26.04",
            "ubuntu:latest",
            "debian:stable-slim",
            "git:latest",
        ] {
            if let Ok(Some(pin)) = local_image_pin("docker", reference) {
                return Some(pin);
            }
        }
        None
    }

    /// A real base repository with two commits: `old` and `checkpoint`.
    fn repository(directory: &Path) -> (PathBuf, String, String) {
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
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .expect("git");
            assert!(status.success(), "git {args:?} failed");
        };
        git(&["init", "--quiet"]);
        std::fs::write(repo.join("app.txt"), "base\n").expect("write");
        git(&["add", "-A"]);
        git(&["commit", "--quiet", "-m", "old"]);
        let old = String::from_utf8_lossy(
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
        std::fs::write(repo.join("improvement.txt"), "better\n").expect("write improvement");
        git(&["add", "-A"]);
        git(&["commit", "--quiet", "-m", "checkpoint"]);
        let checkpoint = String::from_utf8_lossy(
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
        (repo, old, checkpoint)
    }

    /// A verified-looking bundle on disk whose programme measures the arm's
    /// workspace: the checkpoint arm scores 1.0, the old arm 0.25.
    fn bundle(directory: &Path, body: &str) -> PathBuf {
        let root = directory.join("bundle");
        std::fs::create_dir_all(root.join("tools")).expect("mkdir");
        let program = root.join("tools/canary");
        std::fs::write(&program, body).expect("write program");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755))
                .expect("chmod");
        }
        root
    }

    const MEASURING_PROGRAM: &str = r#"#!/bin/sh
stage=""
while [ $# -gt 0 ]; do
  case "$1" in
    --stage) stage="$2"; shift 2 ;;
    *) shift ;;
  esac
done
if [ -f /workspace/improvement.txt ]; then
  mean=1.0
  passed=2
else
  mean=0.25
  passed=0
fi
printf '{"schema_version":"canary_arm_sample.v1","stage_id":"%s","metric":"score","samples":2,"passed":%s,"mean":%s}\n' "$stage" "$passed" "$mean"
"#;

    fn request(
        promotion_id: &str,
        old: &str,
        checkpoint: &str,
        bundle_root: PathBuf,
    ) -> CanaryRunRequest {
        CanaryRunRequest {
            promotion_id: promotion_id.to_string(),
            fence: PromotionFence::new("promotion-owner", 1),
            old_revision: old.to_string(),
            checkpoint_revision: checkpoint.to_string(),
            bundle_source_root: bundle_root,
        }
    }

    #[test]
    fn a_paired_canary_passes_only_when_the_checkpoint_improves() {
        if !docker_available() {
            eprintln!("skipping: no docker daemon available");
            return;
        }
        let Some(image) = available_image() else {
            eprintln!("skipping: no local digest-pinned image available");
            return;
        };
        let directory = tempdir().expect("tempdir");
        let (repo, old, checkpoint) = repository(directory.path());
        let worktrees = directory.path().join("worktrees");
        let bundle_root = bundle(directory.path(), MEASURING_PROGRAM);
        let runner = PromotionCanaryRunner::new(&repo, &worktrees, config(&image)).expect("runner");

        // Passing: both stages improve, so the assembled result is a pass.
        let passing = runner
            .run(
                &spec(vec![stage_spec("stage-1", 0.5), stage_spec("stage-2", 0.5)]),
                &request(PROMOTION_A, &old, &checkpoint, bundle_root.clone()),
            )
            .expect("paired canary passes");
        assert_eq!(passing.status, "pass");
        assert_eq!(passing.stages.len(), 2);
        assert_eq!(passing.stages[0].baseline_mean, 0.25);
        assert_eq!(passing.stages[0].candidate_mean, 1.0);
        assert_eq!(passing.stages[0].samples, 2);

        // Both arm containers and worktrees are gone after the run.
        assert_arm_resources_removed(PROMOTION_A, &repo, &worktrees);

        // A stage that demands more than the checkpoint delivers stops the run.
        let mut strict = stage_spec("stage-1", 0.5);
        strict.min_improvement = 2.0;
        let error = runner
            .run(
                &spec(vec![strict]),
                &request(PROMOTION_A, &old, &checkpoint, bundle_root.clone()),
            )
            .expect_err("no improvement");
        assert_eq!(error.code, PromotionErrorCode::CanaryStageFailed);
        assert_arm_resources_removed(PROMOTION_A, &repo, &worktrees);

        // A programme that does not measure the arm at all (a lying programme)
        // still cannot pass: it can only report its own numbers.
        let liar = bundle(
            directory.path(),
            "#!/bin/sh\nprintf '{\"schema_version\":\"canary_arm_sample.v1\",\"stage_id\":\"stage-1\",\"metric\":\"score\",\"samples\":2,\"passed\":2,\"mean\":1.0}\\n'\n",
        );
        let mut impossible = stage_spec("stage-1", 0.5);
        impossible.stage_id = "stage-9".to_string();
        impossible.args = vec!["--stage".to_string(), "stage-9".to_string()];
        let error = runner
            .run(
                &spec(vec![impossible]),
                &request(PROMOTION_A, &old, &checkpoint, liar),
            )
            .expect_err("mismatched stage");
        assert_eq!(error.code, PromotionErrorCode::CanaryResultInvalid);
    }

    #[test]
    fn a_hanging_or_flooding_programme_stops_the_run() {
        if !docker_available() {
            eprintln!("skipping: no docker daemon available");
            return;
        }
        let Some(image) = available_image() else {
            eprintln!("skipping: no local digest-pinned image available");
            return;
        };
        let directory = tempdir().expect("tempdir");
        let (repo, old, checkpoint) = repository(directory.path());
        let worktrees = directory.path().join("worktrees");
        let runner = PromotionCanaryRunner::new(&repo, &worktrees, config(&image)).expect("runner");

        // A hanging programme is killed and fails the stage.
        let hanging = bundle(directory.path(), "#!/bin/sh\nsleep 600\n");
        let mut short = spec(vec![stage_spec("stage-1", 0.5)]);
        short.timeout_ms = 2_000;
        let error = runner
            .run(&short, &request(PROMOTION_B, &old, &checkpoint, hanging))
            .expect_err("timeout");
        assert_eq!(error.code, PromotionErrorCode::CanaryStageFailed);
        assert!(error.message.contains("exceeded"), "{}", error.message);
        assert_arm_resources_removed(PROMOTION_B, &repo, &worktrees);

        // A flooding programme is rejected as an oversized document.
        let flooding = bundle(
            directory.path(),
            "#!/bin/sh\ni=0\nwhile [ $i -lt 20000 ]; do printf 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\\n'; i=$((i+1)); done\n",
        );
        let mut tiny = spec(vec![stage_spec("stage-1", 0.5)]);
        tiny.max_output_bytes = 4_096;
        let error = runner
            .run(&tiny, &request(PROMOTION_B, &old, &checkpoint, flooding))
            .expect_err("oversize");
        assert_eq!(error.code, PromotionErrorCode::CanaryResultInvalid);
        assert_arm_resources_removed(PROMOTION_B, &repo, &worktrees);
    }

    /// The two arm containers and worktrees of one promotion must not survive a
    /// run.
    fn assert_arm_resources_removed(promotion_id: &str, repo: &Path, worktrees: &Path) {
        let listing = Command::new("docker")
            .args([
                "ps",
                "-a",
                "--filter",
                &format!("name=cs-run-{promotion_id}"),
                "--format",
                "{{.Names}}",
            ])
            .output()
            .expect("docker ps");
        let names = String::from_utf8_lossy(&listing.stdout);
        assert!(names.trim().is_empty(), "arm containers survived: {names}");
        let worktree_listing = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["worktree", "list", "--porcelain"])
            .output()
            .expect("worktree list");
        let listing = String::from_utf8_lossy(&worktree_listing.stdout);
        assert!(
            !listing.contains(promotion_id),
            "arm worktrees survived: {listing}"
        );
        if worktrees.is_dir() {
            let leftovers: Vec<_> = std::fs::read_dir(worktrees)
                .expect("read worktrees")
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.file_name().to_string_lossy().to_string())
                .filter(|name| name.contains(promotion_id))
                .collect();
            assert!(
                leftovers.is_empty(),
                "arm worktrees survived: {leftovers:?}"
            );
        }
    }
}
