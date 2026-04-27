use crate::ai::interaction::chat_completion::ChatState;
use crate::ai::traits::chat::MCPToolDeclaration;
use crate::db::{Agent, AgentConfig, MainStore, WorkflowMessage};
use crate::tools::{NativeToolResult, ToolCallResult, ToolCategory, ToolDefinition, ToolError};
use crate::workflow::react::child_tasks::{
    get_sub_agent_registry, render_call_mode_sub_agent_tool_result,
};
use crate::workflow::react::engine::ReActExecutor;
use crate::workflow::react::error::WorkflowEngineError;
use crate::workflow::react::events::WorkflowEvent;
use crate::workflow::react::gateway::Gateway;
use crate::workflow::react::types::{ExecutionContext, RuntimeState, SubAgentCompletion};

use async_trait::async_trait;
use dashmap::DashMap;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Represents different types of background tasks for unified management
pub enum BackgroundTask {
    /// An autonomous sub-agent running its own ReAct loop
    SubAgent {
        owner_session_id: Option<String>,
        executor: Arc<Mutex<dyn ReActExecutor>>,
        output_accessible: bool,
    },
}

#[derive(Debug, Clone)]
pub struct CompletedTaskSnapshot {
    pub owner_session_id: Option<String>,
    pub output: String,
    pub output_accessible: bool,
    pub consumed: bool,
}

impl BackgroundTask {
    pub fn owner_session_id(&self) -> Option<&str> {
        match self {
            BackgroundTask::SubAgent {
                owner_session_id, ..
            } => owner_session_id.as_deref(),
        }
    }

    pub fn output_accessible(&self) -> bool {
        match self {
            BackgroundTask::SubAgent {
                output_accessible, ..
            } => *output_accessible,
        }
    }
}

lazy_static::lazy_static! {
    /// Global registry for background sub-agents.
    pub static ref BACKGROUND_TASKS: Arc<DashMap<String, BackgroundTask>> = Arc::new(DashMap::new());
    /// Terminal snapshots allow sub_agent_output to inspect sub-agents after they leave the active registry.
    pub static ref COMPLETED_BACKGROUND_TASKS: Arc<DashMap<String, CompletedTaskSnapshot>> = Arc::new(DashMap::new());
    static ref TASK_OUTPUT_THROTTLE: Arc<DashMap<String, (i64, String)>> = Arc::new(DashMap::new());
    static ref PENDING_TASK_OUTPUT_WAITS: Arc<DashMap<String, usize>> = Arc::new(DashMap::new());
}

fn remember_completed_task(
    task_id: impl Into<String>,
    owner_session_id: Option<String>,
    output: impl Into<String>,
    output_accessible: bool,
) {
    COMPLETED_BACKGROUND_TASKS.insert(
        task_id.into(),
        CompletedTaskSnapshot {
            owner_session_id,
            output: output.into(),
            output_accessible,
            consumed: false,
        },
    );
}

fn register_task_output_wait(task_id: &str) {
    PENDING_TASK_OUTPUT_WAITS
        .entry(task_id.to_string())
        .and_modify(|count| *count += 1)
        .or_insert(1);
}

fn unregister_task_output_wait(task_id: &str) {
    if let Some(mut entry) = PENDING_TASK_OUTPUT_WAITS.get_mut(task_id) {
        if *entry <= 1 {
            drop(entry);
            PENDING_TASK_OUTPUT_WAITS.remove(task_id);
        } else {
            *entry -= 1;
        }
    }
}

fn has_pending_task_output_wait(task_id: &str) -> bool {
    PENDING_TASK_OUTPUT_WAITS.contains_key(task_id)
}

fn mark_completed_task_consumed(task_id: &str) {
    if let Some(mut snapshot) = COMPLETED_BACKGROUND_TASKS.get_mut(task_id) {
        snapshot.consumed = true;
    }
}

fn sync_sub_agent_completion_consumed(
    main_store: &Arc<std::sync::RwLock<MainStore>>,
    session_id: &str,
    task_id: &str,
) -> Result<(), WorkflowEngineError> {
    let store = main_store
        .read()
        .map_err(|e| WorkflowEngineError::General(e.to_string()))?;
    let Some(mut context) = store
        .get_execution_context(session_id)
        .map_err(WorkflowEngineError::Db)?
    else {
        return Ok(());
    };

    let mut changed = false;
    for completion in &mut context.pending_sub_agent_completions {
        if completion.sub_agent_id == task_id && !completion.consumed {
            completion.consumed = true;
            changed = true;
        }
    }

    if changed {
        store
            .upsert_execution_context(&context)
            .map_err(WorkflowEngineError::Db)?;
    }

    Ok(())
}

fn find_durable_sub_agent_completion(
    main_store: &Arc<std::sync::RwLock<MainStore>>,
    session_id: &str,
    task_id: &str,
) -> Result<Option<SubAgentCompletion>, WorkflowEngineError> {
    let store = main_store
        .read()
        .map_err(|e| WorkflowEngineError::General(e.to_string()))?;
    Ok(store
        .get_execution_context(session_id)
        .map_err(WorkflowEngineError::Db)?
        .and_then(|context| {
            context
                .pending_sub_agent_completions
                .into_iter()
                .find(|completion| completion.sub_agent_id == task_id)
        }))
}

fn build_sub_agent_completion(
    parent_session_id: &str,
    sub_agent_id: &str,
    result: &Value,
) -> SubAgentCompletion {
    SubAgentCompletion {
        sub_agent_id: sub_agent_id.to_string(),
        parent_session_id: parent_session_id.to_string(),
        status: result
            .get("status")
            .and_then(|value| value.as_str())
            .unwrap_or("completed")
            .to_string(),
        result: result
            .get("result")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        summary: result
            .get("summary")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        error: result
            .get("error")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        tool_calls_count: result
            .get("tool_calls_count")
            .and_then(|value| value.as_u64())
            .unwrap_or(0) as usize,
        completed_at_ms: chrono::Utc::now().timestamp_millis(),
        consumed: false,
    }
}

fn persist_sub_agent_completion(
    main_store: &Arc<std::sync::RwLock<MainStore>>,
    completion: SubAgentCompletion,
) -> Result<(), WorkflowEngineError> {
    let store = main_store
        .read()
        .map_err(|e| WorkflowEngineError::General(e.to_string()))?;
    let mut context = store
        .get_execution_context(&completion.parent_session_id)
        .map_err(WorkflowEngineError::Db)?
        .unwrap_or_else(|| ExecutionContext {
            session_id: completion.parent_session_id.clone(),
            state: RuntimeState::Waiting,
            wait_reason: Some(crate::workflow::react::types::WaitReason::SubAgent),
            current_step: 0,
            max_steps: 0,
            pending_tools: Vec::new(),
            last_action_summary: None,
            current_context_tokens: None,
            max_context_tokens: None,
            last_event_id: None,
            version: ExecutionContext::CURRENT_VERSION.to_string(),
            waiting_on_sub_agent_id: Some(completion.sub_agent_id.clone()),
            sub_agent_sessions: vec![completion.sub_agent_id.clone()],
            pending_sub_agent_completions: Vec::new(),
        });

    context
        .pending_sub_agent_completions
        .retain(|existing| existing.sub_agent_id != completion.sub_agent_id);
    context.pending_sub_agent_completions.push(completion);
    store
        .upsert_execution_context(&context)
        .map_err(WorkflowEngineError::Db)?;
    Ok(())
}

fn extract_submit_result_payload(messages: &[WorkflowMessage]) -> Option<(String, String)> {
    messages.iter().rev().find_map(|message| {
        if message.role != "tool" {
            return None;
        }

        let metadata = message.metadata.as_ref()?;
        if metadata.get("tool_name").and_then(|value| value.as_str())
            != Some(crate::tools::TOOL_SUBMIT_RESULT)
        {
            return None;
        }

        let structured = metadata.get("structured_content")?;
        let result = structured.get("result").and_then(|value| value.as_str())?;
        let summary = structured
            .get("summary")
            .and_then(|value| value.as_str())
            .unwrap_or(result);

        Some((result.to_string(), summary.to_string()))
    })
}

fn build_terminal_sub_agent_result(
    task_id: &str,
    final_state: crate::workflow::react::types::WorkflowState,
    run_result: &Result<(), WorkflowEngineError>,
    messages: &[WorkflowMessage],
) -> Value {
    let tool_calls_count = messages
        .iter()
        .filter(|message| message.role == "tool")
        .count();

    match run_result {
        Ok(_) => {
            let status = if final_state == crate::workflow::react::types::WorkflowState::Cancelled {
                "cancelled"
            } else {
                "completed"
            };
            let (result, summary) = extract_submit_result_payload(messages).unwrap_or_else(|| {
                (
                    "Sub-agent completed".to_string(),
                    "Sub-agent completed".to_string(),
                )
            });
            json!({
                "status": status,
                "task_id": task_id,
                "result": result,
                "summary": summary,
                "tool_calls_count": tool_calls_count
            })
        }
        Err(error) => {
            let status = if matches!(error, WorkflowEngineError::Cancelled(_))
                || final_state == crate::workflow::react::types::WorkflowState::Cancelled
            {
                "cancelled"
            } else {
                "failed"
            };
            json!({
                "status": status,
                "task_id": task_id,
                "error": error.to_string(),
                "tool_calls_count": tool_calls_count
            })
        }
    }
}

