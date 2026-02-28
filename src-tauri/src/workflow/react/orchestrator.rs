use crate::ai::interaction::chat_completion::ChatState;
use crate::ai::traits::chat::MCPToolDeclaration;
use crate::db::MainStore;
use crate::tools::{NativeToolResult, ToolCallResult, ToolCategory, ToolDefinition, ToolError};
use crate::workflow::react::error::WorkflowEngineError;
use crate::workflow::react::executor::WorkflowExecutor;
use crate::workflow::react::gateway::Gateway;
use async_trait::async_trait;
use dashmap::DashMap;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Represents different types of background tasks for unified management
pub enum BackgroundTask {
    /// An autonomous sub-agent running its own ReAct loop
    SubAgent(Arc<Mutex<WorkflowExecutor>>),
    /// A raw shell command running in the background
    ShellCommand {
        command: String,
        stdout: Arc<Mutex<String>>,
        stderr: Arc<Mutex<String>>,
        status: Arc<Mutex<String>>, // "Running", "Completed", "Error"
    },
}

lazy_static::lazy_static! {
    /// Global registry for all background tasks (Sub-agents and Shell commands)
    /// This allows different tools to share the same task_id namespace.
    pub static ref BACKGROUND_TASKS: Arc<DashMap<String, BackgroundTask>> = Arc::new(DashMap::new());
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
    ) -> Result<WorkflowExecutor, WorkflowEngineError>;
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
    ) -> Result<WorkflowExecutor, WorkflowEngineError> {
        let agent_config = {
            let store = self.main_store.read().map_err(|e| {
                WorkflowEngineError::Db(crate::db::error::StoreError::LockError(e.to_string()))
            })?;
            store.get_agent(agent_id)?.ok_or_else(|| {
                WorkflowEngineError::General(format!("Agent config {} not found", agent_id))
            })?
        };

        let (_signal_tx, signal_rx) = tokio::sync::mpsc::channel(32);

        Ok(WorkflowExecutor::new(
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
            vec![], // Allowed paths inherited or managed by the caller
            self.app_data_dir.clone(),
            Some(subagent_type.to_string()),
            signal_rx,
            self.tsid_generator.clone(),
            self.chat_state.tool_manager.clone(),
        ))
    }
}

/// Task tool for spawning autonomous sub-agents (Full Spec Clone)
pub struct TaskTool {
    executor_factory: Arc<dyn SubAgentFactory>,
    tsid_generator: Arc<crate::libs::tsid::TsidGenerator>,
}

impl TaskTool {
    pub fn new(
        factory: Arc<dyn SubAgentFactory>,
        tsid_generator: Arc<crate::libs::tsid::TsidGenerator>,
    ) -> Self {
        Self {
            executor_factory: factory,
            tsid_generator,
        }
    }
}

#[async_trait]
impl ToolDefinition for TaskTool {
    fn name(&self) -> &str {
        crate::tools::TOOL_TASK
    }

