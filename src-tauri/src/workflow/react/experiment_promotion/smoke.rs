//! Phase 2I end-to-end smoke over a real temporary Git repository and a real
//! experiment domain.
//!
//! These tests are the executable form of the U-8 smoke: they drive the *actual*
//! supervisor, the actual checkpoint owner, the actual paired canary (in a real
//! digest-pinned container, when one is available locally) and the actual
//! durable store against a real repository, then audit the Git effect.
//!
//! What they deliberately do **not** do is run an LLM: Phase 2I adds no LLM,
//! tool or network effect (INV-9), and candidate generation stays behind the 2B
//! admission boundary. The two arms are therefore prepared as durable campaign
//! jobs with published artifacts, exactly as the 2G scheduler would leave them.

use super::scheduler::{PromotionSupervisor, PromotionSupervisorConfig, PromotionTickOutcome};
use super::types::{PromotionRequestV1, PromotionState, PROMOTION_REQUEST_V1};
use crate::db::experiment_promotion::{ExperimentPromotionStore, SubmitOutcome};
use crate::db::experiment_schedule::ExperimentScheduleStore;
use crate::db::MainStore;
use crate::headless::promotion_targets::PROMOTION_TARGET_DIR;
use crate::workflow::react::experiment_promotion::policy::{
    CanaryStageSpecV1, MetricDirection, PromotionCanarySpecV1, PromotionMetricRuleV1,
    PromotionPolicyV1, PROMOTION_POLICY_V1, PROMOTION_TARGET_V1,
};
use crate::workflow::react::experiment_promotion::types::{
    PromotionBudgetFactsV1, PromotionEvidenceV1, PromotionMetricFactV1,
    PromotionVerifierIdentityV1, PROMOTION_EVIDENCE_V1,
};
use crate::workflow::react::experiment_schedule::types::{
    ExecutionProfileV1, MountSpecV1, NetworkPolicyV1, OwnerKind, ResourceLimitsV1,
    EXECUTION_PROFILE_V1,
};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use tempfile::tempdir;

pub const BRANCH: &str = "refs/heads/experiment/2i";
pub const PROFILE_REF: &str = "canary";
pub const TARGET_REF: &str = "local-dev";
const BUNDLE_REF: &str = "smoke-tools";
pub const T0: u64 = 1_700_000_000_000;
pub const LEASE_MS: u64 = 15 * 60 * 1_000;

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

pub fn git_ok(cwd: &Path, args: &[&str]) -> String {
    let output = git(cwd, args);
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

pub fn docker_available() -> bool {
    Command::new("docker")
        .args(["version", "--format", "{{.Server.Version}}"])
        .stdin(Stdio::null())
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Serialises every Docker-backed test: the daemon and the machine's container
/// budget are shared resources, and parallel canary runs contend for them.
/// Tests that do not use containers are unaffected.
pub static DOCKER_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A locally available digest-pinned image, for callers that need to decide
/// whether to skip.
pub fn image_or_skip() -> Option<String> {
    available_image()
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
        if let Ok(Some(pin)) =
            crate::workflow::react::experiment_owner::docker::local_image_pin("docker", reference)
        {
            return Some(pin);
        }
    }
    None
}

/// The digest the canary programme reports for an arm that has the improvement.
pub const IMPROVED_MEAN: f64 = 1.0;
/// The digest the canary programme reports for an arm without it.
pub const BASE_MEAN: f64 = 0.25;

/// A real base repository whose `experiment/2i` branch points at one commit.
pub fn repository(directory: &Path) -> (PathBuf, String) {
    let repo = directory.join("base");
    std::fs::create_dir_all(&repo).expect("mkdir repo");
    git_ok(&repo, &["init", "--quiet"]);
    std::fs::write(repo.join("app.txt"), "base\n").expect("write app");
    git_ok(&repo, &["add", "-A"]);
    git_ok(&repo, &["commit", "--quiet", "-m", "base"]);
    git_ok(&repo, &["branch", "experiment/2i"]);
    let head = git_ok(&repo, &["rev-parse", "HEAD"]);
    (repo, head)
}

/// The canary programme bundle, written as an allowlisted bundle the registry
/// can stage and verify.
pub fn write_bundle(domain: &Path) {
    write_bundle_with_program(domain, &canary_program(None));
}

/// The tail every smoke canary programme shares: emit one strict arm sample
/// whose metric is the number of improvement files in the arm's workspace, so
/// the metric rises with each successful candidate and falls when one is
/// reverted.
pub const CANARY_TAIL: &str = r#"count=$(ls /workspace/improvement* 2>/dev/null | wc -l | tr -d ' ')
case "$count" in
  0) passed=0 ;;
  1) passed=1 ;;
  *) passed=2 ;;
esac
printf '{"schema_version":"canary_arm_sample.v1","stage_id":"%s","metric":"score","samples":2,"passed":%s,"mean":%s}\n' "$stage" "$passed" "$count"
"#;

/// The canary programme: parse `--stage <id>` and then emit the arm sample.
pub fn canary_program(slow_secs: Option<u64>) -> String {
    let head = r#"#!/bin/sh
stage=""
while [ $# -gt 0 ]; do
  case "$1" in
    --stage) stage="$2"; shift 2 ;;
    *) shift ;;
  esac
done
"#;
    let slow = slow_secs
        .map(|secs| format!("sleep {secs}\n"))
        .unwrap_or_default();
    format!("{head}{slow}{CANARY_TAIL}")
}