fn render_task_output(result: &Value) -> String {
    match result.get("status").and_then(|value| value.as_str()) {
        Some("completed") => result
            .get("result")
            .and_then(|value| value.as_str())
            .or_else(|| result.get("summary").and_then(|value| value.as_str()))
            .unwrap_or("Sub-agent completed")
            .to_string(),
        Some("failed" | "cancelled" | "interrupted") => result
            .get("error")
            .and_then(|value| value.as_str())
            .or_else(|| result.get("summary").and_then(|value| value.as_str()))
            .unwrap_or("Sub-agent did not complete successfully")
            .to_string(),
        _ => result
            .get("summary")
            .and_then(|value| value.as_str())
            .unwrap_or("Sub-agent completed")
            .to_string(),
    }
}

fn append_sub_agent_event(
    main_store: &Arc<std::sync::RwLock<MainStore>>,
    event: WorkflowEvent,
) -> Result<(), WorkflowEngineError> {
    let store = main_store
        .read()
        .map_err(|e| WorkflowEngineError::General(e.to_string()))?;
    store
        .append_workflow_event(&event)
        .map_err(WorkflowEngineError::Db)?;
    Ok(())
}

fn filter_sub_agent_tool_ids(tools_json: Option<&str>) -> Option<String> {
    let mut tools = tools_json.and_then(|tools| serde_json::from_str::<Vec<String>>(tools).ok())?;
    tools.retain(|tool| {
        // System/internal tools that sub-agents should not access
        tool != crate::tools::TOOL_BASH
            && tool != crate::tools::TOOL_SUB_AGENT_RUN
            && tool != crate::tools::TOOL_SUB_AGENT_OUTPUT
            && tool != crate::tools::TOOL_SUB_AGENT_STOP
            // Plan-related tools are for planning mode (writing records), not for sub-agents
            && tool != crate::tools::TOOL_PLAN_READ_NOTE
            && tool != crate::tools::TOOL_PLAN_EDIT_NOTE
            && tool != crate::tools::TOOL_PLAN_WRITE_NOTE
            // ask_user requires user interaction, sub-agents cannot use it
            && tool != crate::tools::TOOL_ASK_USER
    });
    serde_json::to_string(&tools).ok()
}

fn is_task_output_id(task_id: &str) -> bool {
    task_id.starts_with("subagent_")
}

fn format_available_task_ids_for_owner(owner_session_id: &str) -> String {
    let mut task_ids: Vec<String> = BACKGROUND_TASKS
        .iter()
        .filter_map(|entry| {
            if entry.value().owner_session_id() == Some(owner_session_id)
                && entry.value().output_accessible()
            {
                Some(format!("{} (active)", entry.key()))
            } else {
                None
            }
        })
        .chain(COMPLETED_BACKGROUND_TASKS.iter().filter_map(|entry| {
            if entry.value().owner_session_id.as_deref() == Some(owner_session_id)
                && entry.value().output_accessible
            {
                Some(format!("{} (completed)", entry.key()))
            } else {
                None
            }
        }))
        .collect();
    task_ids.sort();
    task_ids.dedup();

    if task_ids.is_empty() {
        "No sub-agent IDs are currently available in this session. Start a background sub-agent first.".to_string()
    } else {
        format!(
            "Currently accessible sub-agent IDs for this session: {}.",
            task_ids.join(", ")
        )
    }
}

pub fn list_background_task_ids_for_owner(owner_session_id: &str) -> Vec<String> {
    BACKGROUND_TASKS
        .iter()
        .filter_map(|entry| {
            if entry.value().owner_session_id() == Some(owner_session_id)
                && entry.value().output_accessible()
            {
                Some(entry.key().clone())
            } else {
                None
            }
        })
        .collect()
}

fn validate_task_access(current_session_id: &str, task_id: &str) -> Result<(), ToolError> {
    if !is_task_output_id(task_id) {
        return Err(ToolError::InvalidParams(
            "task_id must be a sub-agent id starting with 'subagent_'".to_string(),
        ));
    }

    if let Some(task) = BACKGROUND_TASKS.get(task_id) {
        if task.value().owner_session_id() == Some(current_session_id) {
            return Ok(());
        }
        if !task.value().output_accessible() {
            return Err(ToolError::ExecutionFailed(format!(
                "Task {} is not accessible from the current session.",
                task_id
            )));
        }
        return Err(ToolError::ExecutionFailed(format!(
            "Task {} is not accessible from the current session.",
            task_id
        )));
    }

    if let Some(snapshot) = COMPLETED_BACKGROUND_TASKS.get(task_id) {
        if snapshot.owner_session_id.as_deref() == Some(current_session_id) {
            return Ok(());
        }
        if !snapshot.output_accessible {
            return Err(ToolError::ExecutionFailed(format!(
                "Task {} is not accessible from the current session.",
                task_id
            )));
        }
        return Err(ToolError::ExecutionFailed(format!(
            "Task {} is not accessible from the current session.",
            task_id
        )));
    }

    Ok(())
}

pub async fn stop_background_task(task_id: &str, chat_state: Option<&Arc<ChatState>>) -> bool {
    let Some((_, task)) = BACKGROUND_TASKS.remove(task_id) else {
        return false;
    };

    match task {
        BackgroundTask::SubAgent {
            owner_session_id,
            output_accessible,
            ..
        } => {
            if let Some(chat_state) = chat_state {
                let mut chats = chat_state.chats.lock().await;
                if let Some(protocol_chats) = chats.get_mut(&crate::ccproxy::ChatProtocol::OpenAI) {
                    if let Some(chat) = protocol_chats.get_mut(task_id) {
                        chat.set_stop_flag(true).await;
                    }
                }
            }

            if let Err(error) =
                crate::workflow::react::manager::WorkflowManager::send_signal_to_session(
                    task_id,
                    serde_json::json!({ "type": "stop" }).to_string(),
                )
            {
                log::warn!(
                    "[Workflow][session={}][phase=cleanup] Failed to send stop signal to sub-agent session: {}",
                    task_id,
                    error
                );
            }
            crate::workflow::react::manager::WorkflowManager::unregister_session_signal_tx(task_id);
            remember_completed_task(
                task_id,
                owner_session_id,
                format!("Sub-agent {} has been cancelled.", task_id),
                output_accessible,
            );
            get_sub_agent_registry().unregister_sub_agent(task_id);
            true
        }
    }
}

#[async_trait]
pub trait SubAgentFactory: Send + Sync {
    /// Creates a new executor instance for a sub-agent with specialized configurations.
    async fn create_executor(
        &self,
        agent_id: &str,
        session_id: &str,
        task: &str,
        subagent_type: &str,
        parent_session_id: Option<&str>,
    ) -> Result<Arc<Mutex<dyn ReActExecutor>>, WorkflowEngineError>;
}

/// The default factory used to spawn sub-agents within the ReAct system
pub struct DefaultSubAgentFactory {
    pub main_store: Arc<std::sync::RwLock<MainStore>>,
    pub chat_state: Arc<ChatState>,
    pub gateway: Arc<dyn Gateway>,
    pub app_data_dir: PathBuf,
    pub tsid_generator: Arc<crate::libs::tsid::TsidGenerator>,
}