    fn description(&self) -> &str {
        "Launch a new agent to handle complex, multi-step tasks autonomously.\n\n\
        The Task tool launches specialized agents (subprocesses) that autonomously handle complex tasks. \
        Each agent type has specific capabilities and tools available to it.\n\n\
        Available agent types:\n\
        - Programming: Expert in code generation, debugging, and refactoring. Specialist for code modification and technical implementation.\n\
        - Writing: Expert in content creation, documentation, and translation.\n\
        - Browsing: Fast agent specialized for exploring codebases, web research and information gathering. Use this when you need to quickly find files by patterns, search code for keywords, or answer questions about the codebase.\n\
        - Vision: Specialized in analyzing images, visual UI elements, and screenshots.\n\
        - Planning: Strategic architect for designing multi-stage implementation plans. Returns step-by-step plans, identifies critical files, and considers architectural trade-offs.\n\
        - General: General-purpose agent for researching complex questions, searching for code, and executing multi-step tasks.\n\n\
        When calling this agent, the tool will return a task_id if 'run_in_background' is set to true. \
        You can use the 'task_output' tool to check the progress or get the final result of a background agent."
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
                    "description": { "type": "string", "description": "A short (3-5 word) description of the task" },
                    "prompt": { "type": "string", "description": "The task for the agent to perform" },
                    "subagent_type": {
                        "type": "string",
                        "enum": ["Programming", "Writing", "Browsing", "Vision", "Planning", "General"],
                        "description": "The type of specialized agent to use for this task"
                    },
                    "run_in_background": {
                        "type": "boolean",
                        "description": "Set to true to run this agent in the background. The tool result will include a task_id - use task_output tool to check on output."
                    }
                },
                "required": ["description", "prompt", "subagent_type"]
            }),
            output_schema: None,
            disabled: false,
        }
    }

    async fn call(&self, params: Value) -> NativeToolResult {
        let description = params["description"].as_str().unwrap_or("sub-task");
        let prompt = params["prompt"]
            .as_str()
            .ok_or(ToolError::InvalidParams("prompt is required".to_string()))?;
        let subagent_type = params["subagent_type"].as_str().unwrap_or("General");
        let run_in_background = params["run_in_background"].as_bool().unwrap_or(false);

        // Use TSID for unique time-sorted IDs
        let task_id = format!(
            "task_{}_{}",
            subagent_type.to_lowercase(),
            self.tsid_generator
                .generate()
                .map_err(|e| ToolError::ExecutionFailed(e))?
        );

        let mut sub_executor = self
            .executor_factory
            .create_executor("default_sub_agent", &task_id, prompt, subagent_type)
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to create sub-executor: {}", e))
            })?;

        sub_executor
            .init()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Sub-agent init failed: {}", e)))?;

        let sub_executor_arc = Arc::new(Mutex::new(sub_executor));

        if run_in_background {
            let exec_clone = sub_executor_arc.clone();
            BACKGROUND_TASKS.insert(task_id.clone(), BackgroundTask::SubAgent(sub_executor_arc));

            tokio::spawn(async move {
                let mut guard = exec_clone.lock().await;
                if let Err(e) = guard.run_loop().await {
                    log::error!("Background task {} failed: {}", guard.session_id, e);
                }
            });

            return Ok(ToolCallResult::success(Some(json!({
                "task_id": task_id,
                "status": "Running",
                "message": format!("Task '{}' has been started in the background. Use 'task_output' with the task_id to retrieve results later.", description)
            }).to_string()), None));
        }

        // Blocking mode: Wait for completion
        {
            let mut guard = sub_executor_arc.lock().await;
            guard.run_loop().await.map_err(|e| {
                ToolError::ExecutionFailed(format!("Sub-agent execution failed: {}", e))
            })?;

            // 1. Look for 'finish_task' in assistant messages first
            for msg in guard.context.messages.iter().rev() {
                if msg.role == "assistant" {
                    let cleaned = crate::libs::util::format_json_str(&msg.message);
                    if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&cleaned) {
                        if let Some(tool) = json_val.get("tool") {
                            if tool["name"] == "finish_task" {
                                if let Some(summary) =
                                    tool["arguments"].get("summary").and_then(|v| v.as_str())
                                {
                                    return Ok(ToolCallResult::success(
                                        Some(summary.to_string()),
                                        None,
                                    ));
                                }
                            }
                        } else if let Some(tool_calls) =
                            json_val.get("tool_calls").and_then(|v| v.as_array())
                        {
                            for call in tool_calls {
                                let func = call
                                    .get("function")
                                    .cloned()
                                    .unwrap_or_else(|| call.clone());
                                if func["name"] == "finish_task" {
                                    let args_raw = func
                                        .get("arguments")
                                        .cloned()
                                        .or_else(|| func.get("input").cloned())
                                        .unwrap_or(serde_json::json!({}));
                                    let args = if args_raw.is_string() {
                                        serde_json::from_str(args_raw.as_str().unwrap())
                                            .unwrap_or_default()
                                    } else {
                                        args_raw
                                    };
                                    if let Some(summary) =
                                        args.get("summary").and_then(|v| v.as_str())
                                    {
                                        return Ok(ToolCallResult::success(
                                            Some(summary.to_string()),
                                            None,
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // 2. Fallback: last assistant message content
            let last_assistant_msg = guard
                .context
                .messages
                .iter()
                .rev()
                .find(|m| m.role == "assistant")
                .ok_or_else(|| {
                    ToolError::ExecutionFailed(
                        "Sub-agent finished but provided no answer".to_string(),
                    )
                })?;

            Ok(ToolCallResult::success(
                Some(last_assistant_msg.message.clone()),
                None,
            ))
        }
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
            BackgroundTask::SubAgent(exec_arc) => {
                let guard = exec_arc.lock().await;
                // Simplified: return latest assistant message or state
                let mut result = format!("Status: {:?}. ", guard.state);
                if let Some(last_msg) = guard
                    .context
                    .messages
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
        }
    }

    async fn call(&self, params: Value) -> NativeToolResult {
        let task_id = params["task_id"]
            .as_str()
            .ok_or(ToolError::InvalidParams("task_id required".into()))?;
        if let Some((_, task)) = BACKGROUND_TASKS.remove(task_id) {
            match task {
                BackgroundTask::SubAgent(exec_arc) => {
                    let mut guard = exec_arc.lock().await;
                    guard.state = crate::workflow::react::types::WorkflowState::Completed;
                    // Force stop
                }
                BackgroundTask::ShellCommand { status, .. } => {
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
