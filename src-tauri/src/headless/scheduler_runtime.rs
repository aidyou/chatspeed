//! Wiring the durable scheduler into the headless runtime (Phase 2H).
//!
//! This module is the *only* place that connects the three otherwise
//! independent pieces:
//!
//! - the recoverable [`CampaignScheduler`] (durable queue + lease fencing),
//! - the execution owners (`HostWorktreeOwner` / `PersistentDockerOwner` /
//!   `HarborTaskOwner`), and
//! - the existing campaign run kernel in [`WorkflowApplicationService`].
//!
//! It deliberately adds no second lifecycle: a scheduled job is dispatched by
//! building the *existing* `CampaignRunRequestV1` and calling the *existing*
//! `campaign_run_core`, exactly like the 2F immediate route does (INV-1).
//!
//! The scheduler itself never learns about Tauri, SQLite or execution profiles;
//! everything server-side is resolved here from the domain configuration:
//!
//! - the execution profile is loaded from the domain's own profile directory,
//! - the base repository path comes from the operator's `--base-repo`, never
//!   from a job, a candidate or a caller,
//! - the bundle allowlist is a directory inside the domain, and
//! - the Harbor capability manifest, when present, is what makes a Harbor task
//!   environment ownable.

use crate::db::experiment_schedule::{CampaignRecord, JobRecord};
use crate::headless::profiles::ExecutionProfileRegistry;
use crate::workflow::react::application::WorkflowApplicationService;
use crate::workflow::react::campaign::{
    CampaignFixtureRefV1, CampaignPlanV1, CampaignRunRequestV1, CAMPAIGN_RUN_REQUEST_V1,
};
use crate::workflow::react::experiment_owner::bundle::BundleRegistry;
use crate::workflow::react::experiment_owner::capabilities::{
    CapabilityExecutionTarget, SecretEnvironment,
};
use crate::workflow::react::experiment_owner::docker::{DockerOwnerConfig, PersistentDockerOwner};
use crate::workflow::react::experiment_owner::harbor_task::{
    HarborTaskOwner, HARBOR_CAPABILITY_FILE_NAME,
};
use crate::workflow::react::experiment_owner::worktree::HostWorktreeOwner;
use crate::workflow::react::experiment_owner::{ExecutionOwner, PreparedWorkspace};
use crate::workflow::react::experiment_schedule::fixture;
use crate::workflow::react::experiment_schedule::scheduler::{
    DispatchedRun, RunVerdict, ScheduledDispatch, ScheduledRunKernel, SchedulerResources,
};
use crate::workflow::react::experiment_schedule::types::{
    OwnerKind, ScheduleError, ScheduleErrorCode,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Directory inside the domain that holds the staged-capable bundle allowlist.
pub const BUNDLE_ALLOWLIST_DIR: &str = "bundles-allowlist";

/// Where the Harbor adapter must write its capability manifest.
///
/// It is `<data-dir>/runtime/harbor-task-capability.json`: the adapter proves the
/// sandbox *inside the experiment domain* the instance owns, and
/// `tools/harbor/artifact_contract.py` derives the same path from the same
/// `--data-dir` value. A manifest written anywhere else is one this runtime
/// never reads, which would refuse every Harbor-owner job with
/// `ownership_mismatch`.
pub fn harbor_capability_path(domain_root: &Path) -> PathBuf {
    domain_root
        .join("runtime")
        .join(HARBOR_CAPABILITY_FILE_NAME)
}

fn runtime_error(code: ScheduleErrorCode, message: impl Into<String>) -> ScheduleError {
    ScheduleError::new(code, message)
}

/// Resolves every server-side resource a scheduled job needs.
pub struct DomainSchedulerResources {
    domain_root: PathBuf,
    profiles: ExecutionProfileRegistry,
    /// The operator-configured base repository for filesystem/container owners.
    base_repo: Option<PathBuf>,
    /// Where the Harbor adapter publishes the task capability manifest.
    harbor_capability: PathBuf,
}

impl DomainSchedulerResources {
    pub fn new(domain_root: impl Into<PathBuf>, base_repo: Option<PathBuf>) -> Self {
        let domain_root = domain_root.into();
        Self {
            profiles: ExecutionProfileRegistry::new(&domain_root),
            harbor_capability: harbor_capability_path(&domain_root),
            domain_root,
            base_repo,
        }
    }

    pub fn domain_root(&self) -> &Path {
        &self.domain_root
    }

    /// The base repository, or a fail-closed error.
    fn require_base_repo(&self) -> Result<PathBuf, ScheduleError> {
        self.base_repo.clone().ok_or_else(|| {
            runtime_error(
                ScheduleErrorCode::ExecutorUnavailable,
                "no --base-repo is configured for this domain, so the execution profile \
                 cannot be resolved to an owned workspace",
            )
        })
    }

    fn worktrees_root(&self) -> PathBuf {
        self.domain_root.join("worktrees")
    }
}

impl SchedulerResources for DomainSchedulerResources {
    fn owner_for(
        &self,
        job: &JobRecord,
        campaign: &CampaignRecord,
    ) -> Result<Box<dyn ExecutionOwner>, ScheduleError> {
        // The profile is resolved from the domain's own registry: a job can
        // never name a different one, and an unregistered profile fails closed.
        let profile = self.profiles.load(&campaign.execution_profile_ref)?;
        let profile_hash = profile.profile_hash();
        if campaign.execution_profile_hash.as_deref() != Some(profile_hash.as_str())
            || job.execution_profile_hash.as_deref() != campaign.execution_profile_hash.as_deref()
        {
            return Err(runtime_error(
                ScheduleErrorCode::InvalidExecutionProfile,
                "the registered execution profile no longer matches the durable schedule digest",
            ));
        }
        match profile.owner_kind {
            OwnerKind::HostWorktree => Ok(Box::new(HostWorktreeOwner::new(
                self.require_base_repo()?,
                self.worktrees_root(),
            ))),
            OwnerKind::PersistentDocker => {
                let base_repo = self.require_base_repo()?;
                let image_reference = profile.image_reference.clone().ok_or_else(|| {
                    runtime_error(
                        ScheduleErrorCode::ImageNotDigestPinned,
                        format!(
                            "execution profile '{}' selects the container owner but declares no \\
                             digest-pinned image",
                            profile.profile_ref
                        ),
                    )
                })?;
                let network_policy = profile.network_policy.clone().ok_or_else(|| {
                    runtime_error(
                        ScheduleErrorCode::NetworkPolicyUnsupported,
                        format!(
                            "execution profile '{}' declares no network policy",
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
                        .find(|mount| mount.source_kind == crate::workflow::react::experiment_schedule::types::MountSpecV1::SOURCE_WORKSPACE)
                        .map(|mount| mount.read_only)
                        .unwrap_or(false),
                    mounts: profile.mounts.clone(),
                };
                config.validate()?;
                Ok(Box::new(PersistentDockerOwner::new(
                    HostWorktreeOwner::new(base_repo, self.worktrees_root()),
                    config,
                )))
            }
            OwnerKind::HarborTask => {
                // A Harbor owner exists only when the adapter wrote a capability
                // manifest for this environment; otherwise the job fails closed.
                Ok(Box::new(HarborTaskOwner::load(&self.harbor_capability)?))
            }
        }
    }

    fn bundle_registry(&self) -> Option<BundleRegistry> {
        Some(BundleRegistry::new(
            self.domain_root.join(BUNDLE_ALLOWLIST_DIR),
        ))
    }

    fn secrets(&self) -> SecretEnvironment {
        // Credentials for bundles arrive with the restricted credential input of
        // the domain; nothing is read from the global user database.
        SecretEnvironment::default()
    }

    fn base_revision(&self, campaign: &CampaignRecord) -> String {
        self.profiles
            .load(&campaign.execution_profile_ref)
            .map(|profile| profile.base_revision)
            .unwrap_or_else(|_| "HEAD".to_string())
    }

    fn artifacts_root(&self) -> PathBuf {
        self.domain_root.join("artifacts")
    }

    fn bundles_root(&self) -> PathBuf {
        self.domain_root.join("bundles")
    }
}

/// Drives scheduled jobs through the existing campaign run kernel.
pub struct ScheduledCampaignKernel {
    svc: Arc<WorkflowApplicationService>,
    handle: tokio::runtime::Handle,
}

impl ScheduledCampaignKernel {
    pub fn new(svc: Arc<WorkflowApplicationService>, handle: tokio::runtime::Handle) -> Self {
        Self { svc, handle }
    }

    /// Builds the 2F run intent for one scheduled job.
    ///
    /// The fixture instruction is resolved from the pinned, checked-in catalog at
    /// dispatch time — the durable row stores only refs and digests — and the
    /// plan is the frozen plan the schedule persisted.
    fn run_request(request: &ScheduledDispatch<'_>) -> Result<CampaignRunRequestV1, ScheduleError> {
        let resolved = fixture::resolve_task(request.suite, request.task_id).map_err(|error| {
            runtime_error(
                ScheduleErrorCode::FixtureDigestMismatch,
                format!("{}: {}", error.code, error.message),
            )
        })?;
        Ok(CampaignRunRequestV1 {
            schema_version: CAMPAIGN_RUN_REQUEST_V1.to_string(),
            candidate_key: request.candidate_key.to_string(),
            fixture: CampaignFixtureRefV1 {
                suite: resolved.task_ref().suite,
                task_id: resolved.task_ref().task_id,
                instruction: resolved.task.instruction.clone(),
                instruction_hash: resolved.task.instruction_hash.clone(),
                dataset_id: resolved.task_ref().dataset_id,
                dataset_version: resolved.task_ref().dataset_version,
                split: resolved.task_ref().split,
                manifest_digest: resolved.task_ref().manifest_digest,
                task_digest: resolved.task_ref().task_digest,
                verifier_id: resolved.task.verifier_id.clone(),
                verifier_version: resolved.task.verifier_version.clone(),
            },
            plan: request.plan.clone(),
        })
    }

    /// Ensures the campaign's budget scope exists before the first run.
    ///
    /// A durable schedule never went through the 2F create route, so the kernel
    /// establishes the same scope the immediate path would have created. The
    /// status lookup distinguishes "already frozen" (`Ok(Some(_))`) from "not
    /// frozen yet" (`Ok(None)`): an *absent* scope is not success, and creating
    /// it is what lets the run kernel resolve the campaign at all. Treating an
    /// absent scope as present would leave every scheduled dispatch refused by
    /// the kernel with "campaign not found", after the dispatch intent had
    /// already been recorded.
    fn ensure_campaign_scope(
        &self,
        plan: &CampaignPlanV1,
        campaign_id: &str,
    ) -> Result<(), ScheduleError> {
        // A durable campaign may only ever freeze the scope of the plan it
        // persisted, so a divergence is a pre-dispatch rejection rather than an
        // orphan scope under a different id.
        let derived = crate::workflow::react::campaign::campaign_id_for_plan(&plan.plan_hash());
        if derived != campaign_id {
            return Err(runtime_error(
                ScheduleErrorCode::UnknownCampaign,
                format!(
                    "the durable campaign id '{campaign_id}' does not match the frozen plan it \
                     would freeze ('{derived}')"
                ),
            ));
        }
        match self.svc.main_store.get_budget_scope_status(campaign_id) {
            Ok(Some(_)) => return Ok(()),
            Ok(None) => {}
            Err(error) => {
                return Err(runtime_error(
                    ScheduleErrorCode::ExecutorUnavailable,
                    format!("failed to read the campaign budget scope: {error}"),
                ))
            }
        }
        crate::commands::workflow::campaign_create_core(&self.svc, plan.clone()).map_err(
            |error| {
                runtime_error(
                    ScheduleErrorCode::ExecutorUnavailable,
                    format!(
                        "failed to freeze the campaign budget scope: {}",
                        error.message
                    ),
                )
            },
        )?;
        Ok(())
    }
}

/// Fails closed unless the job's owner-prepared workspace is present.
///
/// A scheduled run may only execute inside the environment its owner prepared,
/// so a missing proof is an error rather than a reason to fall back to the host.
pub fn require_owner_workspace(
    workspace: Option<&PreparedWorkspace>,
) -> Result<&PreparedWorkspace, ScheduleError> {
    workspace.ok_or_else(|| {
        runtime_error(
            ScheduleErrorCode::OwnerExecutionContextUnavailable,
            "the scheduler dispatched a job without an owner-prepared workspace",
        )
    })
}

/// Maps a persisted workflow status onto the scheduler's explicit verdict.
///
/// Terminality and success are separate questions, and this is the only place
/// that answers both for the real production mapping (the in-memory
/// `WorkflowState` renders as `pending|thinking|executing|auditing|stopping|
/// paused|awaiting_*|completed|error|cancelled`). Any status this build does not
/// recognise is treated as "still running", so an unknown value can never make a
/// live run look finished or successful.
pub fn workflow_status_verdict(status: &str) -> RunVerdict {
    match status {
        "completed" => RunVerdict::Succeeded,
        "error" | "cancelled" | "failed" => RunVerdict::Failed,
        _ => RunVerdict::Running,
    }
}

/// Derives the owner-confirmed execution context of one scheduled job.
///
/// A container owner must have produced its instance (the container it fenced
/// with labels/token/generation); a Harbor task owner executes inside a sandbox
/// that is itself the isolation boundary; a filesystem-only owner has no
/// isolated execution environment at all, so a scheduled run under it is refused
/// instead of executing on the user's host (INV-4).
fn owner_execution_context(
    request: &ScheduledDispatch<'_>,
) -> Result<crate::commands::workflow::OwnerExecutionContext, ScheduleError> {
    match request.owner_kind {
        OwnerKind::HostWorktree => Err(runtime_error(
            ScheduleErrorCode::OwnerExecutionContextUnavailable,
            "a filesystem-only execution owner provides no isolated execution environment, so \
             this scheduled run is refused instead of executing on the host",
        )),
        OwnerKind::HarborTask => Ok(crate::commands::workflow::OwnerExecutionContext {
            owner_kind: OwnerKind::HarborTask,
            instance_name: None,
            image_reference: None,
            capabilities: None,
            capability_execution_target: Some(CapabilityExecutionTarget::InSandboxHost),
        }),
        OwnerKind::PersistentDocker => {
            let workspace = require_owner_workspace(request.workspace)?;
            let container = workspace.container.clone().ok_or_else(|| {
                runtime_error(
                    ScheduleErrorCode::OwnerExecutionContextUnavailable,
                    "the container owner reported no owned instance to execute in",
                )
            })?;
            let capability_execution_target = if request.capabilities.is_some() {
                let bundle_mount = container.bundle_mount.clone().ok_or_else(|| {
                    runtime_error(
                        ScheduleErrorCode::OwnerExecutionContextUnavailable,
                        "the Docker owner provides no verified read-only bundle mount for MCP execution",
                    )
                })?;
                Some(CapabilityExecutionTarget::Docker {
                    instance_name: container.name.clone(),
                    host_bundle_root: bundle_mount.host_root,
                    container_bundle_root: bundle_mount.container_root,
                })
            } else {
                None
            };
            Ok(crate::commands::workflow::OwnerExecutionContext::container(
                container.name,
                container.image_reference,
            )
            .with_capability_execution_target_opt(capability_execution_target))
        }
    }
}

impl ScheduledRunKernel for ScheduledCampaignKernel {
    fn preflight_dispatch(&self, request: &ScheduledDispatch<'_>) -> Result<(), ScheduleError> {
        // Resolved *before* any dispatch intent is written, so a job that cannot
        // be isolated ends as a clean pre-dispatch failure instead of leaving an
        // intent that forces it to be parked as an unprovable effect.
        //
        // The campaign's budget scope is part of that: the run kernel refuses a
        // campaign it cannot resolve, so freezing the scope here — idempotently,
        // and shared by every job of the campaign — keeps a missing agent or an
        // uncreatable scope a pre-dispatch rejection too.
        self.ensure_campaign_scope(request.plan, request.campaign_id)?;
        let owner = owner_execution_context(request)?;
        log::info!(
            "[Headless][scheduler] job {} of campaign {} will execute under the {} owner",
            request.job_id,
            request.campaign_id,
            owner.owner_kind.as_str()
        );
        Ok(())
    }

    fn dispatch(&self, request: &ScheduledDispatch<'_>) -> Result<DispatchedRun, ScheduleError> {
        // Everything the backend can prepare was prepared before the intent was
        // recorded; a fixture problem is still reported as itself here.
        let run_request = Self::run_request(request)?;
        let owner = owner_execution_context(request)?
            .with_capabilities(request.capabilities.cloned())
            .map_err(|message| {
                runtime_error(ScheduleErrorCode::OwnerExecutionContextUnavailable, message)
            })?;

        // The run is created by the same kernel the immediate 2F path uses, with
        // the owner-confirmed instance pinned into its sandbox plan (no Auto/Host
        // resolution), so a scheduled run cannot fall back to host execution.
        let result = self
            .handle
            .block_on(async {
                self.svc
                    .campaign_run_owned(request.campaign_id, run_request, owner)
                    .await
            })
            .map_err(|error| {
                // The kernel refused before any effect: the scheduler turns this
                // into a pre-dispatch failure and never retries blindly.
                runtime_error(
                    ScheduleErrorCode::ExecutorUnavailable,
                    format!("the run kernel refused the dispatch: {}", error.message),
                )
            })?;
        Ok(DispatchedRun {
            run_id: result.session_id.clone(),
            session_id: Some(result.session_id),
        })
    }

    fn run_verdict(&self, run_id: &str) -> Result<RunVerdict, ScheduleError> {
        // The workflow snapshot is the authority; a missing snapshot is
        // "still running", never "finished", and never "succeeded".
        let verdict = self
            .svc
            .main_store
            .get_workflow_snapshot(run_id)
            .ok()
            .map(|snapshot| workflow_status_verdict(&snapshot.workflow.status))
            .unwrap_or(RunVerdict::Running);
        if verdict.is_terminal() {
            // The run owns its verified capabilities only while it is alive, so
            // the in-memory lease is dropped the moment the run is terminal.
            // Idempotent: a run without a lease is left untouched.
            self.svc.release_prepared_lease(run_id);
        }
        Ok(verdict)
    }

    fn stop_run(&self, run_id: &str) -> Result<(), ScheduleError> {
        self.handle
            .block_on(async {
                crate::commands::workflow::workflow_stop_core(&self.svc, run_id.to_string()).await
            })
            .map_err(|error| runtime_error(ScheduleErrorCode::ExecutorUnavailable, error.message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::react::experiment_owner::{PreparedWorkspace, WorkspaceProof};
    use crate::workflow::react::experiment_schedule::types::HarborTaskCapabilityV1;

    /// A real service over a temporary store, assembled exactly like the
    /// headless bootstrap (no window, no Tauri handle).
    fn test_service(dir: &Path) -> Arc<WorkflowApplicationService> {
        use crate::ai::interaction::chat_completion::ChatState;
        use crate::libs::tsid::TsidGenerator;
        use crate::libs::window_channels::WindowChannels;
        use crate::workflow::react::client::hub::{NoWindowTransport, WorkflowRuntimeHub};
        use crate::workflow::react::manager::WorkflowManager;
        use crate::workflow::react::orchestrator::{DefaultSubAgentFactory, SubAgentFactory};

        let store = Arc::new(crate::db::MainStore::new(dir.join("scheduler.db")).expect("store"));
        let chat_state = ChatState::new(Arc::new(WindowChannels::new()), None, store.clone());
        let tsid = Arc::new(TsidGenerator::new(1).expect("tsid"));
        let hub = Arc::new(WorkflowRuntimeHub::with_transport(
            Arc::new(NoWindowTransport),
            "scheduler-test".to_string(),
        ));
        let manager = Arc::new(WorkflowManager::new());
        let factory: Arc<dyn SubAgentFactory> = Arc::new(DefaultSubAgentFactory {
            main_store: store.clone(),
            chat_state: chat_state.clone(),
            gateway: hub.clone(),
            workflow_manager: manager.clone(),
            app_data_dir: dir.to_path_buf(),
            tsid_generator: tsid.clone(),
        });
        Arc::new(WorkflowApplicationService::new(
            store,
            chat_state,
            tsid,
            hub,
            factory,
            manager,
            dir.to_path_buf(),
        ))
    }

    /// The frozen plan the durable scheduler dispatches, including an agent id
    /// no domain registers, so a scope creation attempt is observable.
    fn test_plan() -> CampaignPlanV1 {
        let value = serde_json::json!({
            "schema_version": "campaign_plan.v1",
            "campaign_key": "scope-test",
            "stage": "stage_0_manual",
            "agent_id": "builtin:missing-agent",
            "suite": "chatspeed-smoke",
            "task": "smoke_reply_ok",
            "model": "cs@qwen-3.8-flash",
            "concurrency": 1,
            "budget": {
                "money_mode": { "mode": "token_resource_only" },
                "caps": {
                    "input_tokens": 1024,
                    "output_tokens": 1024,
                    "wall_time_ms": 1000,
                    "tool_calls": 0,
                    "processes": 0,
                    "concurrency": 1
                },
                "required_dimensions": [],
                "max_attempts": 1
            },
            "candidates": [{ "candidate_key": "baseline", "kind": "baseline" }],
            "stop_conditions": { "max_infra_failures": 1, "stop_on_verdict_failure": true }
        });
        crate::workflow::react::campaign::parse_and_validate_campaign_plan(&value).expect("plan")
    }

    /// The campaign scope a durable schedule needs must actually be created.
    /// The scope status lookup returns `Ok(None)` for an *absent* scope, so
    /// treating "the read succeeded" as "the scope exists" left every scheduled
    /// dispatch refused by the run kernel ("campaign not found") after the
    /// dispatch intent had already been recorded. This asserts the creation is
    /// attempted — here it fails loudly because the plan names no registered
    /// agent — instead of silently reporting success.
    #[tokio::test]
    async fn an_absent_campaign_scope_is_created_rather_than_assumed() {
        let directory = tempfile::tempdir().expect("temp dir");
        let svc = test_service(directory.path());
        let kernel = ScheduledCampaignKernel::new(svc.clone(), tokio::runtime::Handle::current());
        let plan = test_plan();
        let campaign_id = crate::workflow::react::campaign::campaign_id_for_plan(&plan.plan_hash());

        let error = kernel
            .ensure_campaign_scope(&plan, &campaign_id)
            .expect_err("an absent scope must not be treated as present");
        assert_eq!(error.code, ScheduleErrorCode::ExecutorUnavailable);
        assert!(
            error.message.contains("budget scope"),
            "the failure must name the scope: {}",
            error.message
        );

        // A durable campaign may only freeze the scope of its own plan.
        let mismatch = kernel
            .ensure_campaign_scope(&plan, "camp-00000000000000000000000000000000")
            .expect_err("a foreign campaign id must be refused");
        assert_eq!(mismatch.code, ScheduleErrorCode::UnknownCampaign);
    }

    /// An already frozen scope is left exactly as it is (idempotent).
    #[tokio::test]
    async fn a_frozen_campaign_scope_is_never_refrozen() {
        let directory = tempfile::tempdir().expect("temp dir");
        let svc = test_service(directory.path());
        let kernel = ScheduledCampaignKernel::new(svc.clone(), tokio::runtime::Handle::current());
        let plan = test_plan();
        let campaign_id = crate::workflow::react::campaign::campaign_id_for_plan(&plan.plan_hash());

        let envelope = plan.envelope().expect("envelope");
        svc.main_store
            .create_campaign_atomic(&campaign_id, envelope, 1_700_000_000_000)
            .expect("freeze the campaign scope");

        kernel
            .ensure_campaign_scope(&plan, &campaign_id)
            .expect("a frozen scope is left untouched");
        let scope = svc
            .main_store
            .get_budget_scope_status(&campaign_id)
            .expect("read scope")
            .expect("scope exists");
        assert_eq!(scope.scope_id, campaign_id);
    }

    /// A scheduled dispatch fails closed when the owner context is missing, so
    /// it can never degrade to host execution.
    #[test]
    fn a_missing_owner_context_fails_closed() {
        let error = require_owner_workspace(None).expect_err("no owner context");
        assert_eq!(
            error.code,
            ScheduleErrorCode::OwnerExecutionContextUnavailable
        );
        assert_eq!(
            error.code.as_str(),
            "owner_execution_context_unavailable",
            "the operator must see a stable machine code"
        );

        let workspace = PreparedWorkspace {
            proof: WorkspaceProof {
                job_id: "job-1".to_string(),
                owner_token_hash: "a".repeat(64),
                base_revision: "HEAD".to_string(),
                workspace_root: PathBuf::from("owned-workspace"),
                worktrees_root: PathBuf::from("owned-root"),
            },
            input_patch_applied: false,
            container: None,
        };
        let accepted = require_owner_workspace(Some(&workspace)).expect("owner context");
        assert_eq!(accepted.proof.job_id, "job-1");
    }

    /// The real production mapping: a live run must never look finished, and a
    /// failed or cancelled run must never look successful.
    #[test]
    fn the_workflow_status_verdict_separates_terminality_from_success() {
        // Non-terminal statuses stay "running" so the job remains durable.
        for status in [
            "pending",
            "thinking",
            "executing",
            "auditing",
            "stopping",
            "paused",
            "awaiting_user",
            "awaiting_approval",
            "awaiting_auto_approval",
            "awaiting_sub_agent",
            "some_future_status",
        ] {
            let verdict = workflow_status_verdict(status);
            assert_eq!(verdict, RunVerdict::Running, "status {status}");
            assert!(
                !verdict.is_terminal(),
                "status {status} must not be terminal"
            );
            assert_eq!(verdict.terminal_flag(), None, "status {status}");
        }

        // Only a completed run is a success.
        assert_eq!(workflow_status_verdict("completed"), RunVerdict::Succeeded);
        assert!(workflow_status_verdict("completed").is_terminal());
        assert_eq!(
            workflow_status_verdict("completed").terminal_flag(),
            Some(true)
        );

        // Terminal failures are terminal but never successful.
        for status in ["error", "cancelled", "failed"] {
            assert_eq!(
                workflow_status_verdict(status),
                RunVerdict::Failed,
                "status {status}"
            );
            assert!(
                workflow_status_verdict(status).is_terminal(),
                "status {status}"
            );
        }
    }

    /// The adapter's capability manifest is the one this runtime reads.
    ///
    /// The two sides live in different languages and cannot share a constant, so
    /// the hand-off is a *path contract*: the adapter writes the manifest inside
    /// the experiment domain it starts (`<data-dir>/runtime/…`), and this runtime
    /// resolves exactly that. A mismatch makes every Harbor-owner job fail closed
    /// with `ownership_mismatch`, which is why the contract is asserted here
    /// against the real producer (`tools/harbor/artifact_contract.py`) rather
    /// than against a hand-written fixture.
    #[test]
    fn the_harbor_adapter_writes_the_capability_this_runtime_reads() {
        use std::process::Command;

        let contract = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("tools")
            .join("harbor")
            .join("artifact_contract.py");
        if !contract.exists() {
            eprintln!("skipping: {} is not present", contract.display());
            return;
        }
        let python = std::env::var("PYTHON").unwrap_or_else(|_| "python3".to_string());
        let layout = Command::new(&python).arg(&contract).arg("paths").output();
        let Ok(layout) = layout else {
            eprintln!("skipping: {python} is unavailable");
            return;
        };
        assert!(
            layout.status.success(),
            "the artifact contract must print its layout: {}",
            String::from_utf8_lossy(&layout.stderr)
        );
        let layout: serde_json::Value =
            serde_json::from_slice(&layout.stdout).expect("the layout is JSON");
        let adapter_domain = layout["domain_root"].as_str().expect("domain_root");
        let adapter_capability = layout["capability_file"].as_str().expect("capability_file");
        assert_eq!(
            Path::new(adapter_domain)
                .join("runtime")
                .join(HARBOR_CAPABILITY_FILE_NAME),
            Path::new(adapter_capability),
            "the adapter must write the capability inside the domain it starts"
        );
        // …and the runtime must resolve that same relative layout for a domain.
        let domain = tempfile::tempdir().expect("temp dir");
        let resolved = harbor_capability_path(domain.path());
        assert_eq!(
            resolved
                .strip_prefix(domain.path())
                .expect("inside the domain"),
            Path::new("runtime").join(HARBOR_CAPABILITY_FILE_NAME),
            "the runtime must read `<data-dir>/runtime/{}`",
            HARBOR_CAPABILITY_FILE_NAME
        );
        assert_eq!(
            DomainSchedulerResources::new(domain.path(), None)
                .harbor_capability
                .as_path(),
            resolved.as_path(),
            "resource resolution must use the same contract"
        );

        // The produced manifest is loadable at the resolved path, and the token
        // the runtime checks against is the one the adapter derived.
        let emitted = Command::new(&python)
            .arg(&contract)
            .arg("emit")
            .arg("trial-handoff")
            .arg("nonce-handoff")
            .output()
            .expect("emit the manifest");
        assert!(
            emitted.status.success(),
            "the adapter must emit a manifest: {}",
            String::from_utf8_lossy(&emitted.stderr)
        );
        let manifest: serde_json::Value =
            serde_json::from_slice(&emitted.stdout).expect("the manifest is JSON");
        std::fs::create_dir_all(resolved.parent().expect("runtime dir")).expect("runtime dir");
        std::fs::write(&resolved, &emitted.stdout).expect("write the manifest");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&resolved, std::fs::Permissions::from_mode(0o600))
                .expect("restrict the manifest");
        }

        let owner = HarborTaskOwner::load(&resolved).expect("the runtime must accept it");
        assert_eq!(
            owner.capability_path(),
            resolved.as_path(),
            "the loaded owner must be bound to the resolved path"
        );
        let loaded: HarborTaskCapabilityV1 =
            serde_json::from_slice(&std::fs::read(&resolved).expect("read the manifest"))
                .expect("manifest");
        assert_eq!(
            loaded.owner_token_hash,
            manifest["owner_token_hash"].as_str().expect("token"),
            "the runtime must fence on the adapter-derived token"
        );
        assert_eq!(loaded.task_id, "trial-handoff");
        assert_eq!(loaded.nonce, "nonce-handoff");
    }

    /// The declared roots are the ones the task image must provide.
    ///
    /// `HarborTaskOwner::preflight` refuses a capability whose declared roots do
    /// not exist, so the adapter and the task environment have to agree on the
    /// same list; this asserts the list the adapter declares is exactly what the
    /// documented task image provisions.
    #[test]
    fn the_adapter_declares_the_roots_the_task_image_provides() {
        use std::process::Command;

        let contract = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("tools")
            .join("harbor")
            .join("artifact_contract.py");
        let python = std::env::var("PYTHON").unwrap_or_else(|_| "python3".to_string());
        let Ok(layout) = Command::new(&python).arg(&contract).arg("paths").output() else {
            eprintln!("skipping: {python} is unavailable");
            return;
        };
        if !layout.status.success() {
            eprintln!("skipping: the artifact contract is not importable");
            return;
        }
        let layout: serde_json::Value =
            serde_json::from_slice(&layout.stdout).expect("the layout is JSON");
        let roots: Vec<String> = std::iter::once(layout["task_root"].as_str())
            .chain(std::iter::once(layout["workspace_root"].as_str()))
            .chain(std::iter::once(layout["artifact_root"].as_str()))
            .chain(
                layout["read_only_roots"]
                    .as_array()
                    .expect("read_only_roots")
                    .iter()
                    .map(|root| root.as_str()),
            )
            .map(|root| root.expect("root").to_string())
            .collect();
        assert!(
            roots.contains(&"/workspace".to_string()),
            "the workspace root must be declared: {roots:?}"
        );
        assert!(
            roots.contains(&"/logs/artifacts".to_string()),
            "the artifact root must be declared: {roots:?}"
        );
        // The documented task image (work/agent-cli-harbor-smoke/environment)
        // provisions exactly these roots.
        let dockerfile = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("work")
            .join("agent-cli-harbor-smoke")
            .join("environment")
            .join("Dockerfile");
        let body = std::fs::read_to_string(&dockerfile)
            .unwrap_or_else(|error| panic!("{}: {error}", dockerfile.display()));
        for root in &roots {
            assert!(
                body.contains(root.as_str()),
                "the task image must provision the declared root '{root}'"
            );
        }
    }
}