#[async_trait]
impl SubAgentFactory for DefaultSubAgentFactory {
    async fn create_executor(
        &self,
        agent_id: &str,
        session_id: &str,
        task: &str,
        subagent_type: &str,
        parent_session_id: Option<&str>,
    ) -> Result<Arc<Mutex<dyn ReActExecutor>>, WorkflowEngineError> {
        let mut agent_config = {
            let store = self.main_store.read().map_err(|e| {
                WorkflowEngineError::Db(crate::db::error::StoreError::LockError(e.to_string()))
            })?;
            store.get_agent(agent_id)?.ok_or_else(|| {
                WorkflowEngineError::General(format!("Agent config {} not found", agent_id))
            })?
        };
        agent_config.available_tools =
            filter_sub_agent_tool_ids(agent_config.available_tools.as_deref());
        agent_config.auto_approve = filter_sub_agent_tool_ids(agent_config.auto_approve.as_deref());

        let inherited_allowed_paths = if let Some(parent_session_id) = parent_session_id {
            let store = self.main_store.read().map_err(|e| {
                WorkflowEngineError::Db(crate::db::error::StoreError::LockError(e.to_string()))
            })?;
            store
                .get_workflow_snapshot(parent_session_id)
                .ok()
                .and_then(|snapshot| snapshot.workflow.agent_config)
                .and_then(|config_json| AgentConfig::from_json(&config_json))
                .and_then(|config| config.allowed_paths)
        } else {
            None
        };

        if let Some(paths) = inherited_allowed_paths.clone() {
            agent_config.allowed_paths = serde_json::to_string(&paths).ok();
        }

        let workflow_allowed_paths = inherited_allowed_paths.or_else(|| {
            agent_config
                .allowed_paths
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
        });
        let runtime_allowed_paths = workflow_allowed_paths
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>();

        let workflow_config = AgentConfig {
            allowed_paths: workflow_allowed_paths,
            shell_policy: agent_config
                .shell_policy
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok()),
            approval_level: agent_config.approval_level.clone(),
            auto_approve: agent_config
                .auto_approve
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok()),
            available_tools: agent_config
                .available_tools
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok()),
            final_audit: agent_config.final_audit,
            phase: None,
            models: agent_config.models.clone(),
            max_contexts: agent_config.max_contexts,
        };

        {
            let store = self.main_store.read().map_err(|e| {
                WorkflowEngineError::Db(crate::db::error::StoreError::LockError(e.to_string()))
            })?;
            store.create_workflow(
                session_id,
                task,
                &agent_config.id,
                Some(workflow_config.to_json()),
                parent_session_id,
            )?;
            store.add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "user".to_string(),
                message: task.to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: Some("delegated_task".to_string()),
                segment_id: 1,
                source_event_type: Some("sub_agent_started".to_string()),
                metadata: Some(json!({
                    "type": "delegated_task",
                    "parentSessionId": parent_session_id,
                    "subagentType": subagent_type
                })),
                attached_context: None,
                step_type: None,
                step_index: 0,
                is_error: false,
                error_type: None,
                created_at: None,
            })?;
        }

        let (signal_tx, signal_rx) = tokio::sync::mpsc::channel(32);
        crate::workflow::react::manager::WorkflowManager::register_session_signal_tx(
            session_id.to_string(),
            signal_tx,
        );

        let policy = if subagent_type == "Planning" {
            crate::workflow::react::policy::ExecutionPolicy::planning()
        } else {
            crate::workflow::react::policy::ExecutionPolicy::standard()
        };

        if subagent_type == "Planning" {
            Ok(Arc::new(Mutex::new(
                crate::workflow::react::planners::PlanningExecutor::new(
                    session_id.to_string(),
                    self.main_store.clone(),
                    self.chat_state.clone(),
                    self.gateway.clone(),
                    Arc::new(DefaultSubAgentFactory {
                        main_store: self.main_store.clone(),
                        chat_state: self.chat_state.clone(),
                        gateway: self.gateway.clone(),
                        app_data_dir: self.app_data_dir.clone(),
                        tsid_generator: self.tsid_generator.clone(),
                    }),
                    agent_config,
                    runtime_allowed_paths,
                    self.app_data_dir.clone(),
                    Some(subagent_type.to_string()),
                    Some(signal_rx),
                    self.tsid_generator.clone(),
                    self.chat_state.tool_manager.clone(),
                    policy,
                ),
            )))
        } else {
            Ok(Arc::new(Mutex::new(
                crate::workflow::react::runners::ExecutionExecutor::new(
                    session_id.to_string(),
                    self.main_store.clone(),
                    self.chat_state.clone(),
                    self.gateway.clone(),
                    Arc::new(DefaultSubAgentFactory {
                        main_store: self.main_store.clone(),
                        chat_state: self.chat_state.clone(),
                        gateway: self.gateway.clone(),
                        app_data_dir: self.app_data_dir.clone(),
                        tsid_generator: self.tsid_generator.clone(),
                    }),
                    agent_config,
                    runtime_allowed_paths,
                    self.app_data_dir.clone(),
                    Some(subagent_type.to_string()),
                    Some(signal_rx),
                    self.tsid_generator.clone(),
                    self.chat_state.tool_manager.clone(),
                    policy,
                ),
            )))
        }
    }
}

/// Task tool for spawning autonomous sub-agents (Full Spec Clone)
pub struct TaskTool {
    executor_factory: Arc<dyn SubAgentFactory>,
    main_store: Arc<std::sync::RwLock<MainStore>>,
    gateway: Arc<dyn Gateway>,
    tsid_generator: Arc<crate::libs::tsid::TsidGenerator>,
    parent_session_id: Option<String>,
    child_agents: Vec<Agent>,
}

impl TaskTool {
    pub fn new(
        factory: Arc<dyn SubAgentFactory>,
        main_store: Arc<std::sync::RwLock<MainStore>>,
        gateway: Arc<dyn Gateway>,
        tsid_generator: Arc<crate::libs::tsid::TsidGenerator>,
    ) -> Self {
        Self {
            executor_factory: factory,
            main_store,
            gateway,
            tsid_generator,
            parent_session_id: None,
            child_agents: Vec::new(),
        }
    }

    pub fn with_parent_session(mut self, parent_session_id: String) -> Self {
        self.parent_session_id = Some(parent_session_id);
        self
    }

    pub fn with_child_agents(mut self, child_agents: Vec<Agent>) -> Self {
        self.child_agents = child_agents;
        self
    }
}

#[async_trait]
impl ToolDefinition for TaskTool {
    fn name(&self) -> &str {
        crate::tools::TOOL_SUB_AGENT_RUN
    }

    fn description(&self) -> &str {
        "Launch one of the pre-configured child agents owned by the current primary agent. \
        Each child agent has its own prompt, model setup, and tool permissions. \
        Prefer child_agent_name when selecting the child agent; use child_agent_id only as a fallback. \
        The prompt must contain a clear objective, the exact scope to investigate or implement, relevant constraints, \
        and the expected output format or success criteria. \
        If the child agent must return structured findings, explicitly state which facts, files, risks, or conclusions must be included. \
        execution_mode defaults to 'call'. Use execution_mode='call' when the parent cannot continue until the child finishes; the parent pauses and receives the result automatically. \
        Use execution_mode='background' when the child can run in parallel while the parent continues other work; completion will be reported automatically, and sub_agent_output can be used later if the result is explicitly needed."
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::System
    }

    fn scope(&self) -> crate::tools::ToolScope {
        crate::tools::ToolScope::Workflow
    }

    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        let child_agent_ids: Vec<String> = self
            .child_agents
            .iter()
            .map(|agent| agent.id.clone())
            .collect();
        let child_agent_names: Vec<String> = self
            .child_agents
            .iter()
            .map(|agent| agent.name.clone())
            .collect();
        let child_agent_descriptions = self
            .child_agents
            .iter()
            .map(|agent| {
                let description = agent
                    .description
                    .as_deref()
                    .unwrap_or("No description provided");
                format!("{}: {} ({})", agent.id, agent.name, description)
            })
            .collect::<Vec<_>>()
            .join("\n");

