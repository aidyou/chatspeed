use crate::ai::chat::openai::OpenAIChat;
use crate::ai::interaction::chat_completion::{AiChatEnum, ChatState};
use crate::db::{Agent, MainStore};
use crate::tools::ToolManager;
use crate::workflow::react::compression::ContextCompressor;
use crate::workflow::react::context::ContextManager;
use crate::workflow::react::error::WorkflowEngineError;
use crate::workflow::react::gateway::Gateway;
use crate::workflow::react::observation::ObservationReinforcer;
use crate::workflow::react::security::PathGuard;
use crate::workflow::react::stream_parser::StreamParser;
use crate::workflow::react::types::{GatewayPayload, StepType, WorkflowState};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

use crate::tools::{TodoCreateTool, TodoGetTool, TodoListTool, TodoUpdateTool};
use crate::workflow::react::orchestrator::{
    SubAgentFactory, TaskOutputTool, TaskStopTool, TaskTool,
};
use crate::workflow::react::skills::{SkillManifest, SkillScanner};

pub struct WorkflowExecutor {
    pub session_id: String,
    pub context: ContextManager,
    pub tool_manager: Arc<ToolManager>,
    pub chat_state: Arc<ChatState>,
    pub gateway: Arc<dyn Gateway>,
    pub sub_agent_factory: Arc<dyn SubAgentFactory>,
    pub stream_parser: StreamParser,
    pub compressor: ContextCompressor,
    pub path_guard: Arc<PathGuard>,
    pub skill_scanner: SkillScanner,
    pub available_skills: HashMap<String, SkillManifest>,
    pub agent_config: Agent,
    pub state: WorkflowState,
    pub max_steps: usize,
    pub current_step: usize,
    pub auto_approve: HashSet<String>,
    /// Receiver for user messages and signals (queue)
    pub signal_rx: tokio::sync::mpsc::Receiver<String>,
    /// Active model metadata
    pub active_provider_id: i64,
    pub active_model_name: String,
}

