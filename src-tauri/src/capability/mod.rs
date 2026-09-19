//! Transport-neutral capability management for Agent Skills and MCP servers.
//!
//! This module is the single canonical path for every capability mutation: the
//! Tauri commands, the `/control/v1` HTTP plane and the `cs` CLI all delegate
//! here (AC-1). It owns no runtime of its own — `MainStore`, `ConfigCache`,
//! `ToolManager` and MCP child processes stay with the desktop main process
//! (INV-1) — and it never opens a database connection outside the shared
//! `MainStore`.
//!
//! Structure:
//!
//! - [`repository`] is the durable journal (operations, effects, ownership);
//! - [`operation`] provides canonical hashing, stable ids and resource locks;
//! - [`redaction`] guarantees no secret reaches a log, DTO or journal row;
//! - [`error`] is the stable machine-readable error contract shared by all
//!   adapters;
//! - [`targets`] is the closed Skill install-target registry;
//! - [`skill_inventory`] classifies installed Skills without mutating them;
//! - [`mcp_service`] projects desired/runtime/tools state for MCP servers;
//! - [`doctor`] reports journal, ownership, runtime and staging drift.

pub mod doctor;
pub mod error;
pub mod mcp;
pub mod mcp_service;
pub mod operation;
pub mod reconcile;
pub mod redaction;
pub mod repository;
pub mod skill;
pub mod skill_inventory;
pub mod targets;
pub mod types;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::db::MainStore;

use error::{code, CapabilityError};
use mcp::runtime::{McpRuntimePort, UnavailableRuntimePort};
use mcp_service::{project_mcp_servers, McpServerView};
use operation::ResourceLocks;
use repository::CapabilityRepository;
use skill_inventory::{SkillInventory, SkillInventoryService};
use targets::{resolve_targets, ResolvedSkillTarget, TargetEnvironment};
use types::{
    CapabilityOperation as OperationRecord, EffectOutcome, OperationBegin, OperationRequest,
    OperationState,
};

pub use types::{CapabilityKind, LOCAL_ACTOR_SCOPE};

/// The private capability directory under app data.
///
/// It holds only short-lived staging and quarantine content, plus diagnostics.
/// It is never the journal authority (AC-2).
pub fn capability_private_root(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("capability")
}

/// The private staging directory a mutation stages content into before commit.
pub fn staging_dir(app_data_dir: &Path) -> PathBuf {
    capability_private_root(app_data_dir).join("staging")
}

/// The quarantine directory an uninstall moves content into before finalize.
pub fn quarantine_dir(app_data_dir: &Path) -> PathBuf {
    capability_private_root(app_data_dir).join("quarantine")
}

/// What startup recovery did with the operations left in flight by a crash.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct CapabilityRecoveryReport {
    /// Operations interrupted before any effect: safe to retry.
    pub failed_before_effect: Vec<String>,
    /// Operations interrupted with an unproven effect: doctor must classify.
    pub needs_reconcile: Vec<String>,
}

impl CapabilityRecoveryReport {
    pub fn is_empty(&self) -> bool {
        self.failed_before_effect.is_empty() && self.needs_reconcile.is_empty()
    }
}

/// The single application service every capability adapter delegates to.
pub struct CapabilityApplicationService {
    repository: CapabilityRepository,
    locks: ResourceLocks,
    app_data_dir: PathBuf,
    environment: TargetEnvironment,
    runtime: Arc<dyn McpRuntimePort>,
    mcp_repository: Arc<dyn mcp::repository::McpRepositoryPort>,
    mcp_effects: Arc<dyn mcp::runtime::McpRuntimeEffects>,
    mcp_timing: mcp::orchestrator::McpTiming,
}

impl CapabilityApplicationService {
    /// Creates the service over the shared desktop store.
    ///
    /// `app_data_dir` is used only for short-lived private staging, quarantine
    /// and diagnostics; it is never the journal authority.
    pub fn new(store: Arc<MainStore>, app_data_dir: PathBuf) -> Self {
        Self {
            repository: CapabilityRepository::new(store.clone()),
            locks: ResourceLocks::new(),
            app_data_dir,
            environment: TargetEnvironment::detect(),
            runtime: Arc::new(UnavailableRuntimePort),
            mcp_repository: Arc::new(mcp::repository::MainStoreMcpRepository::new(store)),
            mcp_effects: Arc::new(mcp::runtime::UnavailableRuntimeEffects),
            mcp_timing: mcp::orchestrator::McpTiming::default(),
        }
    }