/// Writes the allowlisted bundle with a caller-supplied canary programme, so a
/// scenario can control the programme's duration or behaviour.
pub fn write_bundle_with_program(domain: &Path, program_body: &str) {
    let root = domain.join("bundles-allowlist").join(BUNDLE_REF);
    std::fs::create_dir_all(root.join("tools")).expect("mkdir bundle");
    let program = root.join("tools/canary");
    std::fs::write(&program, program_body).expect("write canary");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755))
            .expect("chmod canary");
    }
    let manifest = crate::workflow::react::experiment_schedule::types::BundleManifestV1 {
        schema_version: crate::workflow::react::experiment_schedule::types::BUNDLE_MANIFEST_V1
            .to_string(),
        bundle_ref: BUNDLE_REF.to_string(),
        bundle_version: "1".to_string(),
        content_digest: String::new(),
        files: vec![
            crate::workflow::react::experiment_schedule::types::BundleFileV1 {
                relative_path: "tools/canary".to_string(),
                size_bytes: std::fs::metadata(&program).expect("stat").len(),
                sha256: crate::workflow::react::experiment_owner::patch::digest_hex(
                    &std::fs::read(&program).expect("read canary"),
                ),
                mode: 0o755,
                executable: true,
                symlink_target: None,
            },
        ],
        mcp_servers: Vec::new(),
        skills: Vec::new(),
        env_secret_refs: Vec::new(),
    };
    let mut manifest = manifest;
    manifest.content_digest = manifest.computed_content_digest();
    manifest
        .validate()
        .expect("the smoke bundle manifest is valid");
    std::fs::write(
        root.join("bundle-manifest.json"),
        serde_json::to_vec_pretty(&manifest).expect("manifest"),
    )
    .expect("write manifest");
}

pub fn write_profile(domain: &Path, image: &str) {
    let profile = ExecutionProfileV1 {
        schema_version: EXECUTION_PROFILE_V1.to_string(),
        profile_ref: PROFILE_REF.to_string(),
        owner_kind: OwnerKind::PersistentDocker,
        base_repo_ref: "repo:primary".to_string(),
        base_revision: BRANCH.to_string(),
        image_reference: Some(image.to_string()),
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
        allowed_bundle_refs: vec![BUNDLE_REF.to_string()],
        input_patch_ref: None,
        input_patch_digest: None,
    };
    let profiles = domain.join(crate::headless::profiles::EXECUTION_PROFILE_DIR);
    std::fs::create_dir_all(&profiles).expect("profiles");
    std::fs::write(
        profiles.join(format!("{PROFILE_REF}.json")),
        serde_json::to_vec_pretty(&profile).expect("profile"),
    )
    .expect("write profile");
}