impl WorkflowExecutor {
    pub fn new(
        session_id: String,
        main_store: Arc<std::sync::RwLock<MainStore>>,
        chat_state: Arc<ChatState>,
        gateway: Arc<dyn Gateway>,
        sub_agent_factory: Arc<dyn SubAgentFactory>,
        agent_config: Agent,
        allowed_paths: Vec<PathBuf>,
        app_data_dir: PathBuf,
        subagent_type: Option<String>,
        signal_rx: tokio::sync::mpsc::Receiver<String>,
    ) -> Self {
        let max_contexts = agent_config.max_contexts.unwrap_or(128000) as usize;
        let context = ContextManager::new(session_id.clone(), main_store, max_contexts);
        let stream_parser = StreamParser::new(session_id.clone(), gateway.clone());

        // 1. Parse unified 'models' JSON or reconstruct from legacy fields
        let models_val: serde_json::Value = if let Some(m) = &agent_config.models {
            serde_json::from_str(m).unwrap_or_default()
        } else {
            serde_json::json!({
                "plan": agent_config.plan_model.as_deref().and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok()),
                "act": agent_config.act_model.as_deref().and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok()),
                "vision": agent_config.vision_model.as_deref().and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok()),
            })
        };

        // 2. Select the active model based on subagent_type with intelligent fallback
        let target_type = subagent_type.unwrap_or_else(|| "General".to_string());
        let act_model = models_val.get("act").cloned().unwrap_or_default();

        let active_model = match target_type.as_str() {
            "Programming" => models_val
                .get("coding")
                .cloned()
                .filter(|m| !m["model"].is_null())
                .unwrap_or_else(|| act_model.clone()),
            "Vision" => models_val
                .get("vision")
                .cloned()
                .filter(|m| !m["model"].is_null())
                .unwrap_or_else(|| act_model.clone()),
            "Copywriting" => models_val
                .get("copywriting")
                .cloned()
                .filter(|m| !m["model"].is_null())
                .unwrap_or_else(|| act_model.clone()),
            "Browsing" => models_val
                .get("browsing")
                .cloned()
                .filter(|m| !m["model"].is_null())
                .unwrap_or_else(|| act_model.clone()),
            _ => act_model.clone(),
        };

        let provider_id = active_model["id"].as_i64().unwrap_or(0);
        let model_name = active_model["model"]
            .as_str()
            .unwrap_or("gpt-4o")
            .to_string();

        // 3. Initialize components with the resolved model
        let compressor =
            ContextCompressor::new(chat_state.clone(), provider_id, model_name.clone());
        let path_guard = Arc::new(PathGuard::new(allowed_paths));
        let skill_scanner = SkillScanner::new(app_data_dir);
        let tool_manager = Arc::new(ToolManager::new());

        let auto_approve: HashSet<String> =
            serde_json::from_str(agent_config.auto_approve.as_deref().unwrap_or("[]"))
                .unwrap_or_default();

        Self {
            session_id,
            context,
            tool_manager,
            chat_state,
            gateway,
            sub_agent_factory,
            stream_parser,
            compressor,
            path_guard,
            skill_scanner,
            available_skills: HashMap::new(),
            agent_config,
            state: WorkflowState::Pending,
            max_steps: 15,
            current_step: 0,
            auto_approve,
            signal_rx,
            active_provider_id: provider_id,
            active_model_name: model_name,
        }
    }

    pub async fn init(&mut self) -> Result<(), WorkflowEngineError> {
        self.context.load_history().await?;

        // 1. Scan and store skills
        self.available_skills = self.skill_scanner.scan()?;
        for name in self.available_skills.keys() {
            log::info!(
                "WorkflowExecutor {}: Found skill '{}'",
                self.session_id,
                name
            );
        }

        // 2. Register tools (passing skills to the Skill tool)
        self.register_foundation_tools().await?;

        if let Some(last_msg) = self.context.messages.last() {
            self.current_step = last_msg.step_index as usize;
        }
        self.update_state(WorkflowState::Thinking).await?;
        Ok(())
    }

    async fn register_foundation_tools(&self) -> Result<(), WorkflowEngineError> {
        use crate::tools::*;
        use crate::workflow::react::orchestrator::*;
        let tm = &self.tool_manager;
        tm.register_tool(Arc::new(ReadFile)).await?;
        tm.register_tool(Arc::new(WriteFile)).await?;
        tm.register_tool(Arc::new(EditFile)).await?;
        tm.register_tool(Arc::new(ListDir)).await?;
        tm.register_tool(Arc::new(Glob)).await?;
        tm.register_tool(Arc::new(Grep)).await?;
        tm.register_tool(Arc::new(ShellExecute)).await?;
        tm.register_tool(Arc::new(AnswerUser)).await?;
        tm.register_tool(Arc::new(AskUser)).await?;
        tm.register_tool(Arc::new(FinishTask)).await?;

        // Register the Skill execution tool
        tm.register_tool(Arc::new(SkillExecute::new(self.available_skills.clone())))
            .await?;

        // Register the Sub-agent Orchestration suite
        tm.register_tool(Arc::new(TaskTool::new(self.sub_agent_factory.clone())))
            .await?;
        tm.register_tool(Arc::new(TaskOutputTool)).await?;
        tm.register_tool(Arc::new(TaskStopTool)).await?;

        // Register the Session ToDo Management suite (renamed to todo_*)
        tm.register_tool(Arc::new(TodoCreateTool)).await?;
        tm.register_tool(Arc::new(TodoListTool)).await?;
        tm.register_tool(Arc::new(TodoUpdateTool)).await?;
        tm.register_tool(Arc::new(TodoGetTool)).await?;

        Ok(())
    }

    pub async fn run_loop(&mut self) -> Result<(), WorkflowEngineError> {
        while self.state != WorkflowState::Completed && self.state != WorkflowState::Error {
            // Check for explicit pause or stop from UI
            if self.state == WorkflowState::Paused {
                let user_input =
                    self.signal_rx.recv().await.ok_or_else(|| {
                        WorkflowEngineError::Gateway("Signal channel closed".into())
                    })?;
                self.context
                    .add_message(
                        "user".to_string(),
                        user_input,
                        None,
                        self.current_step as i32,
                        None,
                    )
                    .await?;
                self.update_state(WorkflowState::Thinking).await?;
                continue;
            }

            if self.current_step >= self.max_steps {
                self.update_state(WorkflowState::Error).await?;
                return Err(WorkflowEngineError::General(
                    "Max steps exceeded".to_string(),
                ));
            }

            self.current_step += 1;

            // 1. Thinking phase
            self.update_state(WorkflowState::Thinking).await?;
            let (response, usage) = self.call_llm_with_retry().await?;
            self.context
                .add_message(
                    "assistant".to_string(),
                    response.clone(),
                    Some(StepType::Think),
                    self.current_step as i32,
                    usage.map(|u| serde_json::json!({ "usage": u })),
                )
                .await?;

            // 2. Action phase
            self.update_state(WorkflowState::Executing).await?;
            let tool_results = self.execute_tools(response).await?;

            // 3. Observation phase
            let mut needs_compression = false;
            for (tool_call_id, result) in tool_results {
                let compressed_signal = self
                    .context
                    .add_message(
                        "tool".to_string(),
                        result,
                        Some(StepType::Observe),
                        self.current_step as i32,
                        Some(serde_json::json!({ "tool_call_id": tool_call_id })),
                    )
                    .await?;
                if compressed_signal {
                    needs_compression = true;
                }
            }

            // 4. --- Interrupt Check (Queue Handling) ---
            // After tool observation is complete, check if user sent messages during execution.
            while let Ok(interrupt_msg) = self.signal_rx.try_recv() {
                log::info!(
                    "WorkflowExecutor {}: Processing queued user message during cycle end",
                    self.session_id
                );
                self.context
                    .add_message(
                        "user".to_string(),
                        interrupt_msg,
                        None,
                        self.current_step as i32,
                        None,
                    )
                    .await?;
            }

            // 5. Context Compression if needed
            if needs_compression {
                let summary = self.compressor.compress(&self.context.messages).await?;
                self.context
                    .add_summary(summary, self.current_step as i32)
                    .await?;
            }
        }
        Ok(())
    }

    async fn call_llm_with_retry(
        &mut self,
    ) -> Result<(String, Option<serde_json::Value>), WorkflowEngineError> {
        let mut retry_count = 0;
        let max_retries = 10;
        let mut last_error = None;

        while retry_count < max_retries {
            match self.call_llm().await {
                Ok(res) => return Ok(res),
                Err(e) => {
                    log::warn!(
                        "WorkflowExecutor {}: LLM call failed (attempt {}): {}",
                        self.session_id,
                        retry_count + 1,
                        e
                    );
                    last_error = Some(e);
                    retry_count += 1;
                    if retry_count < max_retries {
                        let delay = Duration::from_secs(2u64.pow(retry_count as u32));
                        sleep(delay).await;
                    }
                }
            }
        }
        Err(last_error.unwrap_or(WorkflowEngineError::General(
            "Max retries exceeded".to_string(),
        )))
    }

    async fn call_llm(
        &mut self,
    ) -> Result<(String, Option<serde_json::Value>), WorkflowEngineError> {
        let mut history: Vec<serde_json::Value> = self
            .context
            .get_messages_for_llm()
            .into_iter()
            .map(|m| serde_json::json!({ "role": m.role, "content": m.message }))
            .collect();

        // --- Dynamic System Reminders Injection (Claude Code Style) ---
        let mut reminders = String::new();

        // 1. Skill List Reminder
        if !self.available_skills.is_empty() {
            reminders.push_str("<system-reminder>\nThe following skills are available for use with the Skill tool:\n\n");
            for skill in self.available_skills.values() {
                reminders.push_str(&format!("- {}: {}\n", skill.name, skill.description));
            }
            reminders.push_str("</system-reminder>\n");
        }

        // 2. Context/Date/Environment Reminder
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let cwd = std::env::current_dir().unwrap_or_default();
        let is_git = std::process::Command::new("git")
            .args(["rev-parse", "--is-inside-work-tree"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "true")
            .unwrap_or(false);
        
        let platform = std::env::consts::OS;
        let shell = if platform == "windows" {
            std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".into())
        } else {
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
        };

        let os_version = if platform == "windows" {
            std::process::Command::new("cmd")
                .args(["/c", "ver"])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_else(|_| "Windows".into())
        } else {
            std::process::Command::new("uname")
                .args(["-sr"])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_else(|_| platform.to_string())
        };

        let mut env_info = format!(
            "Environment\nYou have been invoked in the following environment:\n - Primary working directory: {:?}\n  - Is a git repository: {}\n",
            cwd, is_git
        );

        let allowed_roots = self.path_guard.allowed_roots();
        if allowed_roots.len() > 1 || (allowed_roots.len() == 1 && allowed_roots[0] != cwd) {
            env_info.push_str(" - Additional working directories:\n");
            for root in allowed_roots {
                if root != &cwd {
                    env_info.push_str(&format!("  - {:?}\n", root));
                }
            }
        }

        env_info.push_str(&format!(
            " - Platform: {}\n - Shell: {}\n - OS Version: {}\n",
            platform, shell, os_version
        ));

        reminders.push_str(&format!(
            "<system-reminder>\n{}\n\nAs you answer the user's questions, you can use the following context:\n# currentDate\nToday's date is {}.\n\nIMPORTANT: this context may or may not be relevant to your tasks. You should not respond to this context unless it is highly relevant to your task.\n</system-reminder>\n",
            env_info, today
        ));

        // 3. Perform Injection
        if !reminders.is_empty() {
            if let Some(first_user_msg) = history.iter_mut().find(|m| m["role"] == "user") {
                if let Some(content) = first_user_msg["content"].as_str() {
                    let new_content = format!("{}{}", reminders, content);
                    first_user_msg["content"] = serde_json::json!(new_content);
                }
            } else {
                history.insert(
                    0,
                    serde_json::json!({ "role": "system", "content": reminders }),
                );
            }
        }

        let tools = self.tool_manager.get_tool_calling_spec(None).await?;
        let chat_interface = {
            let mut chats_guard = self.chat_state.chats.lock().await;
            let protocol = crate::ccproxy::ChatProtocol::OpenAI;
            let chat_map = chats_guard
                .entry(protocol)
                .or_insert_with(std::collections::HashMap::new);
            chat_map
                .entry(self.session_id.clone())
                .or_insert_with(|| crate::create_chat!(self.context.main_store))
                .clone()
        };

        let (tx, mut rx) = tokio::sync::mpsc::channel(100);

        chat_interface
            .chat(
                self.active_provider_id,
                &self.active_model_name,
                self.session_id.clone(),
                history,
                Some(tools),
                None,
                move |chunk| {
                    let _ = tx.try_send(chunk);
                },
            )
            .await
            .map_err(WorkflowEngineError::Ai)?;

        let mut full_content = String::new();
        let mut usage = None;
        while let Some(chunk) = rx.recv().await {
            match chunk.r#type {
                crate::ai::traits::chat::MessageType::Text
                | crate::ai::traits::chat::MessageType::ToolCalls => {
                    self.stream_parser.push(&chunk.chunk).await;
                    full_content.push_str(&chunk.chunk);
                }
                crate::ai::traits::chat::MessageType::Finished => {
                    if let Some(meta) = &chunk.metadata {
                        if let Some(tokens) = meta.get(crate::ai::interaction::constants::TOKENS) {
                            usage = Some(tokens.clone());
                        }
                    }
                    break;
                }
                crate::ai::traits::chat::MessageType::Error => {
                    return Err(WorkflowEngineError::General(chunk.chunk.clone()))
                }
                _ => {}
            }
        }

        Ok((full_content, usage))
    }

    async fn execute_tools(
        &mut self,
        raw_response: String,
    ) -> Result<Vec<(String, String)>, WorkflowEngineError> {
        let cleaned_response = crate::libs::util::format_json_str(&raw_response);
        let json_val: serde_json::Value = serde_json::from_str(&cleaned_response).map_err(|e| {
            WorkflowEngineError::General(format!(
                "Failed to parse JSON: {}. Original: {}",
                e, raw_response
            ))
        })?;

        let mut results = vec![];

        if let Some(tool_obj) = json_val.get("tool") {
            let name = tool_obj.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = tool_obj
                .get("arguments")
                .cloned()
                .or_else(|| tool_obj.get("input").cloned())
                .unwrap_or(serde_json::json!({}));
            results.push((
                "call_single".to_string(),
                self.dispatch_tool(name, args).await?,
            ));
        } else if let Some(tool_calls) = json_val.get("tool_calls").and_then(|v| v.as_array()) {
            for call in tool_calls {
                let id = call.get("id").and_then(|v| v.as_str()).unwrap_or("unknown");
                let func = call
                    .get("function")
                    .cloned()
                    .unwrap_or_else(|| call.clone());
                let name = func.get("name").and_then(|v| v.as_str()).unwrap_or("");

                let args_raw = func
                    .get("arguments")
                    .cloned()
                    .or_else(|| func.get("input").cloned())
                    .unwrap_or(serde_json::json!({}));
                let args = if args_raw.is_string() {
                    serde_json::from_str(args_raw.as_str().unwrap()).unwrap_or_default()
                } else {
                    args_raw
                };

                results.push((id.to_string(), self.dispatch_tool(name, args).await?));
            }
        }
        Ok(results)
    }

    async fn dispatch_tool(
        &mut self,
        name: &str,
        args: serde_json::Value,
    ) -> Result<String, WorkflowEngineError> {
        log::info!(
            "WorkflowExecutor {}: Dispatching '{}'",
            self.session_id,
            name
        );

        // 1. Safety Whitelist & Confirmation
        if name == "bash" || name == "write_file" || name == "edit_file" {
            if !self.auto_approve.contains(name) {
                let details = match name {
                    "bash" => args["command"].as_str().unwrap_or("").to_string(),
                    "write_file" | "edit_file" => {
                        args["file_path"].as_str().unwrap_or("").to_string()
                    }
                    _ => "".into(),
                };
                self.gateway
                    .send(
                        &self.session_id,
                        GatewayPayload::Confirm {
                            id: uuid::Uuid::new_v4().to_string(),
                            action: name.to_string(),
                            details,
                        },
                    )
                    .await?;
                self.update_state(WorkflowState::Paused).await?;
                return Ok("Waiting for user approval".to_string());
            }
        }

        // 2. Path Guard for FS
        if name == "read_file" || name == "write_file" || name == "list_dir" || name == "edit_file"
        {
            if let Some(path_str) = args["file_path"].as_str() {
                self.path_guard.validate(std::path::Path::new(path_str))?;
            }
        }

        match name {
            "answer_user" => Ok("Summary delivered".into()),
            "ask_user" => {
                self.update_state(WorkflowState::Paused).await?;
                Ok("Waiting for user".into())
            }
            "finish_task" => {
                self.update_state(WorkflowState::Completed).await?;
                Ok("Finished".into())
            }
            _ => {
                let result = self.tool_manager.tool_call(name, args).await;
                Ok(ObservationReinforcer::reinforce(name, &result))
            }
        }
    }

    async fn update_state(&mut self, new_state: WorkflowState) -> Result<(), WorkflowEngineError> {
        self.state = new_state.clone();
        self.gateway
            .send(&self.session_id, GatewayPayload::State { state: new_state })
            .await?;
        Ok(())
    }
}