        MCPToolDeclaration {
            name: self.name().to_string(),
            description: if child_agent_descriptions.is_empty() {
                format!(
                    "{}\n\nNo child agents are currently configured for this primary agent.",
                    self.description()
                )
            } else {
                format!(
                    "{}\n\nAvailable child agents:\n{}",
                    self.description(),
                    child_agent_descriptions
                )
            },
            input_schema: json!({
                "type": "object",
                "properties": {
                    "description": { "type": "string", "description": "A short (3-5 word) description of the task" },
                    "prompt": { "type": "string", "description": "A complete delegation brief for the child agent. Include the objective, exact scope, constraints, relevant context, and what the final output must contain." },
                    "child_agent_name": {
                        "type": "string",
                        "enum": child_agent_names,
                        "description": "Preferred selector. Use the exact displayed child agent name instead of manually copying an opaque id."
                    },
                    "child_agent_id": {
                        "type": "string",
                        "enum": child_agent_ids,
                        "description": "Fallback selector. Use only if child_agent_name is unavailable."
                    },
                    "execution_mode": {
                        "type": "string",
                        "enum": ["call", "background"],
                        "default": "call",
                        "description": "Execution mode for the child agent. Use 'call' if you must wait for the child to finish before continuing; the parent workflow will pause and resume with the final child result. Use 'background' if you can continue other work in parallel; the system will report completion automatically, and you may inspect the result later with sub_agent_output when needed."
                    }
                },
                "required": ["description", "prompt"],
                "anyOf": [
                    { "required": ["child_agent_name"] },
                    { "required": ["child_agent_id"] }
                ]
            }),
            output_schema: None,
            disabled: false,
            scope: Some(self.scope()),
        }
    }

    async fn call(&self, params: Value) -> NativeToolResult {
        let description = params["description"].as_str().unwrap_or("sub-task");
        let prompt = params["prompt"]
            .as_str()
            .ok_or(ToolError::InvalidParams("prompt is required".to_string()))?;
        let execution_mode = params["execution_mode"].as_str().unwrap_or("call");
        if !matches!(execution_mode, "call" | "background") {
            return Err(ToolError::InvalidParams(format!(
                "execution_mode must be either 'call' or 'background', got '{}'",
                execution_mode
            )));
        }

        let available_child_agents = self
            .child_agents
            .iter()
            .map(|agent| format!("{} ({})", agent.name, agent.id))
            .collect::<Vec<_>>()
            .join(", ");

        let child_agent = if let Some(child_agent_name) = params["child_agent_name"].as_str() {
            self.child_agents
                .iter()
                .find(|agent| agent.name == child_agent_name)
                .cloned()
                .ok_or_else(|| {
                    ToolError::InvalidParams(format!(
                        "child_agent_name '{}' is not available to the current agent. Available child agents: {}",
                        child_agent_name, available_child_agents
                    ))
                })?
        } else if let Some(child_agent_id) = params["child_agent_id"].as_str() {
            self.child_agents
                .iter()
                .find(|agent| agent.id == child_agent_id)
                .cloned()
                .ok_or_else(|| {
                    ToolError::InvalidParams(format!(
                        "child_agent_id '{}' is not available to the current agent. Available child agents: {}",
                        child_agent_id, available_child_agents
                    ))
                })?
        } else {
            return Err(ToolError::InvalidParams(format!(
                "Either child_agent_name or child_agent_id is required. Available child agents: {}",
                available_child_agents
            )));
        };

        // Use TSID for unique time-sorted IDs
        let task_id = format!(
            "subagent_{}",
            self.tsid_generator
                .generate()
                .map_err(|e| ToolError::ExecutionFailed(e))?
        );

        let sub_executor = self
            .executor_factory
            .create_executor(
                &child_agent.id,
                &task_id,
                prompt,
                &child_agent.name,
                self.parent_session_id.as_deref(),
            )
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to create sub-executor: {}", e))
            })?;

        {
            let mut guard = sub_executor.lock().await;
            guard
                .init()
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("Sub-agent init failed: {}", e)))?;
        }

        if execution_mode == "background" {
            if let Some(parent_session_id) = self.parent_session_id.as_ref() {
                if let Err(e) = append_sub_agent_event(
                    &self.main_store,
                    WorkflowEvent::sub_agent_started(
                        parent_session_id.clone(),
                        task_id.clone(),
                        "background".to_string(),
                    ),
                ) {
                    log::warn!(
                        "[Workflow][session={}][parent={}][phase=sub_agent_start] Failed to persist background sub-agent event: {}",
                        task_id,
                        parent_session_id,
                        e
                    );
                }
            }
            let exec_clone = sub_executor.clone();
            let task_id_clone = task_id.clone();
            let owner_session_id = self.parent_session_id.clone();
            let main_store = self.main_store.clone();
            let gateway = self.gateway.clone();
            BACKGROUND_TASKS.insert(
                task_id.clone(),
                BackgroundTask::SubAgent {
                    owner_session_id: self.parent_session_id.clone(),
                    executor: sub_executor,
                    output_accessible: true,
                },
            );

            tokio::spawn(async move {
                let mut guard = exec_clone.lock().await;
                let result = guard.run_loop().await;
                let final_state = guard.state();
                let messages = guard.messages();
                if let Err(error) = &result {
                    log::error!("Background task {} failed: {}", guard.session_id(), error);
                }
                let completion_result = build_terminal_sub_agent_result(
                    &task_id_clone,
                    final_state.clone(),
                    &result,
                    &messages,
                );
                let output = render_task_output(&completion_result);
                remember_completed_task(
                    task_id_clone.clone(),
                    owner_session_id.clone(),
                    output,
                    true,
                );
                if let Some(parent_session_id) = owner_session_id.clone() {
                    let completion = build_sub_agent_completion(
                        &parent_session_id,
                        &task_id_clone,
                        &completion_result,
                    );
                    if let Err(e) = persist_sub_agent_completion(&main_store, completion) {
                        log::warn!(
                            "[Workflow][session={}][parent={}][phase=sub_agent_completion] Failed to persist background sub-agent completion: {}",
                            task_id_clone,
                            parent_session_id,
                            e
                        );
                    }
                    if !has_pending_task_output_wait(&task_id_clone) {
                        let _ = gateway
                            .send(
                                &parent_session_id,
                                crate::workflow::react::types::GatewayPayload::Notification {
                                    message: format!(
                                        "Sub-agent {} has completed. If you have not already retrieved the result, use sub_agent_output with task_id {}. If you already received it, do not call sub_agent_output again.",
                                        task_id_clone, task_id_clone
                                    ),
                                    category: Some("info".to_string()),
                                },
                            )
                            .await;
                    }
                }
                TASK_OUTPUT_THROTTLE.remove(&task_id_clone);
                crate::workflow::react::manager::WorkflowManager::unregister_session_signal_tx(
                    &task_id_clone,
                );
                BACKGROUND_TASKS.remove(&task_id_clone);
            });

            return Ok(ToolCallResult::success(Some(json!({
                "task_id": task_id,
                "status": "Running",
                "agent_name": child_agent.name,
                "task": prompt,
                "message": format!("Sub-agent '{}' has been started in the background. Use 'sub_agent_output' with the task_id to retrieve results later.", description)
            }).to_string()), Some(json!({
                "task_id": task_id,
                "status": "running",
                "agent_name": child_agent.name,
                "task": prompt,
                "mode": "background"
            }))));
        }

        let parent_id = self.parent_session_id.as_ref().ok_or_else(|| {
            ToolError::ExecutionFailed(
                "sub_agent_run execution_mode='call' requires an active parent session".to_string(),
            )
        })?;

        get_sub_agent_registry().register_sub_agent(task_id.clone(), parent_id.clone());
        if let Err(e) = append_sub_agent_event(
            &self.main_store,
            WorkflowEvent::sub_agent_started(
                parent_id.clone(),
                task_id.clone(),
                "call".to_string(),
            ),
        ) {
            log::warn!(
                "[Workflow][session={}][parent={}][phase=sub_agent_start] Failed to persist sub-agent event: {}",
                task_id,
                parent_id,
                e
            );
        }

        let exec_clone = sub_executor.clone();
        BACKGROUND_TASKS.insert(
            task_id.clone(),
            BackgroundTask::SubAgent {
                owner_session_id: Some(parent_id.clone()),
                executor: sub_executor,
                output_accessible: false,
            },
        );

        let parent_id = parent_id.clone();
        let task_id_clone = task_id.clone();
        let main_store = self.main_store.clone();
        tokio::spawn(async move {
            let mut guard = exec_clone.lock().await;
            let result = guard.run_loop().await;
            let final_state = guard.state();
            let messages = guard.messages();
            let completion_result = build_terminal_sub_agent_result(
                &task_id_clone,
                final_state.clone(),
                &result,
                &messages,
            );

            let completion_status = completion_result
                .get("status")
                .and_then(|value| value.as_str())
                .unwrap_or("completed")
                .to_string();
            if let Err(e) = append_sub_agent_event(
                &main_store,
                WorkflowEvent::sub_agent_completed(
                    parent_id.clone(),
                    task_id_clone.clone(),
                    completion_status,
                    completion_result.clone(),
                ),
            ) {
                log::warn!(
                    "[Workflow][session={}][parent={}][phase=sub_agent_completion] Failed to persist sub-agent completion event: {}",
                    task_id_clone,
                    parent_id,
                    e
                );
            }

            let completion =
                build_sub_agent_completion(&parent_id, &task_id_clone, &completion_result);
            if let Err(e) = persist_sub_agent_completion(&main_store, completion) {
                log::error!(
                    "[Workflow][session={}][parent={}][phase=sub_agent_completion] Failed to persist sub-agent completion: {}",
                    task_id_clone,
                    parent_id,
                    e
                );
            }

            if let Err(e) = crate::workflow::react::manager::WorkflowManager::send_signal_to_session(
                &parent_id,
                json!({
                    "type": "sub_agent_complete",
                    "sub_agent_id": task_id_clone,
                    "result": completion_result
                })
                .to_string(),
            ) {
                log::warn!(
                    "[Workflow][session={}][parent={}][phase=sub_agent_completion] Live signal delivery failed; durable completion will be replayed on recovery: {}",
                    task_id_clone,
                    parent_id,
                    e
                );
            }

            get_sub_agent_registry().unregister_sub_agent(&task_id_clone);
            TASK_OUTPUT_THROTTLE.remove(&task_id_clone);
            crate::workflow::react::manager::WorkflowManager::unregister_session_signal_tx(
                &task_id_clone,
            );
            BACKGROUND_TASKS.remove(&task_id_clone);
        });

        Ok(ToolCallResult::success(Some(json!({
            "status": "waiting",
            "task_id": task_id,
            "agent_name": child_agent.name,
            "task": prompt,
            "message": format!("Task '{}' has been spawned. Parent workflow will wait for completion.", description)
        }).to_string()), Some(json!({
            "status": "waiting",
            "task_id": task_id,
            "agent_name": child_agent.name,
            "task": prompt,
            "mode": "call"
        }))))
    }
}

/// Tool to retrieve results from background tasks (Full Spec Clone)
pub struct TaskOutputTool {
    session_id: String,
    main_store: Arc<std::sync::RwLock<MainStore>>,
}

impl TaskOutputTool {
    pub fn new(session_id: String, main_store: Arc<std::sync::RwLock<MainStore>>) -> Self {
        Self {
            session_id,
            main_store,
        }
    }
}