pub fn write_target(domain: &Path) {
    let target = crate::workflow::react::experiment_promotion::policy::PromotionTargetV1 {
        schema_version: PROMOTION_TARGET_V1.to_string(),
        target_ref: TARGET_REF.to_string(),
        base_repo_ref: "repo:primary".to_string(),
        branch_ref: BRANCH.to_string(),
        git_identity_name: "ChatSpeed Promotion".to_string(),
        git_identity_email: "promotion@chatspeed.local".to_string(),
        canary: PromotionCanarySpecV1 {
            execution_profile_ref: PROFILE_REF.to_string(),
            bundle_ref: BUNDLE_REF.to_string(),
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
            timeout_ms: 60_000,
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
    let targets = domain.join(PROMOTION_TARGET_DIR);
    std::fs::create_dir_all(&targets).expect("targets");
    std::fs::write(
        targets.join(format!("{TARGET_REF}.json")),
        serde_json::to_vec_pretty(&target).expect("target"),
    )
    .expect("write target");
}

/// A durable campaign whose two (or more) arms already reached `succeeded` with
/// published artifacts, exactly as the 2G scheduler leaves them.
pub struct SmokeCampaign {
    pub campaign_id: String,
    pub execution_profile_hash: String,
    /// The digest-bound fixture identity the schedule pinned.
    fixture: FixtureFacts,
    /// candidate key → (job id, run id, session id, patch digest)
    arms: std::collections::BTreeMap<String, ArmArtifacts>,
}

/// The digest-bound fixture identity, captured from the pinned catalog.
#[derive(Clone)]
struct FixtureFacts {
    fixture_digest: String,
    task_id: String,
    suite: String,
    dataset_id: String,
    dataset_version: u32,
    split: String,
}

#[derive(Clone)]
struct ArmArtifacts {
    job_id: String,
    run_id: String,
    session_id: String,
    patch_sha256: String,
    /// Digest of the published patch manifest row.
    manifest_sha256: String,
    /// Digest of the arm's published job summary (the 2A artifact row).
    summary_sha256: String,
}

/// Builds the durable campaign and drives every arm to `succeeded`, publishing
/// each candidate's patch into the domain artifact root.
///
/// `candidates` maps a candidate key to the patch bytes its job published; the
/// baseline arm publishes only its summary.
#[allow(clippy::too_many_lines)]
pub fn smoke_campaign(
    store: &Arc<MainStore>,
    schedule: &ExperimentScheduleStore,
    domain: &Path,
    repo: &Path,
    candidates: &[(&str, Option<Vec<u8>>)],
) -> SmokeCampaign {
    use crate::workflow::react::experiment_schedule::fixture;
    use crate::workflow::react::experiment_schedule::types::{
        parse_and_validate_campaign_schedule_request, CAMPAIGN_SCHEDULE_V1,
    };

    let resolved = fixture::resolve_task("chatspeed-smoke", "smoke_reply_ok").expect("fixture");
    let fixture = FixtureFacts {
        fixture_digest: resolved.task_ref().manifest_digest.clone(),
        task_id: resolved.task_ref().task_id.clone(),
        suite: resolved.task_ref().suite.clone(),
        dataset_id: resolved.task_ref().dataset_id.clone(),
        dataset_version: resolved.task_ref().dataset_version,
        split: resolved.task_ref().split.clone(),
    };
    let mut candidate_values = Vec::new();
    candidate_values.push(serde_json::json!({
        "candidate_key": "baseline",
        "kind": "baseline",
    }));
    for (key, _) in candidates {
        candidate_values.push(serde_json::json!({
            "candidate_key": key,
            "kind": "candidate",
            "mutable_surface": ["agent_prompt_ref"],
            "agent_prompt_ref": "smoke-terse",
            "prompt_hash": "0".repeat(64),
        }));
    }
    let request = parse_and_validate_campaign_schedule_request(&serde_json::json!({
        "schema_version": CAMPAIGN_SCHEDULE_V1,
        "plan": {
            "schema_version": "campaign_plan.v1",
            "campaign_key": "smoke-2i",
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
            "candidates": candidate_values,
        },
        "fixture_refs": [serde_json::to_value(resolved.task_ref()).expect("fixture ref")],
        "execution_profile_ref": PROFILE_REF,
        "bundle_refs": [BUNDLE_REF],
    }))
    .expect("the smoke schedule request is valid");
    let profile_hash = serde_json::from_slice::<ExecutionProfileV1>(
        &std::fs::read(
            domain
                .join(crate::headless::profiles::EXECUTION_PROFILE_DIR)
                .join(format!("{PROFILE_REF}.json")),
        )
        .expect("profile bytes"),
    )
    .expect("profile")
    .profile_hash();
    let outcome = schedule
        .schedule_campaign(&request, "smoke-key", &profile_hash, T0)
        .expect("schedule");
    let campaign_id = outcome.accepted.campaign_id.clone();

    // Drive every job to `succeeded` and publish its artifacts.
    let runtime = store.db_runtime().expect("runtime");
    let mut arms = std::collections::BTreeMap::new();
    let artifacts_root = domain.join("artifacts");
    let records = schedule.list_jobs(&campaign_id).expect("jobs");
    for record in &records {
        let job_id = record.job.job_id.clone();
        let candidate_key = record.job.candidate_key.clone();
        let run_id = format!("run-{candidate_key}");
        let session_id = format!("session-{candidate_key}");
        runtime
            .write_blocking({
                let job_id = job_id.clone();
                let run_id = run_id.clone();
                let session_id = session_id.clone();
                move |conn| {
                    conn.execute(
                        "UPDATE experiment_campaign_jobs
                            SET state = 'succeeded',
                                dispatch_marker = 'confirmed',
                                run_id = ?2,
                                session_id = ?3,
                                owner_id = NULL,
                                lease_expires_at_ms = NULL
                          WHERE job_id = ?1",
                        rusqlite::params![job_id, run_id, session_id],
                    )?;
                    Ok(())
                }
            })
            .expect("drive job to succeeded");

        let job_dir = artifacts_root.join("jobs").join(&job_id);
        std::fs::create_dir_all(&job_dir).expect("artifact dir");
        let mut rows: Vec<(&str, String, String, Option<String>)> = Vec::new();
        let summary = format!("{{\"job_id\":\"{job_id}\"}}\n");
        let summary_digest =
            crate::workflow::react::experiment_owner::patch::digest_hex(summary.as_bytes());
        std::fs::write(job_dir.join("job-summary.json"), &summary).expect("write summary");
        rows.push((
            "job_summary",
            "jobs/".to_string() + &job_id + "/job-summary.json",
            summary_digest.clone(),
            None,
        ));
        arms.insert(
            candidate_key.clone(),
            ArmArtifacts {
                job_id: job_id.clone(),
                run_id: run_id.clone(),
                session_id: session_id.clone(),
                // The baseline publishes no patch; its digest is never consulted.
                patch_sha256: crate::workflow::react::experiment_owner::patch::digest_hex(b""),
                manifest_sha256: String::new(),
                summary_sha256: summary_digest.clone(),
            },
        );
        if candidate_key != "baseline" {
            let patch = candidates
                .iter()
                .find(|(key, _)| *key == candidate_key)
                .and_then(|(_, patch)| patch.clone())
                .unwrap_or_default();
            let patch_digest = crate::workflow::react::experiment_owner::patch::digest_hex(&patch);
            std::fs::write(job_dir.join("patch.diff"), &patch).expect("write patch");
            let manifest = serde_json::json!({
                "schema_version": crate::workflow::react::experiment_owner::patch::PATCH_MANIFEST_V1,
                "job_id": job_id,
                "run_id": run_id,
                "session_id": session_id,
                "candidate_key": candidate_key,
                "base_revision": BRANCH,
                "diff_sha256": patch_digest,
                "diff_size_bytes": patch.len(),
                "files": [],
                "created_at_ms": T0,
            });
            let manifest_body = serde_json::to_vec_pretty(&manifest).expect("manifest");
            let manifest_digest =
                crate::workflow::react::experiment_owner::patch::digest_hex(&manifest_body);
            std::fs::write(job_dir.join("patch-manifest.json"), &manifest_body)
                .expect("write manifest");
            rows.push((
                "output_patch",
                "jobs/".to_string() + &job_id + "/patch.diff",
                patch_digest.clone(),
                Some(BRANCH.to_string()),
            ));
            rows.push((
                "patch_manifest",
                "jobs/".to_string() + &job_id + "/patch-manifest.json",
                manifest_digest.clone(),
                Some(BRANCH.to_string()),
            ));
            arms.insert(
                candidate_key.clone(),
                ArmArtifacts {
                    job_id: job_id.clone(),
                    run_id: run_id.clone(),
                    session_id: session_id.clone(),
                    patch_sha256: patch_digest.clone(),
                    manifest_sha256: manifest_digest,
                    summary_sha256: summary_digest.clone(),
                },
            );
        }
        for (kind, relative_path, sha256, base_revision) in rows {
            let size = std::fs::metadata(artifacts_root.join(&relative_path))
                .expect("artifact file")
                .len();
            runtime
                .write_blocking({
                    let job_id = job_id.clone();
                    let kind = kind.to_string();
                    let relative_path = relative_path.clone();
                    let sha256 = sha256.clone();
                    let base_revision = base_revision.clone();
                    let run_id = run_id.clone();
                    let session_id = session_id.clone();
                    let candidate_key = candidate_key.clone();
                    move |conn| {
                        conn.execute(
                            "INSERT OR REPLACE INTO experiment_job_artifacts (
                                artifact_id, job_id, kind, relative_path, sha256, size_bytes,
                                base_revision, run_id, session_id, candidate_key, created_at_ms
                             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                            rusqlite::params![
                                format!("{job_id}:{kind}"),
                                job_id,
                                kind,
                                relative_path,
                                sha256,
                                size as i64,
                                base_revision,
                                run_id,
                                session_id,
                                candidate_key,
                                T0 as i64,
                            ],
                        )?;
                        Ok(())
                    }
                })
                .expect("record artifact");
        }
    }
    // The patch must apply onto the branch head, which it does by construction:
    // every candidate adds (or removes) `improvement.txt`.
    let _ = repo;
    SmokeCampaign {
        campaign_id,
        execution_profile_hash: serde_json::from_slice::<ExecutionProfileV1>(
            &std::fs::read(
                domain
                    .join(crate::headless::profiles::EXECUTION_PROFILE_DIR)
                    .join(format!("{PROFILE_REF}.json")),
            )
            .expect("profile bytes"),
        )
        .expect("profile")
        .profile_hash(),
        fixture,
        arms,
    }
}

pub fn evidence(
    campaign: &SmokeCampaign,
    candidate_key: &str,
    baseline_mean: f64,
    candidate_mean: f64,
) -> PromotionEvidenceV1 {
    let arm = &campaign.arms[candidate_key];
    PromotionEvidenceV1 {
        schema_version: PROMOTION_EVIDENCE_V1.to_string(),
        campaign_id: campaign.campaign_id.clone(),
        candidate_key: candidate_key.to_string(),
        baseline_job_id: campaign.arms["baseline"].job_id.clone(),
        candidate_job_id: arm.job_id.clone(),
        baseline_run_id: campaign.arms["baseline"].run_id.clone(),
        candidate_run_id: arm.run_id.clone(),
        candidate_session_id: arm.session_id.clone(),
        baseline_artifact_hash: campaign.arms["baseline"].summary_sha256.clone(),
        candidate_artifact_hash: arm.summary_sha256.clone(),
        baseline_evaluation_hash: "c".repeat(64),
        candidate_evaluation_hash: "d".repeat(64),
        baseline_verdict_hash: "e".repeat(64),
        candidate_verdict_hash: "f".repeat(64),
        fixture_ref: "smoke-tools".to_string(),
        fixture_digest: campaign.fixture.fixture_digest.clone(),
        task_id: campaign.fixture.task_id.clone(),
        suite: campaign.fixture.suite.clone(),
        dataset_id: campaign.fixture.dataset_id.clone(),
        dataset_version: campaign.fixture.dataset_version,
        split: campaign.fixture.split.clone(),
        execution_profile_ref: PROFILE_REF.to_string(),
        execution_profile_hash: campaign.execution_profile_hash.clone(),
        patch_manifest_hash: arm.manifest_sha256.clone(),
        patch_sha256: arm.patch_sha256.clone(),
        base_revision: BRANCH.to_string(),
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
            baseline_mean,
            candidate_mean,
        }],
    }
}