    /// Wires the MCP runtime observation port.
    ///
    /// Without it the service still reports durable desired state, but runtime
    /// state is reported as unobserved rather than guessed (INV-7).
    pub fn with_runtime(mut self, runtime: Arc<dyn McpRuntimePort>) -> Self {
        self.runtime = runtime;
        self
    }

    /// Wires the MCP runtime effect port, which only the process that owns
    /// `ToolManager` may provide (INV-1).
    ///
    /// Observation and effects are wired together so a process can never start a
    /// server it cannot then observe.
    pub fn with_mcp_runtime(
        mut self,
        runtime: Arc<dyn McpRuntimePort>,
        effects: Arc<dyn mcp::runtime::McpRuntimeEffects>,
    ) -> Self {
        self.runtime = runtime;
        self.mcp_effects = effects;
        self
    }

    /// Replaces the MCP persistence port (test seam for a deterministic store).
    pub fn with_mcp_repository(
        mut self,
        repository: Arc<dyn mcp::repository::McpRepositoryPort>,
    ) -> Self {
        self.mcp_repository = repository;
        self
    }

    /// Replaces the bounded runtime wait times (test seam).
    pub fn with_mcp_timing(mut self, timing: mcp::orchestrator::McpTiming) -> Self {
        self.mcp_timing = timing;
        self
    }

    /// The MCP desired-state persistence port.
    pub fn mcp_repository(&self) -> &Arc<dyn mcp::repository::McpRepositoryPort> {
        &self.mcp_repository
    }

    /// The MCP runtime effect port.
    pub fn mcp_effects(&self) -> &Arc<dyn mcp::runtime::McpRuntimeEffects> {
        &self.mcp_effects
    }

    /// The bounded runtime wait policy.
    pub fn mcp_timing(&self) -> mcp::orchestrator::McpTiming {
        self.mcp_timing
    }

    /// Injects the target environment (used by isolated tests and hosted runs).
    pub fn with_environment(mut self, environment: TargetEnvironment) -> Self {
        self.environment = environment;
        self
    }

    /// The durable journal, used by the domain services and doctor.
    pub fn repository(&self) -> &CapabilityRepository {
        &self.repository
    }

    /// The private app-data directory for staging and quarantine.
    pub fn app_data_dir(&self) -> &Path {
        &self.app_data_dir
    }

    /// The target environment Skill targets resolve against.
    pub fn environment(&self) -> &TargetEnvironment {
        &self.environment
    }

    /// Per-resource serialization for in-process single-flight.
    pub fn locks(&self) -> &ResourceLocks {
        &self.locks
    }

    // ------------------------------------------------------------------- reads

    /// Every registered Skill target, resolved for this environment.
    pub fn skill_targets(&self) -> Vec<ResolvedSkillTarget> {
        resolve_targets(&self.environment)
    }

    /// The Agent Skill inventory, built from the shared scanner + journal.
    pub fn skill_inventory(&self) -> Result<SkillInventory, CapabilityError> {
        SkillInventoryService::new(self.app_data_dir.clone(), self.environment.clone())
            .build(&self.repository)
    }

    /// The MCP read projection using the last runtime observation.
    ///
    /// A runtime failure degrades to `observed: false` instead of failing the
    /// read, because the durable desired state is still valid information.
    pub async fn mcp_servers(&self) -> Result<Vec<McpServerView>, CapabilityError> {
        let servers = self.repository.store().config.get_mcps();
        let observation = match tokio::time::timeout(
            self.mcp_timing.status_timeout,
            self.runtime.observed_runtime(),
        )
        .await
        {
            Ok(Ok(observation)) => Some(observation),
            Ok(Err(error)) => {
                log::debug!(
                    "[Capability][mcp] runtime observation unavailable: {}",
                    error.redacted_message()
                );
                None
            }
            Err(_) => {
                log::debug!("[Capability][mcp] runtime observation timed out");
                None
            }
        };
        Ok(project_mcp_servers(&servers, observation.as_ref()))
    }

    /// One MCP server by name.
    pub async fn mcp_server(&self, name: &str) -> Result<McpServerView, CapabilityError> {
        let servers = self.mcp_servers().await?;
        servers
            .into_iter()
            .find(|view| view.name == name)
            .ok_or_else(|| CapabilityError::not_found(format!("MCP server '{name}' not found")))
    }

