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
use crate::db::agent::{is_supported_sub_agent_role, normalize_agent_tool_config, McpToolConfig};
use crate::db::{Agent, MainStore, Workflow};
use crate::libs::tsid::TsidGenerator;
use crate::tools::ShellExecutionMode;
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
    /// The unique Agent Skill / MCP capability service shared by the Tauri
    /// commands, the control plane and (through it) the `cs` CLI. Holding it
    /// here keeps exactly one instance alive for the desktop owner, so the
    /// in-process single-flight locks and the durable journal cannot diverge.
    pub(crate) capability: Arc<crate::capability::CapabilityApplicationService>,
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
        let capability = Arc::new(
            crate::capability::CapabilityApplicationService::new(
                main_store.clone(),
                app_data_dir.clone(),
            )
            // The desktop process is the only runtime owner, so it is also the
            // only process that may observe *and* change MCP runtime state
            // (INV-1). Both ports come from the same `ChatState`.
            .with_mcp_runtime(
                Arc::new(
                    crate::capability::mcp::runtime::ToolManagerRuntimePort::new(
                        chat_state.clone(),
                    ),
                ),
                Arc::new(
                    crate::capability::mcp::runtime::ToolManagerRuntimeEffects::new(
                        chat_state.clone(),
                    ),
                ),
            ),
        );
        Self {
            main_store,
            chat_state,
            tsid_generator,
            gateway,
            factory,
            workflow_manager,
            app_data_dir,
            capability,
        }
    }

    /// The unique capability service, shared with the Tauri command layer.
    pub fn capability(&self) -> &Arc<crate::capability::CapabilityApplicationService> {
        &self.capability
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

    /// Adds an agent after the canonical runtime normalization.
    ///
    /// The runtime owns identity: the TSID is generated here (never by the
    /// transport), the agent is forced non-system, and the same sanitize +
    /// validation the desktop always applied runs before persistence. The new
    /// stable id is returned to the caller.
    pub async fn agent_add(&self, mut agent: Agent) -> Result<String, ApplicationError> {
        agent.id = self
            .tsid_generator
            .generate()
            .map_err(ApplicationError::internal)?;
        agent.is_system = Some(false);
        agent.version = Some(agent.version.unwrap_or(0));
        prepare_agent_for_persistence(&self.main_store, &mut agent)?;
        self.main_store
            .add_agent(&agent)
            .map_err(|e| ApplicationError::internal(e.to_string()))
    }

    /// Updates an agent with the canonical system-agent and sanitize rules.
    ///
    /// System agents keep their identifying fields, child/primary invariants are
    /// re-derived, and the sandbox scheme reference is validated against the
    /// runtime's own store before the row is written.
    pub async fn agent_update(&self, agent: Agent) -> Result<(), ApplicationError> {
        let effective_agent = match self
            .main_store
            .get_agent(&agent.id)
            .map_err(|e| ApplicationError::internal(e.to_string()))?
        {
            Some(existing) if existing.is_system.unwrap_or(false) => {
                let mut updated = agent;
                updated.id = existing.id.clone();
                updated.name = existing.name.clone();
                updated.description = existing.description.clone();
                updated.role = existing.role.clone();
                updated.parent_agent_id = existing.parent_agent_id.clone();
                updated.sub_agent_role = existing.sub_agent_role.clone();
                updated.system_prompt = existing.system_prompt.clone();
                updated.planning_prompt = existing.planning_prompt.clone();
                updated.is_system = existing.is_system;
                updated.version = existing.version;
                updated.sort_index = existing.sort_index;
                updated
            }
            Some(existing) => {
                let mut updated = agent;
                updated.is_system = Some(false);
                updated.version = existing.version.or(Some(0));
                updated.sort_index = existing.sort_index;
                updated
            }
            None => {
                let mut updated = agent;
                updated.is_system = Some(false);
                updated.version = Some(updated.version.unwrap_or(0));
                updated
            }
        };

        let mut effective_agent = effective_agent;
        prepare_agent_for_persistence(&self.main_store, &mut effective_agent)?;
        self.main_store
            .update_agent(&effective_agent)
            .map_err(|e| ApplicationError::internal(e.to_string()))
    }

    /// Deletes an agent, refusing the built-in system agents.
    pub async fn agent_delete(&self, id: &str) -> Result<(), ApplicationError> {
        if self
            .main_store
            .get_agent(id)
            .map_err(|e| ApplicationError::internal(e.to_string()))?
            .is_some_and(|agent| agent.is_system.unwrap_or(false))
        {
            return Err(ApplicationError::invalid_input(
                "System agent cannot be deleted",
            ));
        }
        self.main_store
            .delete_agent(id)
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

    /// Dispatches one allowlisted workflow compatibility command.
    ///
    /// The command name must already be allowlisted by the transport, and the
    /// core re-validates it. The returned JSON keeps the historical Tauri
    /// camelCase shape, so the compatibility route is lossless for the desktop.
    pub async fn workflow_command(
        &self,
        command: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ApplicationError> {
        crate::commands::workflow::workflow_command_core(self, command, params).await
    }
}

/// The canonical agent persistence pipeline: sanitize, then validate the
/// sub-agent role and the sandbox scheme reference.
///
/// This is runtime-owned business logic. The desktop command wrappers only
/// forward the raw agent over `/control/v1`, so normalization can never diverge
/// between the desktop and any other client.
fn prepare_agent_for_persistence(
    store: &MainStore,
    agent: &mut Agent,
) -> Result<(), ApplicationError> {
    sanitize_agent_for_persistence(agent).map_err(ApplicationError::invalid_input)?;
    validate_sub_agent_role(agent).map_err(ApplicationError::invalid_input)?;
    validate_sandbox_scheme_reference(store, agent).map_err(ApplicationError::invalid_input)?;
    Ok(())
}

fn filter_tool_list_json(raw: Option<String>, blocked_tool: &str) -> Option<String> {
    let tools = raw
        .as_deref()
        .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|tool| tool != blocked_tool)
        .collect::<Vec<_>>();
    Some(serde_json::to_string(&tools).unwrap_or_else(|_| "[]".to_string()))
}

fn filter_git_inspection_tools_for_role(raw: Option<String>, role: Option<&str>) -> Option<String> {
    let tools = raw
        .as_deref()
        .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|tool| {
            role == Some("child")
                || !matches!(
                    tool.as_str(),
                    crate::tools::TOOL_GIT_DIFF | crate::tools::TOOL_GIT_INSPECT
                )
        })
        .collect::<Vec<_>>();
    Some(serde_json::to_string(&tools).unwrap_or_else(|_| "[]".to_string()))
}