pub fn supervisor(store: &Arc<MainStore>, domain: &Path, repo: &Path) -> PromotionSupervisor {
    PromotionSupervisor::new(
        ExperimentPromotionStore::new(store.clone()),
        ExperimentScheduleStore::new(store.clone()),
        PromotionSupervisorConfig {
            domain_root: domain.to_path_buf(),
            base_repo: Some(repo.to_path_buf()),
            owner_id: "promotion-supervisor".to_string(),
            lease_ms: LEASE_MS,
        },
    )
}

/// Submits one promotion and ticks the supervisor to its terminal state.
fn run_promotion(
    supervisor: &PromotionSupervisor,
    promotions: &ExperimentPromotionStore,
    request: &PromotionRequestV1,
) -> (String, PromotionState) {
    let promotion_id = request.promotion_id();
    match promotions.submit(request, &promotion_id, &promotion_id, T0) {
        Ok(SubmitOutcome::Created(_)) | Ok(SubmitOutcome::Existing(_)) => {}
        Err(error) => panic!("submit failed: {error}"),
    }
    let mut state = PromotionState::Queued;
    // Each tick advances the clock past the lease, so the supervisor re-claims
    // the row and exercises the same adopt/recovery path a restart would.
    for attempt in 0..12u32 {
        let now = T0 + u64::from(attempt) * (LEASE_MS + 1);
        match supervisor.tick(now).expect("tick") {
            PromotionTickOutcome::Idle => {}
            PromotionTickOutcome::Progressed { .. } => {}
            PromotionTickOutcome::Terminal {
                state: next, code, ..
            } => {
                return (code, next);
            }
            PromotionTickOutcome::Parked { code, .. } => {
                return (code, PromotionState::UnknownManual)
            }
        }
        let record = promotions.get(&promotion_id).expect("record");
        state = record.state;
    }
    panic!("promotion did not terminate; last state {state:?}");
}

