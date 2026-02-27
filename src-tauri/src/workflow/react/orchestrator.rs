use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use std::path::PathBuf;
use dashmap::DashMap;
use tokio::sync::Mutex;
use crate::tools::{ToolDefinition, NativeToolResult, ToolCallResult, ToolCategory, ToolError};
use crate::ai::traits::chat::MCPToolDeclaration;
use crate::workflow::react::executor::WorkflowExecutor;
use crate::workflow::react::error::WorkflowEngineError;
use crate::workflow::react::gateway::Gateway;
use crate::ai::interaction::chat_completion::ChatState;
use crate::db::MainStore;

lazy_static::lazy_static! {
    /// Global registry for background sub-agents
    pub static ref BACKGROUND_TASKS: Arc<DashMap<String, Arc<Mutex<WorkflowExecutor>>>> = Arc::new(DashMap::new());
}

#[async_trait]
pub trait SubAgentFactory: Send + Sync {
    /// Creates a new executor instance for a sub-agent.
    async fn create_executor(&self, agent_id: &str, session_id: &str, task: &str, subagent_type: &str) -> Result<WorkflowExecutor, WorkflowEngineError>;
}

/// The default factory used to spawn sub-agents within the ReAct system
pub struct DefaultSubAgentFactory {
    pub main_store: Arc<std::sync::RwLock<MainStore>>,
    pub chat_state: Arc<ChatState>,
    pub gateway: Arc<dyn Gateway>,
    pub app_data_dir: PathBuf,
}

#[async_trait]
impl SubAgentFactory for DefaultSubAgentFactory {
    async fn create_executor(&self, agent_id: &str, session_id: &str, _task: &str, subagent_type: &str) -> Result<WorkflowExecutor, WorkflowEngineError> {
        let agent_config = {
            let store = self.main_store.read().map_err(|e| WorkflowEngineError::Db(crate::db::error::StoreError::LockError(e.to_string())))?;
            store.get_agent(agent_id)?
                .ok_or_else(|| WorkflowEngineError::General(format!("Agent config {} not found", agent_id)))?
        };

        let (_signal_tx, signal_rx) = tokio::sync::mpsc::channel(32);
        let allowed_paths = vec![]; 

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
            }),
            agent_config,
            allowed_paths,
            self.app_data_dir.clone(),
            Some(subagent_type.to_string()),
            signal_rx,
        ))
    }
}

/// Task tool for spawning autonomous sub-agents (Full Claude Code Clone)
pub struct TaskTool {
    executor_factory: Arc<dyn SubAgentFactory>,
}

impl TaskTool {
    pub fn new(factory: Arc<dyn SubAgentFactory>) -> Self {
        Self { executor_factory: factory }
    }
}

#[async_trait]
impl ToolDefinition for TaskTool {
    fn name(&self) -> &str { "task" }
    fn description(&self) -> &str { 
        "Launch a new agent to handle complex, multi-step tasks autonomously.\n\n\
        The Task tool launches specialized agents (subprocesses) that autonomously handle complex tasks. Each agent type has specific capabilities and tools available to it.\n\n\
        Available agent types and the tools they have access to:\n\
        - Programming: Expert in code generation, debugging, and refactoring. Specialist for code modification and technical implementation.\n\
        - Writing: Expert in content creation, documentation, and translation.\n\
        - Browsing: Fast agent specialized for exploring codebases, web research and information gathering. Use this when you need to quickly find files by patterns, search code for keywords, or answer questions about the codebase.\n\
        - Vision: Specialized in analyzing images, visual UI elements, and screenshots.\n\
        - Planning: Strategic architect for designing multi-stage implementation plans. Returns step-by-step plans, identifies critical files, and considers architectural trade-offs.\n\
        - General: General-purpose agent for researching complex questions, searching for code, and executing multi-step tasks."
    }
    fn category(&self) -> ToolCategory { ToolCategory::System }
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
        let prompt = params["prompt"].as_str().ok_or(ToolError::InvalidParams("prompt is required".to_string()))?;
        let subagent_type = params["subagent_type"].as_str().unwrap_or("General");
        let run_in_background = params["run_in_background"].as_bool().unwrap_or(false);
        
        log::info!("Orchestrator: Spawning {} task '{}'", subagent_type, description);

        let sub_session_id = format!("task_{}_{}", subagent_type.to_lowercase(), uuid::Uuid::new_v4().simple());

        let mut sub_executor = self.executor_factory.create_executor("default_sub_agent", &sub_session_id, prompt, subagent_type).await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to create sub-executor: {}", e)))?;

        sub_executor.init().await
            .map_err(|e| ToolError::ExecutionFailed(format!("Sub-agent init failed: {}", e)))?;
        
        let sub_executor_arc = Arc::new(Mutex::new(sub_executor));

