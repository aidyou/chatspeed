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
    create_workflow_core, get_workflow_events_core, get_workflow_snapshot_core,
    list_workflows_core, workflow_signal_core, workflow_start_core, workflow_stop_core,
};
use crate::db::{Agent, MainStore, Workflow};
use crate::libs::tsid::TsidGenerator;
use crate::workflow::react::client::hub::WorkflowRuntimeHub;
use crate::workflow::react::events::WorkflowEventRecord;
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
        }
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
}