/// Asserts the branch is at `expected`, the checkpoint ref exists, the checkpoint
/// commit carries the English subject and every evidence trailer, and the base
/// repository worktree/index/HEAD are untouched.
fn assert_branch_at(repo: &Path, expected: &str) {
    assert_eq!(
        git_ok(repo, &["rev-parse", BRANCH]),
        expected,
        "the registered branch did not advance as expected"
    );
}

/// A completed promotion advances the registered branch, keeps its checkpoint and
/// audits offline; a second one forms a linear history on top of the first.
#[test]
#[allow(clippy::too_many_lines)]
fn smoke_two_promotions_form_linear_commits_and_a_failure_does_not_advance() {
    let _docker_gate = DOCKER_GATE.lock().expect("docker gate");
    if !docker_available() {
        eprintln!("skipping: no docker daemon available");
        return;
    }
    let Some(image) = available_image() else {
        eprintln!("skipping: no local digest-pinned image available");
        return;
    };
    let directory = tempdir().expect("tempdir");
    let domain = directory.path().to_path_buf();
    let (repo, base_head) = repository(&domain);
    write_profile(&domain, &image);
    write_target(&domain);
    write_bundle(&domain);
    let store = Arc::new(MainStore::new(domain.join("chatspeed.db")).expect("store"));
    let schedule = ExperimentScheduleStore::new(store.clone());

    // Three candidates: two improve (each on top of the previous head, so their
    // patches touch different files), one regresses (it removes the first
    // improvement).
    let improve_a = b"--- /dev/null\n+++ b/improvement.txt\n@@ -0,0 +1 @@\n+better\n".to_vec();
    let improve_b = b"--- /dev/null\n+++ b/improvement2.txt\n@@ -0,0 +1 @@\n+better2\n".to_vec();
    let regress =
        b"--- a/improvement.txt\n+++ b/improvement.txt\n@@ -1 +0,0 @@\n-better\n".to_vec();
    let campaign = smoke_campaign(
        &store,
        &schedule,
        &domain,
        &repo,
        &[
            ("prompt-a", Some(improve_a)),
            ("prompt-b", Some(improve_b)),
            ("prompt-c", Some(regress)),
        ],
    );
    let supervisor = supervisor(&store, &domain, &repo);
    let promotions = ExperimentPromotionStore::new(store.clone());

    // --- First promotion: improves, so the branch moves to its checkpoint.
    let first = super::types::PromotionRequestV1 {
        schema_version: PROMOTION_REQUEST_V1.to_string(),
        campaign_id: campaign.campaign_id.clone(),
        candidate_key: "prompt-a".to_string(),
        target_ref: TARGET_REF.to_string(),
        evidence: evidence(&campaign, "prompt-a", BASE_MEAN, IMPROVED_MEAN),
    };
    let (code, state) = run_promotion(&supervisor, &promotions, &first);
    assert_eq!(state, PromotionState::Promoted, "code {code}");
    let first_commit = git_ok(&repo, &["rev-parse", BRANCH]);
    assert_ne!(first_commit, base_head, "the branch must move");
    // Linear: the new head's parent is the base commit.
    assert_eq!(
        git_ok(&repo, &["rev-parse", &format!("{first_commit}^")]),
        base_head
    );
    // The checkpoint ref exists and the commit is the English checkpoint commit.
    let checkpoint_ref = format!("refs/chatspeed/checkpoints/{}", first.promotion_id());
    assert_eq!(git_ok(&repo, &["rev-parse", &checkpoint_ref]), first_commit);
    let body = git_ok(&repo, &["log", "-1", "--format=%B", &first_commit]);
    assert!(body.starts_with(&format!(
        "experiment(promotion): checkpoint {}",
        first.promotion_id()
    )));
    for trailer in [
        "Promotion-Id",
        "Evidence-Hash",
        "Patch-Sha256",
        "Base-Revision",
        "Target-Ref",
    ] {
        assert!(
            body.contains(&format!("{trailer}:")),
            "missing trailer {trailer} in {body}"
        );
    }
    // The record stores the same checkpoint the repository shows.
    let record = promotions.get(&first.promotion_id()).expect("record");
    assert_eq!(
        record.checkpoint_commit.as_deref(),
        Some(first_commit.as_str())
    );
    assert_eq!(record.branch_intent.as_str(), "completed");

    // --- Second promotion: also improves, on top of the first.
    let second = super::types::PromotionRequestV1 {
        schema_version: PROMOTION_REQUEST_V1.to_string(),
        campaign_id: campaign.campaign_id.clone(),
        candidate_key: "prompt-b".to_string(),
        target_ref: TARGET_REF.to_string(),
        evidence: evidence(&campaign, "prompt-b", BASE_MEAN, IMPROVED_MEAN),
    };
    let (code, state) = run_promotion(&supervisor, &promotions, &second);
    assert_eq!(state, PromotionState::Promoted, "code {code}");
    let second_commit = git_ok(&repo, &["rev-parse", BRANCH]);
    assert_ne!(second_commit, first_commit);
    assert_eq!(
        git_ok(&repo, &["rev-parse", &format!("{second_commit}^")]),
        first_commit,
        "consecutive successful nodes must form linear local commits"
    );

    // --- Third promotion: the campaign metrics improve, but the workspace
    // canary regresses (the candidate reverts the first improvement), so the
    // canary gate fails and the branch stays.
    let third = super::types::PromotionRequestV1 {
        schema_version: PROMOTION_REQUEST_V1.to_string(),
        campaign_id: campaign.campaign_id.clone(),
        candidate_key: "prompt-c".to_string(),
        target_ref: TARGET_REF.to_string(),
        evidence: evidence(&campaign, "prompt-c", BASE_MEAN, IMPROVED_MEAN),
    };
    let (code, state) = run_promotion(&supervisor, &promotions, &third);
    assert_eq!(state, PromotionState::CanaryFailed, "code {code}");
    assert_eq!(code, "canary_stage_failed");
    assert_eq!(
        git_ok(&repo, &["rev-parse", BRANCH]),
        second_commit,
        "a failed canary must never advance the branch"
    );
    // The failed attempt keeps its checkpoint evidence: the commit and ref were
    // created before the canary ran and are never discarded.
    let failed = promotions.get(&third.promotion_id()).expect("record");
    assert_eq!(failed.state, PromotionState::CanaryFailed);
    assert_eq!(failed.branch_intent.as_str(), "not_started");
    let failed_ref = format!("refs/chatspeed/checkpoints/{}", third.promotion_id());
    assert_eq!(
        git_ok(&repo, &["rev-parse", &failed_ref]),
        failed
            .checkpoint_commit
            .clone()
            .expect("the failed attempt keeps its checkpoint"),
        "the failed attempt's checkpoint ref must survive"
    );

    // --- Audit: no remote effect anywhere, and the working tree is clean.
    assert!(
        git(&repo, &["config", "--get-regexp", "^remote\\."])
            .stdout
            .is_empty(),
        "the smoke repository must have no remote configured"
    );
    assert!(git_ok(&repo, &["status", "--porcelain"]).is_empty());
    assert_branch_at(&repo, &second_commit);
}

