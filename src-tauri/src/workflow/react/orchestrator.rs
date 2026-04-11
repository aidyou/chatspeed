use crate::ai::interaction::chat_completion::ChatState;
use crate::ai::traits::chat::MCPToolDeclaration;
use crate::db::{Agent, MainStore};
use crate::tools::{NativeToolResult, ToolCallResult, ToolCategory, ToolDefinition, ToolError};
use crate::workflow::react::child_tasks::get_child_task_registry;
use crate::workflow::react::engine::ReActExecutor;
use crate::workflow::react::error::WorkflowEngineError;
use crate::workflow::react::gateway::Gateway;

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
            get_child_task_registry().unregister_child_task(task_id);
            true
        }
        BackgroundTask::ShellCommand {
            status, stop_tx, ..
        } => {
            if let Some(tx) = stop_tx.lock().await.take() {
                let _ = tx.send(());
            }
            *status.lock().await = "Stopped".to_string();
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
        _task: &str,
        subagent_type: &str,
    ) -> Result<Arc<Mutex<dyn ReActExecutor>>, WorkflowEngineError> {
        let agent_config = {
            let store = self.main_store.read().map_err(|e| {
                WorkflowEngineError::Db(crate::db::error::StoreError::LockError(e.to_string()))
            })?;
            store.get_agent(agent_id)?.ok_or_else(|| {
                WorkflowEngineError::General(format!("Agent config {} not found", agent_id))
            })?
        };

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
    tsid_generator: Arc<crate::libs::tsid::TsidGenerator>,
    parent_session_id: Option<String>,
    child_agents: Vec<Agent>,
}

impl TaskTool {
    pub fn new(
        factory: Arc<dyn SubAgentFactory>,
        tsid_generator: Arc<crate::libs::tsid::TsidGenerator>,
    ) -> Self {
        Self {
            executor_factory: factory,
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
        Use execution_mode='call' when the parent must wait for the child result, or execution_mode='background' when the child should continue asynchronously and be inspected later with task_output."
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
                    "child_agent_id": {
                        "type": "string",
                        "enum": child_agent_ids,
                        "description": "The pre-configured child agent to execute this task"
                    },
                    "execution_mode": {
                        "type": "string",
                        "enum": ["call", "background"],
                        "description": "Execution mode for the child agent. Use 'call' to wait for child completion and resume the parent workflow with the child result. Use 'background' to continue asynchronously and inspect the child later via task_output."
                    }
                },
                "required": ["description", "prompt", "child_agent_id"]
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
        let child_agent_id = params["child_agent_id"]
            .as_str()
            .ok_or(ToolError::InvalidParams(
                "child_agent_id is required".to_string(),
            ))?;
        let execution_mode = params["execution_mode"].as_str().unwrap_or("call");
        if !matches!(execution_mode, "call" | "background") {
            return Err(ToolError::InvalidParams(format!(
                "execution_mode must be either 'call' or 'background', got '{}'",
                execution_mode
            )));
        }

        let child_agent = self
            .child_agents
            .iter()
            .find(|agent| agent.id == child_agent_id)
            .cloned()
            .ok_or_else(|| {
                ToolError::InvalidParams(format!(
                    "child_agent_id '{}' is not available to the current agent",
                    child_agent_id
                ))
            })?;

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
            .create_executor(&child_agent.id, &task_id, prompt, &child_agent.name)
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
            let exec_clone = sub_executor.clone();
            BACKGROUND_TASKS.insert(
                task_id.clone(),
                BackgroundTask::SubAgent {
                    owner_session_id: self.parent_session_id.clone(),
                    executor: sub_executor,
                },
            );

            tokio::spawn(async move {
                let mut guard = exec_clone.lock().await;
                if let Err(e) = guard.run_loop().await {
                    log::error!("Background task {} failed: {}", guard.session_id(), e);
                }
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
        tokio::spawn(async move {
            let mut guard = exec_clone.lock().await;
            let result = guard.run_loop().await;
            let final_state = guard.state();

            let completion_result = match result {
                Ok(_) => {
                    let messages = guard.messages();
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
                        "summary": summary.unwrap_or_default()
                    })
                }
                Err(e) => {
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
                        "error": e.to_string()
                    })
                }
            };

            let _ = crate::workflow::react::manager::WorkflowManager::send_signal_to_session(
                &parent_id,
                json!({
                    "type": "child_task_complete",
                    "child_task_id": task_id_clone,
                    "result": completion_result
                })
                .to_string(),
            );

            get_child_task_registry().unregister_child_task(&task_id_clone);
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
        - Use this tool to check if a parallel agent or process has finished its work."
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
                    "task_id": { "type": "string", "description": "The task ID to get output from" }
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
        let task = BACKGROUND_TASKS.get(task_id).ok_or_else(|| {
            ToolError::ExecutionFailed(format!(
                "Task {} not found in active background tasks",
                task_id
            ))
        })?;

        match task.value() {
            BackgroundTask::SubAgent { executor, .. } => {
                let guard = executor.lock().await;
                // Simplified: return latest assistant message or state
                let mut result = format!("Status: {:?}. ", guard.state());
                if let Some(last_msg) = guard
                    .messages()
                    .iter()
                    .rev()
                    .find(|m| m.role == "assistant")
                {
                    result.push_str("Latest Output: ");
                    result.push_str(&last_msg.message);
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
        }
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
                }
                BackgroundTask::ShellCommand {
                    status, stop_tx, ..
                } => {
                    if let Some(tx) = stop_tx.lock().await.take() {
                        let _ = tx.send(());
                    }
                    *status.lock().await = "Stopped".to_string();
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
