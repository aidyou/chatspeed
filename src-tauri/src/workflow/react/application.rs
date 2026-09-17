//! Transport-neutral workflow application service.
//!
//! `WorkflowApplicationService` is the single canonical path for workflow
//! application operations (agent queries, workflow list/create/snapshot/
//! start/signal/stop/events). The Tauri command wrappers, the workflow
//! automation scheduler and the loopback HTTP control plane all delegate to
//! this service; none of them may copy or bypass its orchestration.
//!
//! The orchestration cores currently live in `commands/workflow.rs` as
//! transport-neutral `*_core` functions that take `&WorkflowApplicationService`.
//! They are `pub(crate)` implementation details of this service and must not be
//! called directly by any transport adapter.
//!
//! The service produces stable [`ApplicationError`] domain errors. Each
//! transport adapter maps them to its own wire representation (Tauri keeps the
//! existing string errors; HTTP maps to status/code; the CLI maps to exit
//! codes).

use crate::ai::interaction::chat_completion::ChatState;
use crate::commands::workflow::{
    campaign_cancel_core, campaign_close_core, campaign_create_core, campaign_get_core,
    campaign_job_core, campaign_jobs_core, campaign_reconcile_core, campaign_run_core,
    campaign_run_core_with_owner, campaign_schedule_core, create_workflow_core,
    get_workflow_events_core, get_workflow_snapshot_core, list_workflows_core, run_experiment_core,
    workflow_signal_core, workflow_start_core, workflow_stop_core,
};
use crate::db::{Agent, MainStore, Workflow};
use crate::libs::tsid::TsidGenerator;
use crate::workflow::react::client::hub::WorkflowRuntimeHub;
use crate::workflow::react::events::WorkflowEventRecord;
use crate::workflow::react::experiment_owner::capabilities::PreparedCapabilityLease;
use crate::workflow::react::manager::WorkflowManager;
use crate::workflow::react::orchestrator::SubAgentFactory;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

/// Stable error classification for workflow application operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplicationErrorKind {
    /// A referenced entity does not exist.
    NotFound,
    /// The request is malformed or references an unusable entity.
    InvalidInput,
    /// The request conflicts with current state (e.g. duplicate idempotency
    /// key with a different body).
    Conflict,
    /// The operation is not valid for the current workflow state.
    State,
    /// The runtime gateway rejected the operation (e.g. no live input route).
    Gateway,
    /// Any other failure (storage, tooling, unexpected errors).
    Internal,
}

/// A workflow application error with a stable kind and a human-readable
/// message. `Display` renders only the message so existing Tauri string errors
/// stay byte-identical.
#[derive(Debug, Clone)]
pub struct ApplicationError {
    pub kind: ApplicationErrorKind,
    pub message: String,
}

impl ApplicationError {
    pub fn new(kind: ApplicationErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ApplicationErrorKind::NotFound, message)
    }

    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::new(ApplicationErrorKind::InvalidInput, message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(ApplicationErrorKind::Conflict, message)
    }

    pub fn state(message: impl Into<String>) -> Self {
        Self::new(ApplicationErrorKind::State, message)
    }

    pub fn gateway(message: impl Into<String>) -> Self {
        Self::new(ApplicationErrorKind::Gateway, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ApplicationErrorKind::Internal, message)
    }
}

impl std::fmt::Display for ApplicationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<String> for ApplicationError {
    fn from(message: String) -> Self {
        Self::internal(message)
    }
}

impl From<&str> for ApplicationError {
    fn from(message: &str) -> Self {
        Self::internal(message)
    }
}

/// Transport-neutral workflow creation request (HTTP canonical snake_case).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkflowCreateRequest {
    pub user_query: Option<String>,
    pub agent_id: String,
    pub allowed_paths: Option<serde_json::Value>,
    pub auto_approve_plan: Option<bool>,
    pub final_audit: Option<bool>,
    pub inherited_agent_config: Option<String>,
}

/// Transport-neutral workflow start request (HTTP canonical snake_case).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkflowStartRequest {
    pub session_id: String,
    pub agent_id: String,
    pub initial_prompt: Option<String>,
    pub initial_metadata: Option<serde_json::Value>,
    pub initial_attached_context: Option<String>,
    pub planning_mode: Option<bool>,
}

/// Explicitly bounded durable-events query. `after` is a durable DB event ID
/// (never a live stream cursor); `limit` is capped by the store.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkflowEventsQuery {
    pub session_id: String,
    pub after: Option<i64>,
    pub limit: Option<u32>,
}

/// The unique transport-neutral workflow application service.
pub struct WorkflowApplicationService {
    pub(crate) main_store: Arc<MainStore>,
    pub(crate) chat_state: Arc<ChatState>,
    pub(crate) tsid_generator: Arc<TsidGenerator>,
    pub(crate) gateway: Arc<WorkflowRuntimeHub>,
    pub(crate) factory: Arc<dyn SubAgentFactory>,
    pub(crate) workflow_manager: Arc<WorkflowManager>,
    pub(crate) app_data_dir: PathBuf,
    /// Run-scoped verified capability leases, keyed by session id.
    ///
    /// A lease's resolved secret values must never be persisted (INV-6), so it
    /// travels in memory only: the durable scheduler registers it for the run it
    /// just dispatched, the run's executor injects it into that session's tool
    /// registry, and the terminal/failure path unregisters it again.
    pub(crate) prepared_leases:
        std::sync::Mutex<std::collections::HashMap<String, PreparedCapabilityLease>>,
}