/// A supervisor restart after the checkpoint intent recreates the checkpoint
/// exactly once and still reaches `promoted`.
#[test]
fn smoke_a_restart_recreates_the_checkpoint_exactly_once() {
    let _docker_gate = DOCKER_GATE.lock().expect("docker gate");
    if !docker_available() {
        eprintln!("skipping: no docker daemon available");
        return;
    }
    let Some(image) = available_image() else {
        eprintln!("skipping: no local digest-pinned image available");
        return;
    };
    let directory = tempdir().expect("tempdir");
    let domain = directory.path().to_path_buf();
    let (repo, base_head) = repository(&domain);
    write_profile(&domain, &image);
    write_target(&domain);
    write_bundle(&domain);
    let store = Arc::new(MainStore::new(domain.join("chatspeed.db")).expect("store"));
    let schedule = ExperimentScheduleStore::new(store.clone());
    let improve = b"--- /dev/null\n+++ b/improvement.txt\n@@ -0,0 +1 @@\n+better\n".to_vec();
    let campaign = smoke_campaign(
        &store,
        &schedule,
        &domain,
        &repo,
        &[("prompt-a", Some(improve))],
    );
    let promotions = ExperimentPromotionStore::new(store.clone());
    let request = PromotionRequestV1 {
        schema_version: PROMOTION_REQUEST_V1.to_string(),
        campaign_id: campaign.campaign_id.clone(),
        candidate_key: "prompt-a".to_string(),
        target_ref: TARGET_REF.to_string(),
        evidence: evidence(&campaign, "prompt-a", BASE_MEAN, IMPROVED_MEAN),
    };
    let promotion_id = request.promotion_id();
    promotions
        .submit(&request, &promotion_id, &promotion_id, T0)
        .expect("submit");

    // One tick: the gate passes and the checkpoint intent is durable, but the
    // effect has not run yet. This is the crash point the restart must recover
    // from.
    {
        let supervisor = supervisor(&store, &domain, &repo);
        let outcome = supervisor.tick(T0).expect("tick");
        assert!(
            matches!(outcome, PromotionTickOutcome::Progressed { .. }),
            "the first tick must reach checkpointing, got {outcome:?}"
        );
        let record = promotions.get(&promotion_id).expect("record");
        assert_eq!(record.state, PromotionState::Checkpointing);
        assert_eq!(record.checkpoint_intent.as_str(), "intent_recorded");
        assert!(record.checkpoint_commit.is_none());
    }
    // The checkpoint ref provably does not exist yet, so the restart may create
    // it (this is exactly what the recovery classifier requires).
    let checkpoint_ref = format!("refs/chatspeed/checkpoints/{promotion_id}");
    assert!(!git(
        &repo,
        &["rev-parse", "--verify", "--quiet", &checkpoint_ref]
    )
    .status
    .success());

    // A fresh supervisor (the old worker is gone) finishes the attempt.
    {
        let supervisor = supervisor(&store, &domain, &repo);
        let mut state = PromotionState::Checkpointing;
        for attempt in 0..12u32 {
            let now = T0 + u64::from(attempt) * (LEASE_MS + 1);
            match supervisor.tick(now).expect("tick") {
                PromotionTickOutcome::Idle => {}
                PromotionTickOutcome::Progressed { .. } => {}
                PromotionTickOutcome::Terminal { state: next, .. } => {
                    state = next;
                    break;
                }
                PromotionTickOutcome::Parked { .. } => {
                    state = PromotionState::UnknownManual;
                    break;
                }
            }
            state = promotions.get(&promotion_id).expect("record").state;
        }
        assert_eq!(state, PromotionState::Promoted);
    }

    // Exactly one checkpoint commit exists for this attempt, and it is the
    // commit the branch now points at.
    let head = git_ok(&repo, &["rev-parse", BRANCH]);
    assert_eq!(git_ok(&repo, &["rev-parse", &checkpoint_ref]), head);
    assert_eq!(
        git_ok(
            &repo,
            &["rev-list", "--count", &format!("{base_head}..{head}")]
        ),
        "1",
        "the restart must not duplicate the checkpoint commit"
    );
    let body = git_ok(&repo, &["log", "-1", "--format=%B", &head]);
    assert!(body.contains(&format!("Promotion-Id: {promotion_id}")));
    // The recorded intent is completed exactly once in the journal.
    let journal = promotions.journal(&promotion_id).expect("journal");
    let intents: Vec<_> = journal
        .iter()
        .filter(|entry| entry.stage == "checkpoint_intent")
        .collect();
    assert_eq!(intents.len(), 1, "one intent, one effect");
    let creations: Vec<_> = journal
        .iter()
        .filter(|entry| entry.stage == "checkpoint_created")
        .collect();
    assert_eq!(creations.len(), 1, "the checkpoint was recorded once");
}

