use crate::ai::interaction::chat_completion::ChatState;
use crate::ai::traits::chat::MCPToolDeclaration;
use crate::db::{Agent, AgentConfig, MainStore};
use crate::tools::{NativeToolResult, ToolCallResult, ToolCategory, ToolDefinition, ToolError};
use crate::workflow::react::child_tasks::get_child_task_registry;
use crate::workflow::react::engine::ReActExecutor;
use crate::workflow::react::error::WorkflowEngineError;
use crate::workflow::react::events::WorkflowEvent;
use crate::workflow::react::gateway::Gateway;
use crate::workflow::react::types::{ChildTaskCompletion, ExecutionContext, RuntimeState};

use async_trait::async_trait;
use dashmap::DashMap;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::oneshot;
use tokio::sync::Mutex;

/// Represents different types of background tasks for unified management
pub enum BackgroundTask {
    /// An autonomous sub-agent running its own ReAct loop
    SubAgent {
        owner_session_id: Option<String>,
        executor: Arc<Mutex<dyn ReActExecutor>>,
    },
    /// A raw shell command running in the background
    ShellCommand {
        owner_session_id: Option<String>,
        command: String,
        stdout: Arc<Mutex<String>>,
        stderr: Arc<Mutex<String>>,
        status: Arc<Mutex<String>>, // "Running", "Completed", "Error"
        stop_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    },
}

#[derive(Debug, Clone)]
pub struct CompletedTaskSnapshot {
    pub output: String,
}

impl BackgroundTask {
    pub fn owner_session_id(&self) -> Option<&str> {
        match self {
            BackgroundTask::SubAgent {
                owner_session_id, ..
            }
            | BackgroundTask::ShellCommand {
                owner_session_id, ..
            } => owner_session_id.as_deref(),
        }
    }
}

lazy_static::lazy_static! {
    /// Global registry for all background tasks (Sub-agents and Shell commands)
    /// This allows different tools to share the same task_id namespace.
    pub static ref BACKGROUND_TASKS: Arc<DashMap<String, BackgroundTask>> = Arc::new(DashMap::new());
    /// Terminal snapshots allow task_output to inspect tasks after they leave the active registry.
    pub static ref COMPLETED_BACKGROUND_TASKS: Arc<DashMap<String, CompletedTaskSnapshot>> = Arc::new(DashMap::new());
    static ref TASK_OUTPUT_THROTTLE: Arc<DashMap<String, (i64, String)>> = Arc::new(DashMap::new());
}

fn remember_completed_task(task_id: impl Into<String>, output: impl Into<String>) {
    COMPLETED_BACKGROUND_TASKS.insert(
        task_id.into(),
        CompletedTaskSnapshot {
            output: output.into(),
        },
    );
}

fn build_child_completion(
    parent_session_id: &str,
    child_task_id: &str,
    result: &Value,
) -> ChildTaskCompletion {
    ChildTaskCompletion {
        child_task_id: child_task_id.to_string(),
        parent_session_id: parent_session_id.to_string(),
        status: result
            .get("status")
            .and_then(|value| value.as_str())
            .unwrap_or("completed")
            .to_string(),
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

fn persist_child_completion(
    main_store: &Arc<std::sync::RwLock<MainStore>>,
    completion: ChildTaskCompletion,
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
            wait_reason: Some(crate::workflow::react::types::WaitReason::ChildTask),
            current_step: 0,
            max_steps: 0,
            pending_tools: Vec::new(),
            last_action_summary: None,
            current_context_tokens: None,
            max_context_tokens: None,
            last_event_id: None,
            version: ExecutionContext::CURRENT_VERSION.to_string(),
            waiting_on_task_id: Some(completion.child_task_id.clone()),
            child_sessions: vec![completion.child_task_id.clone()],
            pending_child_completions: Vec::new(),
        });

    context
        .pending_child_completions
        .retain(|existing| existing.child_task_id != completion.child_task_id);
    context.pending_child_completions.push(completion);
    store
        .upsert_execution_context(&context)
        .map_err(WorkflowEngineError::Db)?;
    Ok(())
}