impl WorkflowApplicationService {
    pub fn new(
        main_store: Arc<MainStore>,
        chat_state: Arc<ChatState>,
        tsid_generator: Arc<TsidGenerator>,
        gateway: Arc<WorkflowRuntimeHub>,
        factory: Arc<dyn SubAgentFactory>,
        workflow_manager: Arc<WorkflowManager>,
        app_data_dir: PathBuf,
    ) -> Self {
        Self {
            main_store,
            chat_state,
            tsid_generator,
            gateway,
            factory,
            workflow_manager,
            app_data_dir,
            prepared_leases: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Registers the verified capability lease of one dispatched run.
    ///
    /// Registering an already-registered session replaces the lease, so a
    /// re-dispatch of the same job cannot accumulate capabilities.
    pub(crate) fn register_prepared_lease(&self, session_id: &str, lease: PreparedCapabilityLease) {
        if let Ok(mut leases) = self.prepared_leases.lock() {
            leases.insert(session_id.to_string(), lease);
        }
    }

    /// The verified capability lease of one session, when the durable scheduler
    /// registered one for the run it dispatched.
    pub(crate) fn prepared_lease(&self, session_id: &str) -> Option<PreparedCapabilityLease> {
        self.prepared_leases
            .lock()
            .ok()
            .and_then(|leases| leases.get(session_id).cloned())
    }

    /// Drops the session's capability lease. Idempotent: a session without a
    /// lease is left untouched.
    pub(crate) fn release_prepared_lease(
        &self,
        session_id: &str,
    ) -> Option<PreparedCapabilityLease> {
        self.prepared_leases
            .lock()
            .ok()
            .and_then(|mut leases| leases.remove(session_id))
    }

    /// Lists all agents from the same `MainStore` authority the UI uses.
    pub async fn agent_list(&self) -> Result<Vec<Agent>, ApplicationError> {
        let runtime = self.main_store.db_runtime().map_err(|e| e.to_string())?;
        MainStore::get_all_agents_with_runtime(runtime)
            .await
            .map_err(|e| ApplicationError::internal(e.to_string()))
    }

    /// Gets one agent by stable ID from the same `MainStore` authority the UI
    /// uses. Returns `Ok(None)` when the agent does not exist.
    pub async fn agent_get(&self, agent_id: &str) -> Result<Option<Agent>, ApplicationError> {
        let runtime = self.main_store.db_runtime().map_err(|e| e.to_string())?;
        MainStore::get_agent_with_runtime(runtime, agent_id.to_string())
            .await
            .map_err(|e| ApplicationError::internal(e.to_string()))
    }

    /// Lists workflows from the durable store.
    pub async fn workflow_list(&self) -> Result<Vec<Workflow>, ApplicationError> {
        list_workflows_core(self).await
    }

    /// Creates a normal workflow session (same orchestration as the Tauri
    /// `create_workflow` command).
    pub async fn create_workflow(
        &self,
        request: WorkflowCreateRequest,
    ) -> Result<String, ApplicationError> {
        create_workflow_core(self, request).await
    }

    /// Returns the authoritative workflow snapshot.
    pub async fn workflow_snapshot(
        &self,
        session_id: &str,
    ) -> Result<serde_json::Value, ApplicationError> {
        get_workflow_snapshot_core(self, session_id.to_string()).await
    }

    /// Starts (or resumes) a workflow session.
    pub async fn workflow_start(
        &self,
        request: WorkflowStartRequest,
    ) -> Result<String, ApplicationError> {
        workflow_start_core(
            self,
            request.session_id,
            request.agent_id,
            request.initial_prompt,
            request.initial_metadata,
            request.initial_attached_context,
            request.planning_mode,
        )
        .await
    }

    /// Submits a typed signal to a workflow session.
    pub async fn workflow_signal(
        &self,
        session_id: &str,
        signal: String,
    ) -> Result<String, ApplicationError> {
        workflow_signal_core(self, session_id.to_string(), signal).await
    }

    /// Stops a workflow session (active, waiting or retrying).
    pub async fn workflow_stop(&self, session_id: &str) -> Result<(), ApplicationError> {
        workflow_stop_core(self, session_id.to_string()).await
    }

    /// Queries durable workflow events with an explicit bound.
    pub async fn workflow_events(
        &self,
        query: WorkflowEventsQuery,
    ) -> Result<Vec<WorkflowEventRecord>, ApplicationError> {
        get_workflow_events_core(self, query).await
    }

    /// Runs one budgeted, single-attempt experiment workflow. This is the
    /// single backend-owned facade for Phase 2C: it validates the strict spec,
    /// atomically creates the workflow plus its four-level budget scope chain,
    /// and reuses the existing start kernel. The CLI and HTTP control plane
    /// both delegate here; neither opens the database nor runs a second
    /// executor (INV-1).
    pub async fn experiment_run(
        &self,
        request: crate::workflow::react::experiment::ExperimentRunRequest,
    ) -> Result<crate::workflow::react::experiment::ExperimentRunResult, ApplicationError> {
        run_experiment_core(self, request).await
    }

    /// Creates the shared Phase 2F campaign budget scope for one frozen plan.
    /// The campaign id is derived by the backend from the plan hash; the caller
    /// never supplies a scope id (INV-2).
    pub async fn campaign_create(
        &self,
        plan: crate::workflow::react::campaign::CampaignPlanV1,
    ) -> Result<crate::workflow::react::campaign::CampaignCreateResult, ApplicationError> {
        campaign_create_core(self, plan)
    }

    /// Reads one campaign projection (frozen scope plus its candidate scopes).
    pub async fn campaign_get(
        &self,
        campaign_id: &str,
    ) -> Result<crate::workflow::react::campaign::CampaignProjection, ApplicationError> {
        campaign_get_core(self, campaign_id)
    }

    /// Creates one run under an existing shared campaign scope, reusing the
    /// same run kernel and admission path as the 2C experiment facade.
    pub async fn campaign_run(
        &self,
        campaign_id: &str,
        request: crate::workflow::react::campaign::CampaignRunRequestV1,
    ) -> Result<crate::workflow::react::campaign::CampaignRunResult, ApplicationError> {
        campaign_run_core(self, campaign_id, request).await
    }

    /// The durable scheduler's entry point into the same run kernel.
    ///
    /// The only difference from [`Self::campaign_run`] is that the created run is
    /// pinned to the owner-confirmed execution context the scheduler prepared, so
    /// the run cannot resolve its shell execution environment through the
    /// host-capable path (AC-3/INV-4).
    pub(crate) async fn campaign_run_owned(
        &self,
        campaign_id: &str,
        request: crate::workflow::react::campaign::CampaignRunRequestV1,
        owner: crate::commands::workflow::OwnerExecutionContext,
    ) -> Result<crate::workflow::react::campaign::CampaignRunResult, ApplicationError> {
        campaign_run_core_with_owner(self, campaign_id, request, Some(owner)).await
    }

    /// Closes a campaign scope so no further run or reservation is admitted.
    pub async fn campaign_close(
        &self,
        campaign_id: &str,
        reason: &str,
    ) -> Result<crate::workflow::react::campaign::CampaignCloseResult, ApplicationError> {
        campaign_close_core(self, campaign_id, reason)
    }

    // -----------------------------------------------------------------------
    // Phase 2G+2H durable campaign schedule surface
    // -----------------------------------------------------------------------
    //
    // Additive to the immediate 2F campaign surface above: the existing
    // create/run/get/close contract is untouched, and these methods add the
    // durable queue that survives a restart (AC-2/AC-6). Only a marked
    // experiment domain accepts them, and every resource (execution profile,
    // bundle refs, fixture refs) is resolved server-side.

    /// Persists one validated durable schedule request and its ordered jobs.
    pub fn campaign_schedule(
        &self,
        request: crate::workflow::react::experiment_schedule::types::CampaignScheduleRequestV1,
        idempotency_key: &str,
    ) -> Result<
        crate::workflow::react::experiment_schedule::types::CampaignScheduleAcceptedV1,
        ApplicationError,
    > {
        campaign_schedule_core(self, request, idempotency_key)
    }

    /// The durable job list of one campaign, ordered by candidate order.
    pub fn campaign_jobs(
        &self,
        campaign_id: &str,
    ) -> Result<
        crate::workflow::react::experiment_schedule::types::CampaignJobListV1,
        ApplicationError,
    > {
        campaign_jobs_core(self, campaign_id)
    }

    /// One durable job by its backend-minted id.
    pub fn campaign_job(
        &self,
        job_id: &str,
    ) -> Result<crate::workflow::react::experiment_schedule::types::CampaignJobV1, ApplicationError>
    {
        campaign_job_core(self, job_id)
    }

    /// Cancels a campaign's pre-dispatch work; dispatched work is reported, not
    /// cancelled.
    pub async fn campaign_cancel(
        &self,
        campaign_id: &str,
        reason: &str,
    ) -> Result<
        crate::workflow::react::experiment_schedule::types::CampaignCancelV1,
        ApplicationError,
    > {
        campaign_cancel_core(self, campaign_id, reason).await
    }

    /// Evidence-only reconciliation: classify every non-terminal job from the
    /// durable state plus the workflow authority, parking the ones whose effect
    /// cannot be proven absent. It never requeues and never runs the kernel.
    pub fn campaign_reconcile(
        &self,
        campaign_id: &str,
    ) -> Result<
        crate::workflow::react::experiment_schedule::types::CampaignReconcileV1,
        ApplicationError,
    > {
        campaign_reconcile_core(self, campaign_id)
    }
}