#[async_trait]
impl ToolDefinition for TaskOutputTool {
    fn name(&self) -> &str {
        crate::tools::TOOL_SUB_AGENT_OUTPUT
    }

    fn description(&self) -> &str {
        "- Retrieves output from a running or completed sub-agent\n\
        - Takes a task_id parameter identifying the sub-agent\n\
        - Background sub-agents can be checked while running; call-mode sub-agents normally deliver results automatically, but this tool can return the already-delivered result again if needed\n\
        - Returns the latest sub-agent output along with status information\n\
        - Set wait_until_complete=true only when the next step depends on the final child result; the tool will wait until the sub-agent finishes, fails, or is stopped\n\
        - Leave wait_until_complete=false when you only need a non-blocking status check. Do not poll running sub-agents repeatedly; continue other work or wait for the automatic completion notification."
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::System
    }

    fn scope(&self) -> crate::tools::ToolScope {
        crate::tools::ToolScope::Workflow
    }

    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string", "description": "The sub-agent ID to get output from" },
                    "wait_until_complete": {
                        "type": "boolean",
                        "description": "When true, wait until the sub-agent reaches a terminal state and return only the final result. Use this if you cannot continue until the child agent finishes. When false or omitted, return the current status immediately; use this for non-blocking checks while continuing other work."
                    }
                },
                "required": ["task_id"]
            }),
            output_schema: None,
            disabled: false,
            scope: Some(self.scope()),
        }
    }

    async fn call(&self, params: Value) -> NativeToolResult {
        struct TaskOutputWaitGuard<'a> {
            task_id: &'a str,
        }

        impl Drop for TaskOutputWaitGuard<'_> {
            fn drop(&mut self) {
                unregister_task_output_wait(self.task_id);
            }
        }

        let task_id = params["task_id"]
            .as_str()
            .ok_or(ToolError::InvalidParams("task_id required".into()))?;
        let wait_until_complete = params["wait_until_complete"].as_bool().unwrap_or(false);
        validate_task_access(&self.session_id, task_id)?;

        if wait_until_complete {
            register_task_output_wait(task_id);
            let _wait_guard = TaskOutputWaitGuard { task_id };
            loop {
                if let Some(snapshot) = COMPLETED_BACKGROUND_TASKS.get(task_id) {
                    let output = snapshot.output.clone();
                    drop(snapshot);
                    mark_completed_task_consumed(task_id);
                    let _ = sync_sub_agent_completion_consumed(
                        &self.main_store,
                        &self.session_id,
                        task_id,
                    );
                    return Ok(ToolCallResult::success(Some(output), None));
                }

                let Some(task) = BACKGROUND_TASKS.get(task_id) else {
                    break;
                };

                match task.value() {
                    BackgroundTask::SubAgent {
                        executor,
                        output_accessible,
                        ..
                    } => {
                        if let Ok(Some(completion)) = find_durable_sub_agent_completion(
                            &self.main_store,
                            &self.session_id,
                            task_id,
                        ) {
                            let delivered = completion
                                .result
                                .as_deref()
                                .or(completion.summary.as_deref())
                                .or(completion.error.as_deref())
                                .unwrap_or("Sub-agent completed")
                                .to_string();
                            let _ = sync_sub_agent_completion_consumed(
                                &self.main_store,
                                &self.session_id,
                                task_id,
                            );
                            mark_completed_task_consumed(task_id);
                            let content = if *output_accessible {
                                delivered
                            } else {
                                render_call_mode_sub_agent_tool_result(
                                    task_id,
                                    &completion.status,
                                    None,
                                    &delivered,
                                    None,
                                    false,
                                )
                            };
                            return Ok(ToolCallResult::success(Some(content), None));
                        }

                        let executor = executor.clone();
                        drop(task);
                        let guard = executor.lock().await;
                        let state = guard.state();
                        if matches!(
                            state,
                            crate::workflow::react::types::WorkflowState::Completed
                                | crate::workflow::react::types::WorkflowState::Error
                                | crate::workflow::react::types::WorkflowState::Cancelled
                        ) {
                            drop(guard);
                            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                            continue;
                        }
                    }
                }

                tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
            }
        }

        if let Some(task) = BACKGROUND_TASKS.get(task_id) {
            return match task.value() {
                BackgroundTask::SubAgent { executor, .. } => {
                    let guard = executor.lock().await;
                    let state = guard.state();
                    let is_terminal = matches!(
                        state,
                        crate::workflow::react::types::WorkflowState::Completed
                            | crate::workflow::react::types::WorkflowState::Error
                            | crate::workflow::react::types::WorkflowState::Cancelled
                    );
                    if !is_terminal {
                        let now = chrono::Utc::now().timestamp_millis();
                        if let Some(cached) = TASK_OUTPUT_THROTTLE.get(task_id) {
                            if now - cached.0 < 15_000 {
                                return Ok(ToolCallResult::success(
                                    Some(format!(
                                        "{}\n<SYSTEM_REMINDER>This sub-agent is still running. Do not poll it repeatedly; continue other work or wait for the completion event.</SYSTEM_REMINDER>",
                                        cached.1
                                    )),
                                    None,
                                ));
                            }
                        }
                    }

                    let mut result = format!("Status: {:?}. ", state);
                    if let Some(last_msg) = guard
                        .messages()
                        .iter()
                        .rev()
                        .find(|m| m.role == "assistant")
                    {
                        result.push_str("Latest Output: ");
                        result.push_str(&last_msg.message);
                    }
                    if !is_terminal {
                        TASK_OUTPUT_THROTTLE.insert(
                            task_id.to_string(),
                            (chrono::Utc::now().timestamp_millis(), result.clone()),
                        );
                    }
                    Ok(ToolCallResult::success(Some(result), None))
                }
            };
        }

        if let Some(snapshot) = COMPLETED_BACKGROUND_TASKS.get(task_id) {
            let output = snapshot.output.clone();
            drop(snapshot);
            mark_completed_task_consumed(task_id);
            let _ = sync_sub_agent_completion_consumed(&self.main_store, &self.session_id, task_id);
            return Ok(ToolCallResult::success(Some(output), None));
        }

        if let Ok(Some(completion)) =
            find_durable_sub_agent_completion(&self.main_store, &self.session_id, task_id)
        {
            let delivered = completion
                .result
                .as_deref()
                .or(completion.summary.as_deref())
                .or(completion.error.as_deref())
                .unwrap_or("Sub-agent completed")
                .to_string();
            let _ = sync_sub_agent_completion_consumed(&self.main_store, &self.session_id, task_id);
            mark_completed_task_consumed(task_id);
            return Ok(ToolCallResult::success(
                Some(render_call_mode_sub_agent_tool_result(
                    task_id,
                    &completion.status,
                    None,
                    &delivered,
                    None,
                    false,
                )),
                Some(json!({
                    "task_id": task_id,
                    "status": completion.status,
                    "result": delivered,
                    "mode": "call",
                    "already_delivered": true
                })),
            ));
        }

        let available_tasks = format_available_task_ids_for_owner(&self.session_id);
        Err(ToolError::ExecutionFailed(format!(
            "Sub-agent {} not found in active, completed, or durable sub-agent records for this session.\n<SYSTEM_REMINDER>sub_agent_output retrieves accessible background sub-agent outputs and can re-read durable call-mode completions only when they exist for this parent session. If this task was launched with execution_mode='call', its result is normally delivered directly to the parent workflow as a sub-agent completion observation. Use any delivered observation you already have; otherwise continue without polling this task_id. {} Do not use sub_agent_output as a generic final-answer tool or with a main session ID.</SYSTEM_REMINDER>",
            task_id,
            available_tasks
        )))
    }
}

/// Tool to stop a background agent (Full Spec Clone)
pub struct TaskStopTool {
    session_id: String,
}

impl TaskStopTool {
    pub fn new(session_id: String) -> Self {
        Self { session_id }
    }
}

#[async_trait]
impl ToolDefinition for TaskStopTool {
    fn name(&self) -> &str {
        crate::tools::TOOL_SUB_AGENT_STOP
    }

    fn description(&self) -> &str {
        "- Stops a currently running sub-agent owned by this workflow by its task_id\n\
        - Works for active sub-agents that are still registered in this workflow, including background runs and any still-running child sessions\n\
        - Takes a task_id parameter identifying the sub-agent to stop\n\
        - Returns success only when the stop signal was issued to an active sub-agent\n\
        - Use this tool when the sub-agent is no longer needed, is stuck, or is taking too long. Do not use it for already completed tasks."
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::System
    }