        if run_in_background {
            let exec_clone = sub_executor_arc.clone();
            BACKGROUND_TASKS.insert(sub_session_id.clone(), sub_executor_arc);
            
            tokio::spawn(async move {
                let mut guard = exec_clone.lock().await;
                if let Err(e) = guard.run_loop().await {
                    log::error!("Background task {} failed: {}", guard.session_id, e);
                }
            });

            return Ok(ToolCallResult::success(Some(json!({ 
                "task_id": sub_session_id,
                "status": "Running",
                "message": "The task has been started in the background. Use 'task_output' with the task_id to retrieve results later."
            }).to_string()), None));
        }

        // Blocking mode
        {
            let mut guard = sub_executor_arc.lock().await;
            guard.run_loop().await
                .map_err(|e| ToolError::ExecutionFailed(format!("Sub-agent execution failed: {}", e)))?;

            for msg in guard.context.messages.iter().rev() {
                if msg.role == "assistant" {
                    let cleaned = crate::libs::util::format_json_str(&msg.message);
                    if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&cleaned) {
                        if let Some(tool) = json_val.get("tool") {
                            if tool["name"] == "finish_task" {
                                if let Some(summary) = tool["arguments"].get("summary").and_then(|v| v.as_str()) {
                                    return Ok(ToolCallResult::success(Some(summary.to_string()), None));
                                }
                            }
                        } else if let Some(tool_calls) = json_val.get("tool_calls").and_then(|v| v.as_array()) {
                            for call in tool_calls {
                                let func = call.get("function").cloned().unwrap_or_else(|| call.clone());
                                if func["name"] == "finish_task" {
                                    let args_raw = func.get("arguments").cloned().or_else(|| func.get("input").cloned()).unwrap_or(serde_json::json!({}));
                                    let args = if args_raw.is_string() {
                                        serde_json::from_str(args_raw.as_str().unwrap()).unwrap_or_default()
                                    } else {
                                        args_raw
                                    };
                                    if let Some(summary) = args.get("summary").and_then(|v| v.as_str()) {
                                        return Ok(ToolCallResult::success(Some(summary.to_string()), None));
                                    }
                                }
                            }
                        }
                    }
                }
            }

            let last_assistant_msg = guard.context.messages.iter()
                .rev()
                .find(|m| m.role == "assistant")
                .ok_or_else(|| ToolError::ExecutionFailed("Sub-agent finished but provided no answer".to_string()))?;

            Ok(ToolCallResult::success(Some(last_assistant_msg.message.clone()), None))
        }
    }
}

/// Tool to retrieve results from background agents (Full Claude Code Clone)
pub struct TaskOutputTool;

#[async_trait]
impl ToolDefinition for TaskOutputTool {
    fn name(&self) -> &str { "task_output" }
    fn description(&self) -> &str { 
        "- Retrieves output from a running or completed task (background agent or session)\n\
        - Takes a task_id parameter identifying the task\n\
        - Returns the task output along with status information\n\
        - Use this tool to check if a parallel agent has finished its work."
    }
    fn category(&self) -> ToolCategory { ToolCategory::System }
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
        let task_id = params["task_id"].as_str().ok_or(ToolError::InvalidParams("task_id required".into()))?;
        let executor_arc = BACKGROUND_TASKS.get(task_id)
            .ok_or_else(|| ToolError::ExecutionFailed(format!("Task {} not found", task_id)))?;
        let guard = executor_arc.lock().await;
        
        match guard.state {
            crate::workflow::react::types::WorkflowState::Completed => {
                let mut result = "Task completed successfully. Output: ".to_string();
                if let Some(last_msg) = guard.context.messages.iter().rev().find(|m| m.role == "assistant") {
                    result.push_str(&last_msg.message);
                }
                Ok(ToolCallResult::success(Some(result), None))
            },
            crate::workflow::react::types::WorkflowState::Error => {
                Ok(ToolCallResult::success(Some("Task failed with error.".to_string()), None))
            },
            _ => {
                Ok(ToolCallResult::success(Some(format!("Task is still in progress (State: {:?}).", guard.state)), None))
            }
        }
    }
}

/// Tool to stop a background agent (Full Claude Code Clone)
pub struct TaskStopTool;

#[async_trait]
impl ToolDefinition for TaskStopTool {
    fn name(&self) -> &str { "task_stop" }
    fn description(&self) -> &str { 
        "- Stops a running background task by its ID\n\
        - Takes a task_id parameter identifying the task to stop\n\
        - Returns a success or failure status\n\
        - Use this tool when you need to terminate a long-running task"
    }
    fn category(&self) -> ToolCategory { ToolCategory::System }
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
        let task_id = params["task_id"].as_str().ok_or(ToolError::InvalidParams("task_id required".into()))?;
        if let Some((_, executor_arc)) = BACKGROUND_TASKS.remove(task_id) {
            let mut guard = executor_arc.lock().await;
            guard.state = crate::workflow::react::types::WorkflowState::Completed; 
            Ok(ToolCallResult::success(Some(format!("Task {} has been stopped.", task_id)), None))
        } else {
            Err(ToolError::ExecutionFailed(format!("Task {} not found", task_id)))
        }
    }
}