fn append_child_task_event(
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

fn format_available_task_ids() -> String {
    let mut task_ids: Vec<String> = BACKGROUND_TASKS
        .iter()
        .map(|entry| entry.key().clone())
        .chain(
            COMPLETED_BACKGROUND_TASKS
                .iter()
                .map(|entry| entry.key().clone()),
        )
        .collect();
    task_ids.sort();
    task_ids.dedup();

    if task_ids.is_empty() {
        "No active or completed background tasks are currently available.".to_string()
    } else {
        format!("Currently available task IDs: {}.", task_ids.join(", "))
    }
}

pub fn list_background_task_ids_for_owner(owner_session_id: &str) -> Vec<String> {
    BACKGROUND_TASKS
        .iter()
        .filter_map(|entry| {
            if entry.value().owner_session_id() == Some(owner_session_id) {
                Some(entry.key().clone())
            } else {
                None
            }
        })
        .collect()
}

pub async fn stop_background_task(task_id: &str, chat_state: Option<&Arc<ChatState>>) -> bool {
    let Some((_, task)) = BACKGROUND_TASKS.remove(task_id) else {
        return false;
    };

    match task {
        BackgroundTask::SubAgent { executor, .. } => {
            if let Some(chat_state) = chat_state {
                let mut chats = chat_state.chats.lock().await;
                if let Some(protocol_chats) = chats.get_mut(&crate::ccproxy::ChatProtocol::OpenAI) {
                    if let Some(chat) = protocol_chats.get_mut(task_id) {
                        chat.set_stop_flag(true).await;
                    }
                }
            }

            let mut guard = executor.lock().await;
            guard.set_state(crate::workflow::react::types::WorkflowState::Cancelled);
            remember_completed_task(task_id, format!("Task {} has been cancelled.", task_id));
            get_child_task_registry().unregister_child_task(task_id);
            true
        }
        BackgroundTask::ShellCommand {
            command,
            stdout,
            stderr,
            status,
            stop_tx,
            ..
        } => {
            if let Some(tx) = stop_tx.lock().await.take() {
                let _ = tx.send(());
            }
            *status.lock().await = "Stopped".to_string();
            let out = stdout.lock().await.clone();
            let err = stderr.lock().await.clone();
            remember_completed_task(
                task_id,
                format!(
                    "Command: {}\nStatus: Stopped\nSTDOUT: {}\nSTDERR: {}",
                    command, out, err
                ),
            );
            get_child_task_registry().unregister_child_task(task_id);
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

        let workflow_config = AgentConfig {
            allowed_paths: inherited_allowed_paths.or_else(|| {
                agent_config
                    .allowed_paths
                    .as_deref()
                    .and_then(|s| serde_json::from_str(s).ok())
            }),
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
        }

        let (_signal_tx, signal_rx) = tokio::sync::mpsc::channel(32);

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
                    vec![],
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
                    vec![],
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
    tsid_generator: Arc<crate::libs::tsid::TsidGenerator>,
    parent_session_id: Option<String>,
    child_agents: Vec<Agent>,
}

impl TaskTool {
    pub fn new(
        factory: Arc<dyn SubAgentFactory>,
        main_store: Arc<std::sync::RwLock<MainStore>>,
        tsid_generator: Arc<crate::libs::tsid::TsidGenerator>,
    ) -> Self {
        Self {
            executor_factory: factory,
            main_store,
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
        crate::tools::TOOL_TASK
    }

    fn description(&self) -> &str {
        "Launch one of the pre-configured child agents owned by the current primary agent. \
        Each child agent has its own prompt, model setup, and tool permissions. \
        Use the child_agent_id that best matches the requested task. \
        The prompt must contain a clear objective, the exact scope to investigate or implement, relevant constraints, \
        and the expected output format or success criteria. \
        If the child agent must return structured findings, explicitly state which facts, files, risks, or conclusions must be included. \
        Use execution_mode='call' when the parent cannot continue until the child finishes. \
        Use execution_mode='background' when the child can run in parallel while the parent continues other work; completion will be reported automatically, and task_output can be used later if the result is explicitly needed."
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
                        "description": "Execution mode for the child agent. Use 'call' if you must wait for the child to finish before continuing; the parent workflow will pause and resume with the final child result. Use 'background' if you can continue other work in parallel; the system will report completion automatically, and you may inspect the result later with task_output when needed."
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
            "task_{}_{}",
            child_agent.name.to_lowercase().replace(' ', "_"),
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
                if let Err(e) = append_child_task_event(
                    &self.main_store,
                    WorkflowEvent::child_task_started(
                        parent_session_id.clone(),
                        task_id.clone(),
                        "background".to_string(),
                    ),
                ) {
                    log::warn!(
                        "[Workflow][session={}][parent={}][phase=child_start] Failed to persist background child task event: {}",
                        task_id,
                        parent_session_id,
                        e
                    );
                }
            }
            let exec_clone = sub_executor.clone();
            let task_id_clone = task_id.clone();
            BACKGROUND_TASKS.insert(
                task_id.clone(),
                BackgroundTask::SubAgent {
                    owner_session_id: self.parent_session_id.clone(),
                    executor: sub_executor,
                },
            );

            tokio::spawn(async move {
                let mut guard = exec_clone.lock().await;
                let result = guard.run_loop().await;
                let final_state = guard.state();
                let messages = guard.messages();
                let tool_calls_count = messages
                    .iter()
                    .filter(|message| message.role == "tool")
                    .count();
                let latest_output = messages
                    .iter()
                    .rev()
                    .find(|message| message.role == "assistant")
                    .map(|message| message.message.clone())
                    .unwrap_or_default();

                let output = match result {
                    Ok(_) => {
                        let status = if final_state
                            == crate::workflow::react::types::WorkflowState::Cancelled
                        {
                            "cancelled"
                        } else {
                            "completed"
                        };
                        format!(
                            "Status: {}. Tool Calls: {}. Latest Output: {}",
                            status, tool_calls_count, latest_output
                        )
                    }
                    Err(e) => {
                        log::error!("Background task {} failed: {}", guard.session_id(), e);
                        format!(
                            "Status: failed. Tool Calls: {}. Latest Output: {}",
                            tool_calls_count, e
                        )
                    }
                };
                remember_completed_task(task_id_clone.clone(), output);
                TASK_OUTPUT_THROTTLE.remove(&task_id_clone);
                BACKGROUND_TASKS.remove(&task_id_clone);
            });

            return Ok(ToolCallResult::success(Some(json!({
                "task_id": task_id,
                "status": "Running",
                "message": format!("Task '{}' has been started in the background. Use 'task_output' with the task_id to retrieve results later.", description)
            }).to_string()), Some(json!({
                "task_id": task_id,
                "status": "running",
                "mode": "background"
            }))));
        }

        let parent_id = self.parent_session_id.as_ref().ok_or_else(|| {
            ToolError::ExecutionFailed(
                "task execution_mode='call' requires an active parent session".to_string(),
            )
        })?;

        get_child_task_registry().register_child_task(task_id.clone(), parent_id.clone());
        if let Err(e) = append_child_task_event(
            &self.main_store,
            WorkflowEvent::child_task_started(
                parent_id.clone(),
                task_id.clone(),
                "call".to_string(),
            ),
        ) {
            log::warn!(
                "[Workflow][session={}][parent={}][phase=child_start] Failed to persist child task event: {}",
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
            },
        );

        let parent_id = parent_id.clone();
        let task_id_clone = task_id.clone();
        let main_store = self.main_store.clone();
        tokio::spawn(async move {
            let mut guard = exec_clone.lock().await;
            let result = guard.run_loop().await;
            let final_state = guard.state();

            let completion_result = match result {
                Ok(_) => {
                    let messages = guard.messages();
                    let tool_calls_count = messages
                        .iter()
                        .filter(|message| message.role == "tool")
                        .count();
                    let mut summary = None;
                    for msg in messages.iter().rev() {
                        if msg.role == "assistant" {
                            summary = Some(msg.message.clone());
                            break;
                        }
                    }
                    let status =
                        if final_state == crate::workflow::react::types::WorkflowState::Cancelled {
                            "cancelled"
                        } else {
                            "completed"
                        };
                    json!({
                        "status": status,
                        "task_id": task_id_clone,
                        "summary": summary.unwrap_or_default(),
                        "tool_calls_count": tool_calls_count
                    })
                }
                Err(e) => {
                    let messages = guard.messages();
                    let tool_calls_count = messages
                        .iter()
                        .filter(|message| message.role == "tool")
                        .count();
                    let status = if matches!(e, WorkflowEngineError::Cancelled(_))
                        || final_state == crate::workflow::react::types::WorkflowState::Cancelled
                    {
                        "cancelled"
                    } else {
                        "failed"
                    };
                    json!({
                        "status": status,
                        "task_id": task_id_clone,
                        "error": e.to_string(),
                        "tool_calls_count": tool_calls_count
                    })
                }
            };

            let completion_status = completion_result
                .get("status")
                .and_then(|value| value.as_str())
                .unwrap_or("completed")
                .to_string();
            if let Err(e) = append_child_task_event(
                &main_store,
                WorkflowEvent::child_task_completed(
                    parent_id.clone(),
                    task_id_clone.clone(),
                    completion_status,
                    completion_result.clone(),
                ),
            ) {
                log::warn!(
                    "[Workflow][session={}][parent={}][phase=child_completion] Failed to persist child completion event: {}",
                    task_id_clone,
                    parent_id,
                    e
                );
            }

            let completion = build_child_completion(&parent_id, &task_id_clone, &completion_result);
            if let Err(e) = persist_child_completion(&main_store, completion) {
                log::error!(
                    "[Workflow][session={}][parent={}][phase=child_completion] Failed to persist child completion: {}",
                    task_id_clone,
                    parent_id,
                    e
                );
            }

            if let Err(e) = crate::workflow::react::manager::WorkflowManager::send_signal_to_session(
                &parent_id,
                json!({
                    "type": "child_task_complete",
                    "child_task_id": task_id_clone,
                    "result": completion_result
                })
                .to_string(),
            ) {
                log::warn!(
                    "[Workflow][session={}][parent={}][phase=child_completion] Live signal delivery failed; durable completion will be replayed on recovery: {}",
                    task_id_clone,
                    parent_id,
                    e
                );
            }

            let output = match completion_result
                .get("status")
                .and_then(|value| value.as_str())
            {
                Some("completed") => format!(
                    "Status: completed. Latest Output: {}",
                    completion_result
                        .get("summary")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default()
                ),
                Some(status) => format!(
                    "Status: {}. Latest Output: {}",
                    status,
                    completion_result
                        .get("error")
                        .and_then(|value| value.as_str())
                        .or_else(|| completion_result
                            .get("summary")
                            .and_then(|value| value.as_str()))
                        .unwrap_or_default()
                ),
                None => "Status: completed.".to_string(),
            };
            remember_completed_task(task_id_clone.clone(), output);
            get_child_task_registry().unregister_child_task(&task_id_clone);
            TASK_OUTPUT_THROTTLE.remove(&task_id_clone);
            BACKGROUND_TASKS.remove(&task_id_clone);
        });

        Ok(ToolCallResult::success(Some(json!({
            "status": "waiting",
            "task_id": task_id,
            "message": format!("Task '{}' has been spawned. Parent workflow will wait for completion.", description)
        }).to_string()), Some(json!({
            "status": "waiting",
            "task_id": task_id,
            "mode": "call"
        }))))
    }
}

/// Tool to retrieve results from background tasks (Full Spec Clone)
pub struct TaskOutputTool;

#[async_trait]
impl ToolDefinition for TaskOutputTool {
    fn name(&self) -> &str {
        crate::tools::TOOL_TASK_OUTPUT
    }

    fn description(&self) -> &str {
        "- Retrieves output from a running or completed task (background agent, shell, or remote session)\n\
        - Takes a task_id parameter identifying the task\n\
        - Returns the task output along with status information\n\
        - Set wait_until_complete=true only when the next step depends on the final child result; the tool will wait until the task finishes, fails, or is stopped\n\
        - Leave wait_until_complete=false when you only need a non-blocking status check. Do not poll running child agents repeatedly; continue other work or wait for the automatic completion notification."
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
                    "task_id": { "type": "string", "description": "The task ID to get output from" },
                    "wait_until_complete": {
                        "type": "boolean",
                        "description": "When true, wait until the task reaches a terminal state and return only the final result. Use this if you cannot continue until the child agent finishes. When false or omitted, return the current status immediately; use this for non-blocking checks while continuing other work."
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
        let task_id = params["task_id"]
            .as_str()
            .ok_or(ToolError::InvalidParams("task_id required".into()))?;
        let wait_until_complete = params["wait_until_complete"].as_bool().unwrap_or(false);
        if wait_until_complete {
            loop {
                if let Some(snapshot) = COMPLETED_BACKGROUND_TASKS.get(task_id) {
                    return Ok(ToolCallResult::success(Some(snapshot.output.clone()), None));
                }

                let Some(task) = BACKGROUND_TASKS.get(task_id) else {
                    break;
                };

                match task.value() {
                    BackgroundTask::SubAgent { executor, .. } => {
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
                            let mut result = format!("Status: {:?}. ", state);
                            if let Some(last_msg) = guard
                                .messages()
                                .iter()
                                .rev()
                                .find(|m| m.role == "assistant" || m.role == "tool")
                            {
                                result.push_str("Latest Output: ");
                                result.push_str(&last_msg.message);
                            }
                            return Ok(ToolCallResult::success(Some(result), None));
                        }
                    }
                    BackgroundTask::ShellCommand {
                        command,
                        stdout,
                        stderr,
                        status,
                        ..
                    } => {
                        let command = command.clone();
                        let stdout = stdout.clone();
                        let stderr = stderr.clone();
                        let status = status.clone();
                        drop(task);

                        let current_status = status.lock().await.clone();
                        if current_status != "Running" {
                            let out = stdout.lock().await.clone();
                            let err = stderr.lock().await.clone();
                            return Ok(ToolCallResult::success(
                                Some(format!(
                                    "Command: {}\nStatus: {}\nSTDOUT: {}\nSTDERR: {}",
                                    command, current_status, out, err
                                )),
                                None,
                            ));
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
                                        "{}\n<SYSTEM_REMINDER>This task is still running. Do not poll it repeatedly; continue other work or wait for the completion event.</SYSTEM_REMINDER>",
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
                BackgroundTask::ShellCommand {
                    command,
                    stdout,
                    stderr,
                    status,
                    ..
                } => {
                    let out = stdout.lock().await;
                    let err = stderr.lock().await;
                    let s = status.lock().await;
                    Ok(ToolCallResult::success(
                        Some(format!(
                            "Command: {}\nStatus: {}\nSTDOUT: {}\nSTDERR: {}",
                            command, s, out, err
                        )),
                        None,
                    ))
                }
            };
        }

        if let Some(snapshot) = COMPLETED_BACKGROUND_TASKS.get(task_id) {
            return Ok(ToolCallResult::success(Some(snapshot.output.clone()), None));
        }

        let available_tasks = format_available_task_ids();
        Err(ToolError::ExecutionFailed(format!(
            "Task {} not found in active or completed background tasks.\n<SYSTEM_REMINDER>Before calling task_output, make sure you have created a background task with the task tool and use the exact task_id returned by that tool. {} Do not use task_output as a generic final-answer tool.</SYSTEM_REMINDER>",
            task_id,
            available_tasks
        )))
    }
}

/// Tool to stop a background agent (Full Spec Clone)
pub struct TaskStopTool;

#[async_trait]
impl ToolDefinition for TaskStopTool {
    fn name(&self) -> &str {
        crate::tools::TOOL_TASK_STOP
    }

    fn description(&self) -> &str {
        "- Stops a running background task by its ID\n\
        - Takes a task_id parameter identifying the task to stop\n\
        - Returns a success or failure status\n\
        - Use this tool when you need to terminate a long-running task"
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
                    "task_id": { "type": "string", "description": "The ID of the background task to stop" }
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
        if let Some((_, task)) = BACKGROUND_TASKS.remove(task_id) {
            match task {
                BackgroundTask::SubAgent { executor, .. } => {
                    let mut guard = executor.lock().await;
                    guard.set_state(crate::workflow::react::types::WorkflowState::Cancelled);
                    remember_completed_task(
                        task_id,
                        format!("Task {} has been terminated.", task_id),
                    );
                }
                BackgroundTask::ShellCommand {
                    command,
                    stdout,
                    stderr,
                    status,
                    stop_tx,
                    ..
                } => {
                    if let Some(tx) = stop_tx.lock().await.take() {
                        let _ = tx.send(());
                    }
                    *status.lock().await = "Stopped".to_string();
                    let out = stdout.lock().await.clone();
                    let err = stderr.lock().await.clone();
                    remember_completed_task(
                        task_id,
                        format!(
                            "Command: {}\nStatus: Stopped\nSTDOUT: {}\nSTDERR: {}",
                            command, out, err
                        ),
                    );
                }
            }
            Ok(ToolCallResult::success(
                Some(format!("Task {} has been terminated.", task_id)),
                None,
            ))
        } else {
            Err(ToolError::ExecutionFailed(format!(
                "Task {} not found",
                task_id
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{remember_completed_task, TaskOutputTool, COMPLETED_BACKGROUND_TASKS};
    use crate::tools::ToolDefinition;
    use serde_json::json;

    #[tokio::test]
    async fn task_output_reads_completed_snapshot() {
        let task_id = "task_test_completed_snapshot";
        COMPLETED_BACKGROUND_TASKS.remove(task_id);
        remember_completed_task(task_id, "Status: completed. Latest Output: done");

        let tool = TaskOutputTool;
        let result = tool
            .call(json!({ "task_id": task_id }))
            .await
            .expect("task_output should read completed snapshot");

        let content = result.content.unwrap_or_default();
        assert!(content.contains("Status: completed"));
        assert!(content.contains("done"));

        COMPLETED_BACKGROUND_TASKS.remove(task_id);
    }

    #[test]
    fn completed_snapshot_overwrites_latest_terminal_state() {
        let task_id = "task_test_snapshot_overwrite";
        COMPLETED_BACKGROUND_TASKS.remove(task_id);

        remember_completed_task(task_id, "first");
        remember_completed_task(task_id, "second");

        let snapshot = COMPLETED_BACKGROUND_TASKS
            .get(task_id)
            .map(|entry| entry.output.clone());
        let snapshot = snapshot.expect("snapshot should exist");
        assert_eq!(snapshot, "second");

        COMPLETED_BACKGROUND_TASKS.remove(task_id);
    }
}