    /// The capability doctor report: journal, ownership, runtime and staging.
    pub async fn doctor(&self) -> Result<doctor::CapabilityDoctorReport, CapabilityError> {
        let inventory = self.skill_inventory()?;
        let mcp_servers = self.mcp_servers().await?;
        doctor::build_doctor_report(
            &self.repository,
            &inventory,
            &mcp_servers,
            &self.app_data_dir,
        )
    }

    // -------------------------------------------------------------- operations

    /// Opens (or replays) one operation, serialized per resource key.
    ///
    /// The lock is held only for the durable `begin`, not for the whole
    /// mutation: a long external effect must never block the journal.
    pub async fn begin_operation(
        &self,
        request: OperationRequest,
    ) -> Result<OperationBegin, CapabilityError> {
        let lock_key = format!("{}:{}", request.capability.as_str(), request.resource_key);
        let _guard = self.locks.lock(&lock_key).await;
        self.repository.begin(&request)
    }

    /// Loads one operation by id.
    pub fn operation(&self, operation_id: &str) -> Result<OperationRecord, CapabilityError> {
        self.repository.get(operation_id)?.ok_or_else(|| {
            CapabilityError::new(
                code::OPERATION_NOT_FOUND,
                format!("capability operation '{operation_id}' does not exist"),
            )
        })
    }

    /// Advances an operation into a new phase.
    pub fn set_state(
        &self,
        operation_id: &str,
        state: OperationState,
        phase: Option<&str>,
    ) -> Result<OperationRecord, CapabilityError> {
        self.repository.set_state(operation_id, state, phase)
    }

    /// Records the terminal state of an operation.
    pub fn finish_operation(
        &self,
        operation_id: &str,
        state: OperationState,
        result: Option<&serde_json::Value>,
        error: Option<&CapabilityError>,
    ) -> Result<OperationRecord, CapabilityError> {
        self.repository.finish(operation_id, state, result, error)
    }

    /// Records the intent of an external effect before performing it.
    pub fn record_effect_intent(
        &self,
        operation_id: &str,
        effect_key: &str,
        target: Option<&str>,
        detail: Option<&serde_json::Value>,
    ) -> Result<(), CapabilityError> {
        self.repository
            .record_effect_intent(operation_id, effect_key, target, detail)
            .map(|_| ())
    }

    /// Records the observation of an external effect after performing it.
    pub fn record_effect_outcome(
        &self,
        operation_id: &str,
        effect_key: &str,
        outcome: EffectOutcome,
        detail: Option<&serde_json::Value>,
    ) -> Result<(), CapabilityError> {
        self.repository
            .record_effect_outcome(operation_id, effect_key, outcome, detail)
            .map(|_| ())
    }