/// The smoke's own positive control: the secret scanner finds a planted
/// credential and finds nothing in a clean directory.
#[test]
fn smoke_the_audit_scanner_has_a_positive_control() {
    let directory = tempdir().expect("tempdir");
    let clean = directory.path().join("clean");
    let planted = directory.path().join("planted");
    std::fs::create_dir_all(&clean).expect("mkdir clean");
    std::fs::create_dir_all(&planted).expect("mkdir planted");
    std::fs::write(clean.join("audit.json"), "{\"state\":\"promoted\"}\n").expect("write clean");
    std::fs::write(
        planted.join("audit.json"),
        "{\"note\":\"sk-0123456789abcdef0123456789abcdef\"}\n",
    )
    .expect("write planted");
    assert_eq!(scan_directory(&clean), 0, "a clean directory has no hits");
    assert_eq!(
        scan_directory(&planted),
        1,
        "a planted credential must be found"
    );
}

/// Counts the credential markers in a directory tree. Kept in one place so the
/// smoke and any future audit share exactly one implementation.
fn scan_directory(root: &Path) -> usize {
    const MARKERS: &[&str] = &["sk-", "ghp_", "xoxb-", "-----BEGIN", "Bearer eyJ"];
    let mut hits = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if let Ok(body) = std::fs::read_to_string(&path) {
                if MARKERS
                    .iter()
                    .any(|marker| body.to_ascii_lowercase().contains(marker))
                {
                    hits += 1;
                }
            }
        }
    }
    hits
}

/// A deterministic checkpoint failure is recorded as a terminal rejection rather
/// than leaving the promotion in `checkpointing` to retry forever.
#[test]
fn a_checkpoint_failure_converges_without_touching_the_branch() {
    let directory = tempdir().expect("tempdir");
    let domain = directory.path().to_path_buf();
    let (repo, base_head) = repository(&domain);
    write_profile(
        &domain,
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    );
    write_target(&domain);
    write_bundle(&domain);
    let store = Arc::new(MainStore::new(domain.join("chatspeed.db")).expect("store"));
    let schedule = ExperimentScheduleStore::new(store.clone());
    let campaign = smoke_campaign(
        &store,
        &schedule,
        &domain,
        &repo,
        &[(
            "prompt-a",
            Some(b"--- /dev/null\n+++ b/improvement.txt\n@@ -0,0 +1 @@\n+better\n".to_vec()),
        )],
    );
    let promotions = ExperimentPromotionStore::new(store.clone());
    let request = PromotionRequestV1 {
        schema_version: PROMOTION_REQUEST_V1.to_string(),
        campaign_id: campaign.campaign_id.clone(),
        candidate_key: "prompt-a".to_string(),
        target_ref: TARGET_REF.to_string(),
        evidence: evidence(&campaign, "prompt-a", BASE_MEAN, IMPROVED_MEAN),
    };
    std::fs::write(domain.join("worktrees"), "not a directory\n")
        .expect("seed deterministic checkpoint failure");
    let (code, state) = run_promotion(&supervisor(&store, &domain, &repo), &promotions, &request);
    assert_eq!(state, PromotionState::Rejected);
    assert_eq!(code, "checkpoint_failed");
    assert_eq!(git_ok(&repo, &["rev-parse", BRANCH]), base_head);
    let record = promotions.get(&request.promotion_id()).expect("record");
    assert_eq!(record.error_code.as_deref(), Some("checkpoint_failed"));
    assert_eq!(record.checkpoint_intent.as_str(), "intent_recorded");
    assert!(
        record.owner_id.is_none(),
        "terminal rejection releases the lease"
    );
}