fn sanitize_agent_for_persistence(agent: &mut Agent) -> Result<(), String> {
    agent.personality = agent
        .personality
        .take()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    let available_tools = agent
        .available_tools
        .as_deref()
        .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok());
    let auto_approve = agent
        .auto_approve
        .as_deref()
        .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok());
    let mcp_tools = agent
        .mcp_tool_exposure
        .as_deref()
        .and_then(|value| serde_json::from_str::<McpToolConfig>(value).ok());
    let (available_tools, auto_approve, mcp_tools) =
        normalize_agent_tool_config(available_tools, auto_approve, mcp_tools);
    agent.available_tools = serde_json::to_string(&available_tools.unwrap_or_default()).ok();
    agent.auto_approve = serde_json::to_string(&auto_approve.unwrap_or_default()).ok();
    agent.mcp_tool_exposure = mcp_tools.and_then(|config| serde_json::to_string(&config).ok());

    let available_tools = agent
        .available_tools
        .as_deref()
        .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
        .unwrap_or_default();
    let has_bash = available_tools
        .iter()
        .any(|tool| tool == crate::tools::TOOL_BASH);
    let role = agent.role.as_deref();

    // Children inherit approval from their parent and never configure a shell sandbox or
    // shell auto-approval, so those stay at their safe defaults for them.
    if !has_bash || role == Some("child") {
        agent.auto_approve =
            filter_tool_list_json(agent.auto_approve.clone(), crate::tools::TOOL_BASH);
        agent.sandbox_execution_mode = ShellExecutionMode::HostOnly;
        agent.sandbox_scheme_id = None;
    } else if matches!(agent.sandbox_execution_mode, ShellExecutionMode::HostOnly) {
        agent.sandbox_scheme_id = None;
    }

    let role = agent.role.as_deref();
    if role != Some("child") {
        agent.parent_agent_id = None;
        agent.sub_agent_role = None;
    } else {
        agent.sub_agent_role = agent
            .sub_agent_role
            .take()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
    }
    agent.available_tools =
        filter_git_inspection_tools_for_role(agent.available_tools.clone(), role);
    agent.auto_approve = filter_git_inspection_tools_for_role(agent.auto_approve.clone(), role);

    if role != Some("child") {
        return Ok(());
    }

    agent.planning_prompt = None;
    agent.image_recognition_prompt = None;
    agent.personality = None;
    agent.allowed_paths = Some("[]".to_string());
    // Shell rules stay per-agent: a child that enables shell owns its own command rules,
    // and the workflow later merges the parent's allow rules into them.
    agent.sandbox_execution_mode = ShellExecutionMode::HostOnly;
    agent.sandbox_scheme_id = None;
    agent.skill_enabled = Some(false);
    agent.selected_skills = Some("[]".to_string());

    if let Some(models) = agent.models.as_mut() {
        models.plan = None;
        models.vision = None;
        models.utility = None;
        models.lite = None;
        models.decision_enabled = false;
        models.decision = None;
    }
    Ok(())
}