    /// Classifies every operation a crash left in flight.
    ///
    /// Called once at startup, before any mutation is admitted. An operation
    /// interrupted before its first effect is failed (retryable); one with an
    /// unproven effect is moved to `needs_reconcile` and is never blindly
    /// retried (INV-8).
    pub fn recover_interrupted_operations(
        &self,
    ) -> Result<CapabilityRecoveryReport, CapabilityError> {
        let interrupted = self.repository.list_interrupted()?;
        let mut report = CapabilityRecoveryReport::default();

        for operation in interrupted {
            let unproven = self.repository.count_unproven_effects(&operation.operation_id)?;
            if unproven == 0 {
                let error = CapabilityError::new(
                    code::INTERRUPTED_BEFORE_EFFECT,
                    "operation was interrupted before any effect was attempted",
                );
                self.repository.finish(
                    &operation.operation_id,
                    OperationState::Failed,
                    None,
                    Some(&error),
                )?;
                report.failed_before_effect.push(operation.operation_id);
            } else {
                let error = CapabilityError::new(
                    code::EFFECT_STATE_UNKNOWN,
                    format!("{unproven} effect(s) had an unproven outcome after an interruption"),
                );
                self.repository.mark_needs_reconcile(
                    &operation.operation_id,
                    "interrupted with an unproven effect outcome",
                    Some(&error),
                )?;
                report.needs_reconcile.push(operation.operation_id);
            }
        }

        if !report.is_empty() {
            log::info!(
                "[Capability][recovery] classified {} interrupted operation(s): {} retryable, {} need reconcile",
                report.failed_before_effect.len() + report.needs_reconcile.len(),
                report.failed_before_effect.len(),
                report.needs_reconcile.len()
            );
        }

        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn service(app_data_dir: PathBuf) -> CapabilityApplicationService {
        let store = Arc::new(MainStore::new(":memory:").expect("in-memory store"));
        CapabilityApplicationService::new(store, app_data_dir)
    }

    fn request(key: &str) -> OperationRequest {
        OperationRequest {
            capability: CapabilityKind::Skill,
            operation_kind: "skill.install".to_string(),
            actor_scope: LOCAL_ACTOR_SCOPE.to_string(),
            idempotency_key: key.to_string(),
            request: json!({ "name": "demo" }),
            resource_key: "skill:demo".to_string(),
        }
    }

    #[tokio::test]
    async fn recovery_fails_operations_interrupted_before_any_effect() {
        let temp = TempDir::new().expect("temp dir");
        let service = service(temp.path().to_path_buf());
        let operation = service
            .begin_operation(request("recovery-1"))
            .await
            .expect("begin")
            .into_operation();

        let report = service
            .recover_interrupted_operations()
            .expect("recovery must succeed");
        assert_eq!(
            report.failed_before_effect,
            vec![operation.operation_id.clone()]
        );
        assert!(report.needs_reconcile.is_empty());

        let stored = service.operation(&operation.operation_id).expect("stored");
        assert_eq!(stored.state, OperationState::Failed);
        assert_eq!(
            stored.error_code.as_deref(),
            Some(code::INTERRUPTED_BEFORE_EFFECT)
        );
    }

    #[tokio::test]
    async fn recovery_requires_reconcile_when_an_effect_is_unproven() {
        let temp = TempDir::new().expect("temp dir");
        let service = service(temp.path().to_path_buf());
        let operation = service
            .begin_operation(request("recovery-2"))
            .await
            .expect("begin")
            .into_operation();

        service
            .record_effect_intent(
                &operation.operation_id,
                "target:chatspeed",
                Some("chatspeed"),
                None,
            )
            .expect("intent");

        let report = service
            .recover_interrupted_operations()
            .expect("recovery must succeed");
        assert!(report.failed_before_effect.is_empty());
        assert_eq!(report.needs_reconcile, vec![operation.operation_id.clone()]);

        let stored = service.operation(&operation.operation_id).expect("stored");
        assert_eq!(stored.state, OperationState::NeedsReconcile);
        assert!(stored.reconcile_reason.is_some());
    }

    #[tokio::test]
    async fn recovery_leaves_completed_operations_alone() {
        let temp = TempDir::new().expect("temp dir");
        let service = service(temp.path().to_path_buf());
        let operation = service
            .begin_operation(request("recovery-3"))
            .await
            .expect("begin")
            .into_operation();
        service
            .finish_operation(
                &operation.operation_id,
                OperationState::Completed,
                Some(&json!({ "installed": true })),
                None,
            )
            .expect("finish");

        let report = service
            .recover_interrupted_operations()
            .expect("recovery must succeed");
        assert!(report.is_empty());
        assert_eq!(
            service
                .operation(&operation.operation_id)
                .expect("stored")
                .state,
            OperationState::Completed
        );
    }

    #[tokio::test]
    async fn a_replayed_operation_never_starts_a_second_attempt() {
        let temp = TempDir::new().expect("temp dir");
        let service = service(temp.path().to_path_buf());
        let first = service
            .begin_operation(request("replay-1"))
            .await
            .expect("first begin");
        assert!(!first.is_replay());

        let second = service
            .begin_operation(request("replay-1"))
            .await
            .expect("second begin");
        assert!(second.is_replay());
        assert_eq!(
            first.operation().operation_id,
            second.operation().operation_id
        );
    }

    #[tokio::test]
    async fn the_read_only_surfaces_agree_with_the_registry_and_the_empty_journal() {
        let temp = TempDir::new().expect("temp dir");
        let service = service(temp.path().to_path_buf());

        let targets = service.skill_targets();
        assert_eq!(
            targets.len(),
            crate::capability::targets::SkillTargetId::ALL.len()
        );
        assert_eq!(
            targets
                .iter()
                .filter(|target| target.default_selected)
                .count(),
            1
        );

        // The default scanner also reads the real HOME and the bundled
        // resources, so the durable invariant under test is that an empty
        // journal never claims ownership of anything.
        let inventory = service.skill_inventory().expect("inventory");
        assert!(
            inventory.skills.iter().all(|entry| !entry.managed),
            "an empty journal must not claim any ownership"
        );

        let servers = service.mcp_servers().await.expect("mcp servers");
        assert!(servers.is_empty());

        let report = service.doctor().await.expect("doctor report");
        assert!(
            report.is_clean(),
            "unexpected findings: {:?}",
            report.findings
        );
    }
}