/// Every canary non-success converges to a terminal state and never touches the
/// branch:
/// - an untrustable result document (garbage instead of the strict arm sample)
///   is `canary_failed`, recorded once, never retried;
/// - an unusable environment (a pinned image that is not locally available) is
///   parked as `unknown_manual` instead of spinning on an effect that cannot be
///   reasoned about.
#[test]
fn a_canary_that_cannot_be_trusted_converges_without_touching_the_branch() {
    let _docker_gate = DOCKER_GATE.lock().expect("docker gate");
    // ---- untrustable result document --------------------------------------
    let directory = tempdir().expect("tempdir");
    let domain = directory.path().to_path_buf();
    let (repo, base_head) = repository(&domain);
    let Some(image) = image_or_skip() else {
        eprintln!("skipping: no local digest-pinned image available");
        return;
    };
    write_profile(&domain, &image);
    write_target(&domain);
    write_bundle_with_program(&domain, "#!/bin/sh\necho this is not a canary document\n");
    let store = Arc::new(MainStore::new(domain.join("chatspeed.db")).expect("store"));
    let schedule = ExperimentScheduleStore::new(store.clone());
    let improve = b"--- /dev/null\n+++ b/improvement.txt\n@@ -0,0 +1 @@\n+better\n".to_vec();
    let campaign = smoke_campaign(
        &store,
        &schedule,
        &domain,
        &repo,
        &[("prompt-a", Some(improve.clone()))],
    );
    let request = PromotionRequestV1 {
        schema_version: PROMOTION_REQUEST_V1.to_string(),
        campaign_id: campaign.campaign_id.clone(),
        candidate_key: "prompt-a".to_string(),
        target_ref: TARGET_REF.to_string(),
        evidence: evidence(&campaign, "prompt-a", BASE_MEAN, IMPROVED_MEAN),
    };
    let promotion_id = request.promotion_id();
    let harness = supervisor(&store, &domain, &repo);
    let promotions = ExperimentPromotionStore::new(store.clone());
    promotions
        .submit(&request, &promotion_id, &promotion_id, T0)
        .expect("submit");

    let mut state = PromotionState::Queued;
    for attempt in 0..12u32 {
        let now = T0 + u64::from(attempt) * (LEASE_MS + 1);
        match harness.tick(now).expect("tick") {
            PromotionTickOutcome::Terminal {
                state: next, code, ..
            } => {
                state = next;
                assert_eq!(code, "canary_result_invalid");
                break;
            }
            _ => state = promotions.get(&promotion_id).expect("record").state,
        }
    }
    assert_eq!(state, PromotionState::CanaryFailed);
    assert_eq!(
        git_ok(&repo, &["rev-parse", BRANCH]),
        base_head,
        "an untrustable canary result must never advance the branch"
    );
    let record = promotions.get(&promotion_id).expect("record");
    assert_eq!(record.error_code.as_deref(), Some("canary_result_invalid"));
    assert_eq!(record.branch_intent.as_str(), "not_started");
    assert!(
        record.checkpoint_commit.is_some(),
        "checkpoint evidence is retained"
    );

    // The row is terminal: further ticks never re-claim or re-run it.
    let attempt_before = record.attempt;
    for attempt in 12..16u32 {
        let now = T0 + u64::from(attempt) * (LEASE_MS + 1);
        assert_eq!(harness.tick(now).expect("tick"), PromotionTickOutcome::Idle);
    }
    assert_eq!(
        promotions.get(&promotion_id).expect("record").attempt,
        attempt_before,
        "a terminal promotion is never re-claimed"
    );

    // ---- unusable environment ---------------------------------------------
    let directory = tempdir().expect("tempdir");
    let domain = directory.path().to_path_buf();
    let (repo, base_head) = repository(&domain);
    // A syntactically valid digest that is not available locally: the owner
    // never pulls, so the canary environment is unusable.
    write_profile(&domain, &format!("sha256:{}", "f".repeat(64)));
    write_target(&domain);
    write_bundle(&domain);
    let store = Arc::new(MainStore::new(domain.join("chatspeed.db")).expect("store"));
    let schedule = ExperimentScheduleStore::new(store.clone());
    let campaign = smoke_campaign(
        &store,
        &schedule,
        &domain,
        &repo,
        &[("prompt-a", Some(improve))],
    );
    let request = PromotionRequestV1 {
        schema_version: PROMOTION_REQUEST_V1.to_string(),
        campaign_id: campaign.campaign_id.clone(),
        candidate_key: "prompt-a".to_string(),
        target_ref: TARGET_REF.to_string(),
        evidence: evidence(&campaign, "prompt-a", BASE_MEAN, IMPROVED_MEAN),
    };
    let promotion_id = request.promotion_id();
    let harness = supervisor(&store, &domain, &repo);
    let promotions = ExperimentPromotionStore::new(store.clone());
    promotions
        .submit(&request, &promotion_id, &promotion_id, T0)
        .expect("submit");

    let mut state = PromotionState::Queued;
    for attempt in 0..12u32 {
        let now = T0 + u64::from(attempt) * (LEASE_MS + 1);
        match harness.tick(now).expect("tick") {
            PromotionTickOutcome::Parked { code, .. } => {
                assert_eq!(code, "executor_unavailable");
                state = PromotionState::UnknownManual;
                break;
            }
            _ => state = promotions.get(&promotion_id).expect("record").state,
        }
    }
    assert_eq!(state, PromotionState::UnknownManual);
    assert_eq!(
        git_ok(&repo, &["rev-parse", BRANCH]),
        base_head,
        "a parked promotion must never have touched the branch"
    );
}