    fn scope(&self) -> crate::tools::ToolScope {
        crate::tools::ToolScope::Workflow
    }

    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string", "description": "The ID of the background sub-agent to stop" }
                },
                "required": ["task_id"]
            }),
            output_schema: None,
            disabled: false,
            scope: Some(self.scope()),
        }
    }

    async fn call(&self, params: Value) -> NativeToolResult {
        let task_id = params["task_id"]
            .as_str()
            .ok_or(ToolError::InvalidParams("task_id required".into()))?;
        validate_task_access(&self.session_id, task_id)?;
        if stop_background_task(task_id, None).await {
            Ok(ToolCallResult::success(
                Some(format!("Sub-agent {} has been terminated.", task_id)),
                None,
            ))
        } else {
            Err(ToolError::ExecutionFailed(format!(
                "Sub-agent {} not found",
                task_id
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        remember_completed_task, stop_background_task, BackgroundTask, CompletedTaskSnapshot,
        DefaultSubAgentFactory, SubAgentFactory, TaskOutputTool, TaskTool, BACKGROUND_TASKS,
        COMPLETED_BACKGROUND_TASKS,
    };
    use crate::ai::interaction::chat_completion::ChatState;
    use crate::db::{AgentConfig, MainStore, WorkflowMessage};
    use crate::libs::tsid::TsidGenerator;
    use crate::libs::window_channels::WindowChannels;
    use crate::tools::ToolDefinition;
    use crate::workflow::react::engine::ReActExecutor;
    use crate::workflow::react::error::WorkflowEngineError;
    use crate::workflow::react::gateway::Gateway;
    use crate::workflow::react::manager::WorkflowManager;
    use crate::workflow::react::types::{
        ExecutionContext, GatewayPayload, RuntimeState, StepType, SubAgentCompletion, WorkflowState,
    };
    use async_trait::async_trait;
    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::{Arc, RwLock};
    use tokio::sync::{mpsc, Mutex};

    struct NoopGateway;

    #[async_trait]
    impl Gateway for NoopGateway {
        async fn send(
            &self,
            _session_id: &str,
            _payload: GatewayPayload,
        ) -> Result<(), WorkflowEngineError> {
            Ok(())
        }

        async fn inject_input(
            &self,
            _session_id: &str,
            _input: String,
        ) -> Result<(), WorkflowEngineError> {
            Ok(())
        }
    }

    struct DiagnosticMockExecutor {
        session_id: String,
        state: WorkflowState,
        messages: Vec<WorkflowMessage>,
    }

    impl DiagnosticMockExecutor {
        fn new(session_id: String) -> Self {
            Self {
                session_id: session_id.clone(),
                state: WorkflowState::Pending,
                messages: vec![WorkflowMessage {
                    id: None,
                    session_id,
                    role: "assistant".to_string(),
                    message: "mock Code Browse scan completed".to_string(),
                    reasoning: None,
                    message_kind: "message".to_string(),
                    message_subtype: None,
                    segment_id: 1,
                    source_event_type: None,
                    metadata: None,
                    attached_context: None,
                    step_type: None,
                    step_index: 0,
                    is_error: false,
                    error_type: None,
                    created_at: None,
                }],
            }
        }
    }

    #[async_trait]
    impl ReActExecutor for DiagnosticMockExecutor {
        async fn init(&mut self) -> Result<(), WorkflowEngineError> {
            Ok(())
        }

        async fn run_loop(&mut self) -> Result<(), WorkflowEngineError> {
            self.state = WorkflowState::Completed;
            Ok(())
        }

        async fn begin_new_context_segment(&mut self) -> Result<(), WorkflowEngineError> {
            Ok(())
        }

        async fn begin_execution_context_from_approved_plan(
            &mut self,
        ) -> Result<(), WorkflowEngineError> {
            Ok(())
        }

        async fn add_message_and_notify(
            &mut self,
            _role: String,
            _content: String,
            _attached_context: Option<String>,
            _reasoning: Option<String>,
            _step_type: Option<StepType>,
            _is_error: bool,
            _error_type: Option<String>,
            _metadata: Option<serde_json::Value>,
        ) -> Result<bool, WorkflowEngineError> {
            Ok(true)
        }

        fn session_id(&self) -> String {
            self.session_id.clone()
        }

        fn state(&self) -> WorkflowState {
            self.state.clone()
        }

        fn set_state(&mut self, state: WorkflowState) {
            self.state = state;
        }

        fn messages(&self) -> Vec<WorkflowMessage> {
            self.messages.clone()
        }
    }

    #[derive(Default)]
    struct CapturedSubAgentCall {
        agent_id: String,
        session_id: String,
        task: String,
        subagent_type: String,
        parent_session_id: Option<String>,
    }

    struct DiagnosticMockFactory {
        captured: Arc<Mutex<Option<CapturedSubAgentCall>>>,
    }

    #[async_trait]
    impl SubAgentFactory for DiagnosticMockFactory {
        async fn create_executor(
            &self,
            agent_id: &str,
            session_id: &str,
            task: &str,
            subagent_type: &str,
            parent_session_id: Option<&str>,
        ) -> Result<Arc<Mutex<dyn ReActExecutor>>, WorkflowEngineError> {
            *self.captured.lock().await = Some(CapturedSubAgentCall {
                agent_id: agent_id.to_string(),
                session_id: session_id.to_string(),
                task: task.to_string(),
                subagent_type: subagent_type.to_string(),
                parent_session_id: parent_session_id.map(str::to_string),
            });

            Ok(Arc::new(Mutex::new(DiagnosticMockExecutor::new(
                session_id.to_string(),
            ))))
        }
    }

    fn test_store() -> (tempfile::TempDir, Arc<RwLock<MainStore>>) {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let db_path = dir.path().join("orchestrator_diagnostic_test.db");
        let store = MainStore::new(&db_path).expect("failed to create MainStore");
        (dir, Arc::new(RwLock::new(store)))
    }

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("src-tauri should have a parent repo directory")
            .to_path_buf()
    }

    #[tokio::test]
    async fn sub_agent_output_reads_completed_snapshot() {
        let task_id = "subagent_test_completed_snapshot";
        COMPLETED_BACKGROUND_TASKS.remove(task_id);
        remember_completed_task(task_id, Some("session_a".to_string()), "done", true);

        let (_dir, store) = test_store();
        let tool = TaskOutputTool::new("session_a".to_string(), store);
        let result = tool
            .call(json!({ "task_id": task_id }))
            .await
            .expect("sub_agent_output should read completed snapshot");

        let content = result.content.unwrap_or_default();
        assert_eq!(content, "done");

        COMPLETED_BACKGROUND_TASKS.remove(task_id);
    }

    #[tokio::test]
    async fn sub_agent_output_wait_until_complete_marks_result_consumed() {
        let task_id = "subagent_test_consumed_snapshot";
        COMPLETED_BACKGROUND_TASKS.remove(task_id);
        let (_dir, store) = test_store();
        {
            let store_guard = store.read().expect("store lock");
            store_guard
                .upsert_execution_context(&ExecutionContext {
                    session_id: "session_a".to_string(),
                    state: RuntimeState::Running,
                    wait_reason: None,
                    current_step: 0,
                    max_steps: 0,
                    pending_tools: Vec::new(),
                    last_action_summary: None,
                    current_context_tokens: None,
                    max_context_tokens: None,
                    last_event_id: None,
                    version: ExecutionContext::CURRENT_VERSION.to_string(),
                    waiting_on_sub_agent_id: None,
                    sub_agent_sessions: vec![task_id.to_string()],
                    pending_sub_agent_completions: vec![SubAgentCompletion {
                        sub_agent_id: task_id.to_string(),
                        parent_session_id: "session_a".to_string(),
                        status: "completed".to_string(),
                        result: Some("done".to_string()),
                        summary: Some("done".to_string()),
                        error: None,
                        tool_calls_count: 1,
                        completed_at_ms: chrono::Utc::now().timestamp_millis(),
                        consumed: false,
                    }],
                })
                .expect("failed to upsert execution context");
        }

        remember_completed_task(task_id, Some("session_a".to_string()), "done", true);

        let tool = TaskOutputTool::new("session_a".to_string(), store.clone());
        let result = tool
            .call(json!({ "task_id": task_id, "wait_until_complete": true }))
            .await
            .expect("sub_agent_output should read and consume completed snapshot");

        let content = result.content.unwrap_or_default();
        assert_eq!(content, "done");

        let snapshot_consumed = COMPLETED_BACKGROUND_TASKS
            .get(task_id)
            .map(|entry| entry.consumed)
            .unwrap_or(false);
        assert!(snapshot_consumed);

        let context_consumed = {
            let store_guard = store.read().expect("store lock");
            store_guard
                .get_execution_context("session_a")
                .expect("failed to get execution context")
                .and_then(|ctx| {
                    ctx.pending_sub_agent_completions
                        .into_iter()
                        .find(|completion| completion.sub_agent_id == task_id)
                })
                .map(|completion| completion.consumed)
                .unwrap_or(false)
        };
        assert!(context_consumed);

        COMPLETED_BACKGROUND_TASKS.remove(task_id);
    }

    #[test]
    fn completed_snapshot_overwrites_latest_terminal_state() {
        let task_id = "subagent_test_snapshot_overwrite";
        COMPLETED_BACKGROUND_TASKS.remove(task_id);

        remember_completed_task(task_id, Some("session_a".to_string()), "first", true);
        remember_completed_task(task_id, Some("session_a".to_string()), "second", true);

        let snapshot = COMPLETED_BACKGROUND_TASKS
            .get(task_id)
            .map(|entry| entry.output.clone());
        let snapshot = snapshot.expect("snapshot should exist");
        assert_eq!(snapshot, "second");

        COMPLETED_BACKGROUND_TASKS.remove(task_id);
    }

    #[tokio::test]
    async fn sub_agent_output_rejects_non_subagent_prefix() {
        let (_dir, store) = test_store();
        let tool = TaskOutputTool::new("session_a".to_string(), store);
        let error = tool
            .call(json!({ "task_id": "0q508dwjw0400" }))
            .await
            .expect_err("non-subagent session ids should be rejected");

        assert!(error.to_string().contains("task_id must be a sub-agent id"));
    }

    #[tokio::test]
    async fn sub_agent_output_rejects_other_session_snapshot() {
        let task_id = "subagent_test_other_session";
        COMPLETED_BACKGROUND_TASKS.remove(task_id);
        remember_completed_task(
            task_id,
            Some("session_b".to_string()),
            "Status: completed. Latest Output: hidden",
            true,
        );

        let (_dir, store) = test_store();
        let tool = TaskOutputTool::new("session_a".to_string(), store);
        let error = tool
            .call(json!({ "task_id": task_id }))
            .await
            .expect_err("cross-session access should be rejected");

        assert!(error
            .to_string()
            .contains("is not accessible from the current session"));

        COMPLETED_BACKGROUND_TASKS.remove(task_id);
    }

    #[tokio::test]
    async fn sub_agent_output_reads_call_mode_snapshot() {
        let task_id = "subagent_test_call_mode_snapshot";
        COMPLETED_BACKGROUND_TASKS.remove(task_id);
        COMPLETED_BACKGROUND_TASKS.insert(
            task_id.to_string(),
            CompletedTaskSnapshot {
                owner_session_id: Some("session_a".to_string()),
                output: "done".to_string(),
                output_accessible: false,
                consumed: false,
            },
        );

        let (_dir, store) = test_store();
        let tool = TaskOutputTool::new("session_a".to_string(), store);
        let result = tool
            .call(json!({ "task_id": task_id }))
            .await
            .expect("call-mode snapshot should be readable");

        assert_eq!(result.content.as_deref(), Some("done"));

        COMPLETED_BACKGROUND_TASKS.remove(task_id);
    }

    #[tokio::test]
    async fn sub_agent_output_reads_durable_call_mode_completion() {
        let task_id = "subagent_test_durable_call_mode";
        COMPLETED_BACKGROUND_TASKS.remove(task_id);
        let (_dir, store) = test_store();
        {
            let store_guard = store.read().expect("store lock");
            store_guard
                .upsert_execution_context(&ExecutionContext {
                    session_id: "session_a".to_string(),
                    state: RuntimeState::Running,
                    wait_reason: None,
                    current_step: 0,
                    max_steps: 0,
                    pending_tools: Vec::new(),
                    last_action_summary: None,
                    current_context_tokens: None,
                    max_context_tokens: None,
                    last_event_id: None,
                    version: ExecutionContext::CURRENT_VERSION.to_string(),
                    waiting_on_sub_agent_id: None,
                    sub_agent_sessions: Vec::new(),
                    pending_sub_agent_completions: vec![SubAgentCompletion {
                        sub_agent_id: task_id.to_string(),
                        parent_session_id: "session_a".to_string(),
                        status: "completed".to_string(),
                        result: Some("OCR findings".to_string()),
                        summary: Some("OCR summary".to_string()),
                        error: None,
                        tool_calls_count: 2,
                        completed_at_ms: chrono::Utc::now().timestamp_millis(),
                        consumed: false,
                    }],
                })
                .expect("failed to upsert execution context");
        }

        let tool = TaskOutputTool::new("session_a".to_string(), store);
        let result = tool
            .call(json!({ "task_id": task_id }))
            .await
            .expect("durable call-mode completion should be readable");

        let message = result.content.unwrap_or_default();
        assert!(message.contains("<tool_result"));
        assert!(message.contains("mode=\"call\""));
        assert!(message.contains("OCR findings"));
    }

    #[tokio::test]
    async fn stop_background_task_sends_stop_and_preserves_owner() {
        let task_id = "subagent_test_stop_signal";
        COMPLETED_BACKGROUND_TASKS.remove(task_id);
        BACKGROUND_TASKS.remove(task_id);
        WorkflowManager::unregister_session_signal_tx(task_id);

        let (tx, mut rx) = mpsc::channel(1);
        WorkflowManager::register_session_signal_tx(task_id.to_string(), tx);
        BACKGROUND_TASKS.insert(
            task_id.to_string(),
            BackgroundTask::SubAgent {
                owner_session_id: Some("session_a".to_string()),
                executor: Arc::new(Mutex::new(DiagnosticMockExecutor::new(task_id.to_string()))),
                output_accessible: true,
            },
        );

        assert!(stop_background_task(task_id, None).await);

        let signal = tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv())
            .await
            .expect("stop signal should be delivered before timeout")
            .expect("stop signal channel should yield a message");
        assert_eq!(signal, serde_json::json!({ "type": "stop" }).to_string());

        let snapshot = COMPLETED_BACKGROUND_TASKS
            .get(task_id)
            .expect("completed snapshot should be retained");
        assert_eq!(snapshot.owner_session_id.as_deref(), Some("session_a"));
        assert!(snapshot.output_accessible);

        COMPLETED_BACKGROUND_TASKS.remove(task_id);
        BACKGROUND_TASKS.remove(task_id);
        WorkflowManager::unregister_session_signal_tx(task_id);
    }

    #[tokio::test]
    async fn sub_agent_run_invokes_code_browse_for_module_scan() {
        let (_dir, store) = test_store();
        let parent_agent = crate::db::Agent::new(
            "diagnostic-parent-agent".to_string(),
            "Diagnostic Parent Agent".to_string(),
            Some("Parent agent fixture for sub-agent diagnostics".to_string()),
            Some("primary".to_string()),
            None,
            "Parent diagnostic system prompt".to_string(),
            None,
            None,
            None,
            None,
            None,
            None,
            Some(false),
            None,
            Some(false),
            Some(false),
            Some(false),
            None,
        );
        let parent_session_id = "diagnostic_parent_session";
        {
            let store_guard = store.read().expect("store lock");
            store_guard
                .add_agent(&parent_agent)
                .expect("failed to create parent agent");
            store_guard
                .create_workflow(
                    parent_session_id,
                    "diagnostic parent workflow",
                    &parent_agent.id,
                    None,
                    None,
                )
                .expect("failed to create parent workflow");
        }

        let captured = Arc::new(Mutex::new(None));
        let factory: Arc<dyn SubAgentFactory> = Arc::new(DiagnosticMockFactory {
            captured: captured.clone(),
        });
        let tsid_generator = Arc::new(TsidGenerator::new(7).expect("failed to create tsid"));
        let tool = TaskTool::new(factory, store, Arc::new(NoopGateway), tsid_generator)
            .with_parent_session(parent_session_id.to_string())
            .with_child_agents(vec![crate::db::Agent::new(
                "0q0xff0pm0400".to_string(),
                "Code Browse".to_string(),
                Some("Read project information before implementation".to_string()),
                Some("child".to_string()),
                Some("diagnostic-parent-agent".to_string()),
                "You read project modules and report concise findings.".to_string(),
                None,
                None,
                None,
                None,
                None,
                None,
                Some(false),
                None,
                Some(false),
                Some(false),
                Some(false),
                None,
            )]);

        let prompt = "Scan module `src-tauri/src/workflow/react/orchestrator.rs`. \
            Report the key responsibilities, sub_agent_run/sub_agent_output flow, \
            and any obvious risk areas. Do not modify files.";
        let result = tool
            .call(json!({
                "description": "Scan orchestrator module",
                "prompt": prompt,
                "child_agent_name": "Code Browse",
                "execution_mode": "call"
            }))
            .await
            .expect("sub_agent_run should invoke Code Browse");

        let content = result.content.unwrap_or_default();
        assert!(content.contains("\"status\":\"waiting\""));
        assert!(content.contains("\"task_id\":\"subagent_"));
        let task_id = result
            .structured_content
            .as_ref()
            .and_then(|value| value.get("task_id"))
            .and_then(|value| value.as_str())
            .expect("sub_agent_run result should include task_id")
            .to_string();
        assert_eq!(
            result
                .structured_content
                .as_ref()
                .and_then(|value| value.get("agent_name"))
                .and_then(|value| value.as_str()),
            Some("Code Browse")
        );
        assert_eq!(
            result
                .structured_content
                .as_ref()
                .and_then(|value| value.get("task"))
                .and_then(|value| value.as_str()),
            Some(prompt)
        );

        let captured = captured.lock().await;
        let captured = captured.as_ref().expect("factory should be called");
        assert_eq!(captured.agent_id, "0q0xff0pm0400");
        assert!(captured.session_id.starts_with("subagent_"));
        assert_eq!(captured.subagent_type, "Code Browse");
        assert_eq!(
            captured.parent_session_id.as_deref(),
            Some(parent_session_id)
        );
        assert_eq!(captured.task, prompt);

        for _ in 0..20 {
            if COMPLETED_BACKGROUND_TASKS.contains_key(&task_id) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        COMPLETED_BACKGROUND_TASKS.remove(&task_id);
    }

    #[tokio::test]
    async fn default_factory_persists_child_user_message_and_parent_workspace() {
        let (dir, store) = test_store();
        let workspace = dir.path().join("workspace");
        std::fs::create_dir_all(&workspace).expect("failed to create workspace");
        let workspace = workspace
            .canonicalize()
            .expect("failed to canonicalize workspace");

        let parent_agent = crate::db::Agent::new(
            "parent-agent".to_string(),
            "Parent Agent".to_string(),
            None,
            Some("primary".to_string()),
            None,
            "Parent prompt".to_string(),
            None,
            None,
            None,
            None,
            None,
            None,
            Some(false),
            None,
            Some(true),
            Some(false),
            Some(false),
            None,
        );
        let child_agent = crate::db::Agent::new(
            "child-agent".to_string(),
            "Code Browse".to_string(),
            None,
            Some("child".to_string()),
            Some(parent_agent.id.clone()),
            "Child prompt".to_string(),
            None,
            Some(
                serde_json::to_string(&vec![
                    crate::tools::TOOL_READ_FILE,
                    crate::tools::TOOL_BASH,
                    crate::tools::TOOL_SUB_AGENT_RUN,
                ])
                .expect("tools json"),
            ),
            Some(
                serde_json::to_string(&vec![crate::tools::TOOL_READ_FILE, crate::tools::TOOL_BASH])
                    .expect("auto approve json"),
            ),
            None,
            None,
            None,
            Some(false),
            None,
            Some(false),
            Some(false),
            Some(false),
            None,
        );
        let parent_session_id = "parent-session";
        let child_session_id = "subagent_child_test";
        let task = "Inspect the workflow module and report the key risks.";
        let parent_config = AgentConfig {
            allowed_paths: Some(vec![workspace.to_string_lossy().to_string()]),
            ..AgentConfig::default()
        };

        {
            let store_guard = store.read().expect("store lock");
            store_guard
                .add_agent(&parent_agent)
                .expect("failed to add parent agent");
            store_guard
                .add_agent(&child_agent)
                .expect("failed to add child agent");
            store_guard
                .create_workflow(
                    parent_session_id,
                    "Parent workflow",
                    &parent_agent.id,
                    Some(parent_config.to_json()),
                    None,
                )
                .expect("failed to create parent workflow");
        }

        let chat_state = ChatState::new(Arc::new(WindowChannels::new()), None, store.clone());
        let factory = DefaultSubAgentFactory {
            main_store: store.clone(),
            chat_state,
            gateway: Arc::new(NoopGateway),
            app_data_dir: dir.path().to_path_buf(),
            tsid_generator: Arc::new(TsidGenerator::new(7).expect("failed to create tsid")),
        };

        let _executor = factory
            .create_executor(
                &child_agent.id,
                child_session_id,
                task,
                &child_agent.name,
                Some(parent_session_id),
            )
            .await
            .expect("failed to create child executor");

        let snapshot = {
            let store_guard = store.read().expect("store lock");
            store_guard
                .get_workflow_snapshot(child_session_id)
                .expect("failed to read child snapshot")
        };
        assert_eq!(snapshot.messages.len(), 1);
        assert_eq!(snapshot.messages[0].role, "user");
        assert_eq!(snapshot.messages[0].message, task);

        let config = snapshot
            .workflow
            .agent_config
            .as_deref()
            .and_then(AgentConfig::from_json)
            .expect("child workflow config should parse");
        assert_eq!(
            config.allowed_paths,
            Some(vec![workspace.to_string_lossy().to_string()])
        );
        assert_eq!(
            config.available_tools,
            Some(vec![crate::tools::TOOL_READ_FILE.to_string()])
        );
        assert_eq!(
            config.auto_approve,
            Some(vec![crate::tools::TOOL_READ_FILE.to_string()])
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "diagnostic test: uses local dev_data/chatspeed.db and a live model/proxy"]
    async fn diagnostic_run_real_code_browse_sub_agent_for_module_scan() {
        let db_path = std::env::var("CHATSPEED_DIAGNOSTIC_DB")
            .map(PathBuf::from)
            .unwrap_or_else(|_| repo_root().join("dev_data").join("chatspeed.db"));
        let app_data_dir = std::env::var("CHATSPEED_DIAGNOSTIC_APP_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                db_path
                    .parent()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| repo_root().join("dev_data"))
            });
        let timeout_secs = std::env::var("CHATSPEED_DIAGNOSTIC_TIMEOUT_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(120);

        eprintln!("diagnostic db: {}", db_path.display());
        eprintln!("diagnostic app_data_dir: {}", app_data_dir.display());
        eprintln!("diagnostic timeout: {}s", timeout_secs);

        let main_store = Arc::new(RwLock::new(
            MainStore::new(&db_path).expect("failed to open diagnostic MainStore"),
        ));
        let code_browse_agent = {
            let store = main_store.read().expect("store lock");
            store
                .get_agent("0q0xff0pm0400")
                .expect("failed to query Code Browse agent")
                .expect("Code Browse agent 0q0xff0pm0400 not found")
        };
        assert_eq!(code_browse_agent.name, "Code Browse");

        let tsid_generator = Arc::new(TsidGenerator::new(8).expect("failed to create tsid"));
        let parent_agent_id = format!(
            "diagnostic-parent-agent-{}",
            tsid_generator
                .generate()
                .expect("failed to generate parent agent id")
        );
        let parent_session_id = format!(
            "diagnostic_parent_{}",
            tsid_generator
                .generate()
                .expect("failed to generate parent id")
        );
        let parent_agent = crate::db::Agent::new(
            parent_agent_id.clone(),
            format!("Diagnostic Parent Agent {}", parent_agent_id),
            Some("Temporary parent agent for Code Browse diagnostic test".to_string()),
            Some("primary".to_string()),
            None,
            "Parent diagnostic system prompt".to_string(),
            None,
            None,
            None,
            None,
            None,
            None,
            Some(false),
            None,
            Some(false),
            Some(false),
            Some(false),
            None,
        );
        let parent_config = AgentConfig {
            allowed_paths: Some(vec![repo_root().to_string_lossy().to_string()]),
            ..AgentConfig::default()
        };
        {
            let store = main_store.read().expect("store lock");
            store
                .add_agent(&parent_agent)
                .expect("failed to create diagnostic parent agent");
            store
                .create_workflow(
                    &parent_session_id,
                    "Diagnostic parent workflow for Code Browse sub-agent",
                    &parent_agent_id,
                    Some(parent_config.to_json()),
                    None,
                )
                .expect("failed to create diagnostic parent workflow");
        }

        let chat_state = ChatState::new(Arc::new(WindowChannels::new()), None, main_store.clone());
        let gateway: Arc<dyn Gateway> = Arc::new(NoopGateway);
        let factory: Arc<dyn SubAgentFactory> = Arc::new(DefaultSubAgentFactory {
            main_store: main_store.clone(),
            chat_state,
            gateway,
            app_data_dir,
            tsid_generator: tsid_generator.clone(),
        });

        let tool = TaskTool::new(factory, main_store, Arc::new(NoopGateway), tsid_generator)
            .with_parent_session(parent_session_id.clone())
            .with_child_agents(vec![code_browse_agent]);

        let prompt = "Scan module `src-tauri/src/workflow/react/orchestrator.rs`. \
            Report the key responsibilities, the sub_agent_run/sub_agent_output execution flow, \
            and any likely cause of failures when Code Browse is launched as a pre-task. \
            Do not modify files. Keep the result concise.";
        let result = tool
            .call(json!({
                "description": "Scan orchestrator module",
                "prompt": prompt,
                "child_agent_name": "Code Browse",
                "execution_mode": "call"
            }))
            .await
            .unwrap_or_else(|error| panic!("sub_agent_run failed before execution: {}", error));

        eprintln!("sub_agent_run result: {:?}", result);
        let task_id = result
            .structured_content
            .as_ref()
            .and_then(|value| value.get("task_id"))
            .and_then(|value| value.as_str())
            .expect("sub_agent_run result should include task_id")
            .to_string();
        eprintln!("diagnostic sub-agent task_id: {}", task_id);

        let started_at = std::time::Instant::now();
        loop {
            if let Some(snapshot) = COMPLETED_BACKGROUND_TASKS.get(&task_id) {
                eprintln!("diagnostic completed output:\n{}", snapshot.output);
                assert!(
                    !snapshot.output.contains("Status: failed"),
                    "Code Browse sub-agent failed: {}",
                    snapshot.output
                );
                break;
            }

            if started_at.elapsed() > std::time::Duration::from_secs(timeout_secs) {
                let active = super::list_background_task_ids_for_owner(&parent_session_id);
                panic!(
                    "Code Browse sub-agent did not complete within {}s. task_id={}, active_tasks={:?}",
                    timeout_secs, task_id, active
                );
            }

            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }
}