fn validate_sandbox_scheme_reference(store: &MainStore, agent: &Agent) -> Result<(), String> {
    match agent.sandbox_execution_mode {
        ShellExecutionMode::HostOnly => {
            if agent.sandbox_scheme_id.is_some() {
                return Err("host_only agents cannot reference a sandbox scheme".to_string());
            }
        }
        ShellExecutionMode::Auto | ShellExecutionMode::SandboxOnly => {
            let scheme_id = agent.sandbox_scheme_id.as_deref().ok_or_else(|| {
                "auto and sandbox_only agents must select a sandbox scheme".to_string()
            })?;
            let scheme = store
                .get_sandbox_scheme(scheme_id)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "sandbox scheme not found".to_string())?;
            if scheme.disabled {
                return Err("disabled sandbox schemes cannot be assigned to agents".to_string());
            }
            if agent.sandbox_execution_mode == ShellExecutionMode::Auto
                && crate::tools::enabled_common_profile(scheme.config.profiles.iter())?.is_none()
            {
                return Err(
                    "auto agents require one enabled common catch-all sandbox profile".to_string(),
                );
            }
        }
    }
    Ok(())
}

fn validate_sub_agent_role(agent: &Agent) -> Result<(), String> {
    if agent.role.as_deref() == Some("child") && agent.parent_agent_id.is_none() {
        return Err("Child agents must belong to a primary agent".to_string());
    }
    if let Some(role) = agent.sub_agent_role.as_deref() {
        if !is_supported_sub_agent_role(role) {
            return Err(format!("Unsupported sub-agent role: {role}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{sanitize_agent_for_persistence, validate_sandbox_scheme_reference};
    use crate::db::{Agent, MainStore, SandboxScheme};
    use crate::tools::{
        SandboxNetworkPolicy, SandboxProfileConfig, SandboxSchemeConfig, ShellExecutionMode,
        WorkspaceAccess,
    };

    #[test]
    fn primary_agents_cannot_persist_git_review_tools() {
        let mut agent = Agent::new(
            "primary-test".to_string(),
            "Primary Test".to_string(),
            None,
            Some("primary".to_string()),
            None,
            String::new(),
            None,
            None,
            Some(
                serde_json::json!([
                    crate::tools::TOOL_GIT_DIFF,
                    crate::tools::TOOL_GIT_INSPECT,
                    crate::tools::TOOL_READ_FILE,
                ])
                .to_string(),
            ),
            Some(
                serde_json::json!([crate::tools::TOOL_GIT_DIFF, crate::tools::TOOL_GIT_INSPECT,])
                    .to_string(),
            ),
            None,
            Some("[]".to_string()),
            Some("[]".to_string()),
            Some(false),
            Some("default".to_string()),
            Some(true),
            Some("[]".to_string()),
            Some("standard".to_string()),
            Some(false),
            Some(true),
            None,
        );

        sanitize_agent_for_persistence(&mut agent).expect("sanitize primary agent");
        let available_tools = serde_json::from_str::<Vec<String>>(
            agent.available_tools.as_deref().expect("available tools"),
        )
        .expect("available tools json");
        let auto_approve = serde_json::from_str::<Vec<String>>(
            agent.auto_approve.as_deref().expect("auto approve"),
        )
        .expect("auto approve json");
        assert_eq!(available_tools, vec![crate::tools::TOOL_READ_FILE]);
        assert!(auto_approve.is_empty());
    }

    #[test]
    fn no_shell_and_child_agents_cannot_persist_sandbox_scheme_references() {
        let mut no_shell = Agent::new(
            "no-shell".to_string(),
            "No Shell".to_string(),
            None,
            Some("primary".to_string()),
            None,
            String::new(),
            None,
            None,
            Some(serde_json::json!([crate::tools::TOOL_READ_FILE]).to_string()),
            Some(serde_json::json!([crate::tools::TOOL_BASH]).to_string()),
            None,
            Some("[]".to_string()),
            Some("[]".to_string()),
            Some(false),
            Some("default".to_string()),
            Some(true),
            Some("[]".to_string()),
            Some("standard".to_string()),
            Some(false),
            Some(false),
            None,
        );
        no_shell.sandbox_execution_mode = ShellExecutionMode::Auto;
        no_shell.sandbox_scheme_id = Some("scheme-1".to_string());

        sanitize_agent_for_persistence(&mut no_shell).expect("sanitize no-shell agent");
        assert_eq!(
            no_shell.sandbox_execution_mode,
            ShellExecutionMode::HostOnly
        );
        assert!(no_shell.sandbox_scheme_id.is_none());
        assert_eq!(
            serde_json::from_str::<Vec<String>>(no_shell.auto_approve.as_deref().unwrap())
                .expect("auto approve json"),
            Vec::<String>::new()
        );

        no_shell.available_tools = Some(serde_json::json!([crate::tools::TOOL_BASH]).to_string());
        no_shell.sandbox_execution_mode = ShellExecutionMode::Auto;
        no_shell.sandbox_scheme_id = Some("scheme-1".to_string());
        sanitize_agent_for_persistence(&mut no_shell).expect("sanitize bash agent");
        assert_eq!(no_shell.sandbox_execution_mode, ShellExecutionMode::Auto);
        assert_eq!(no_shell.sandbox_scheme_id.as_deref(), Some("scheme-1"));

        let mut child = Agent::new(
            "child".to_string(),
            "Child".to_string(),
            None,
            Some("child".to_string()),
            Some("parent".to_string()),
            String::new(),
            Some("planning".to_string()),
            None,
            Some(serde_json::json!([crate::tools::TOOL_BASH]).to_string()),
            Some(serde_json::json!([crate::tools::TOOL_BASH]).to_string()),
            None,
            Some("[{\"pattern\":\"^git status$\",\"decision\":\"allow\"}]".to_string()),
            Some(serde_json::json!(["/tmp"]).to_string()),
            Some(false),
            Some("default".to_string()),
            Some(true),
            Some(serde_json::json!(["help"]).to_string()),
            Some("standard".to_string()),
            Some(false),
            Some(false),
            None,
        );
        child.sandbox_execution_mode = ShellExecutionMode::SandboxOnly;
        child.sandbox_scheme_id = Some("scheme-1".to_string());

        child.models = Some(crate::db::agent::AgentModels {
            decision_enabled: true,
            decision: Some(crate::db::agent::ModelConfig {
                id: 42,
                model: "jev-latest".into(),
                temperature: None,
                thinking: None,
                function_call: None,
                context_size: None,
                max_tokens: None,
            }),
            ..Default::default()
        });
        sanitize_agent_for_persistence(&mut child).expect("sanitize child agent");
        assert_eq!(child.sandbox_execution_mode, ShellExecutionMode::HostOnly);
        assert!(child.sandbox_scheme_id.is_none());
        // Shell is an opt-in child capability, so the selected tool survives sanitizing
        // while automatic shell approval stays off.
        assert_eq!(
            serde_json::from_str::<Vec<String>>(child.available_tools.as_deref().unwrap())
                .expect("available tools json"),
            vec![crate::tools::TOOL_BASH]
        );
        assert_eq!(
            serde_json::from_str::<Vec<String>>(child.auto_approve.as_deref().unwrap())
                .expect("auto approve json"),
            Vec::<String>::new()
        );
        assert_eq!(child.allowed_paths.as_deref(), Some("[]"));
        // A child keeps the shell rules it configured; only its paths and sandbox stay forced.
        assert_eq!(
            child.shell_policy.as_deref(),
            Some(r#"[{"pattern":"^git status$","decision":"allow"}]"#)
        );
        assert!(!child.models.as_ref().unwrap().decision_enabled);
        assert!(child.models.as_ref().unwrap().decision.is_none());
    }

    #[test]
    fn auto_agents_require_exactly_one_enabled_common_profile() {
        let store = MainStore::new(":memory:").expect("create store");
        let common_profile = SandboxProfileConfig {
            id: "common".to_string(),
            name: "Common".to_string(),
            enabled: true,
            priority: 0,
            command_patterns: vec![".*".to_string()],
            runtime_preference: Default::default(),
            image: "busybox:latest".to_string(),
            instance_name: None,
            image_size_bytes: Some(1),
            network: SandboxNetworkPolicy::default(),
            resources: Default::default(),
            workspace_access: WorkspaceAccess::ReadWrite,
        };
        let scheme_config = SandboxSchemeConfig {
            runtime_preference: Default::default(),
            profiles: vec![common_profile.clone()],
            host_rules: vec![],
        };
        let scheme = SandboxScheme {
            id: "one-common".to_string(),
            name: "One common".to_string(),
            description: String::new(),
            config: scheme_config.clone(),
            disabled: false,
            created_at: None,
            updated_at: None,
        };
        store
            .add_sandbox_scheme(&scheme)
            .expect("add one-common scheme");

        let mut missing_common_config = scheme_config.clone();
        missing_common_config.profiles[0].command_patterns = vec![r"^echo(?:\s|$)".to_string()];
        store
            .add_sandbox_scheme(&SandboxScheme {
                id: "missing-common".to_string(),
                name: "Missing common".to_string(),
                description: String::new(),
                config: missing_common_config,
                disabled: false,
                created_at: None,
                updated_at: None,
            })
            .expect("add missing-common scheme");

        let mut duplicate_common_config = scheme_config;
        let mut second_common = common_profile;
        second_common.id = "common-second".to_string();
        second_common.name = "Second common".to_string();
        duplicate_common_config.profiles.push(second_common);
        store
            .add_sandbox_scheme(&SandboxScheme {
                id: "duplicate-common".to_string(),
                name: "Duplicate common".to_string(),
                description: String::new(),
                config: duplicate_common_config,
                disabled: false,
                created_at: None,
                updated_at: None,
            })
            .expect("add duplicate-common scheme");

        let mut agent = Agent::new(
            "agent".to_string(),
            "Agent".to_string(),
            None,
            Some("primary".to_string()),
            None,
            String::new(),
            None,
            None,
            Some(serde_json::json!([crate::tools::TOOL_BASH]).to_string()),
            Some("[]".to_string()),
            None,
            Some("[]".to_string()),
            Some("[]".to_string()),
            Some(false),
            Some("default".to_string()),
            Some(true),
            Some("[]".to_string()),
            Some("standard".to_string()),
            Some(false),
            Some(false),
            None,
        );
        agent.sandbox_execution_mode = ShellExecutionMode::Auto;

        agent.sandbox_scheme_id = Some("one-common".to_string());
        validate_sandbox_scheme_reference(&store, &agent)
            .expect("auto accepts one enabled common profile");

        agent.sandbox_scheme_id = Some("missing-common".to_string());
        assert!(validate_sandbox_scheme_reference(&store, &agent).is_err());

        agent.sandbox_scheme_id = Some("duplicate-common".to_string());
        assert!(validate_sandbox_scheme_reference(&store, &agent).is_err());

        agent.sandbox_execution_mode = ShellExecutionMode::SandboxOnly;
        validate_sandbox_scheme_reference(&store, &agent).expect("sandbox-only common is allowed");
    }
}
