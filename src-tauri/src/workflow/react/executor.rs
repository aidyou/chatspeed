use crate::ai::chat::openai::OpenAIChat;
use crate::ai::interaction::chat_completion::{AiChatEnum, ChatState};
use crate::db::{Agent, MainStore};
use crate::tools::{
    ToolManager, TOOL_ANSWER_USER, TOOL_ASK_USER, TOOL_BASH, TOOL_EDIT_FILE, TOOL_FINISH_TASK,
    TOOL_LIST_DIR, TOOL_READ_FILE, TOOL_WEB_FETCH, TOOL_WRITE_FILE,
};
use crate::workflow::react::compression::ContextCompressor;
use crate::workflow::react::context::ContextManager;
use crate::workflow::react::error::WorkflowEngineError;
use crate::workflow::react::gateway::Gateway;
use crate::workflow::react::observation::ObservationReinforcer;
use crate::workflow::react::security::PathGuard;
use crate::workflow::react::types::{GatewayPayload, StepType, WorkflowState};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

use crate::workflow::react::orchestrator::SubAgentFactory;
use crate::workflow::react::skills::{SkillManifest, SkillScanner};

pub const CORE_SYSTEM_PROMPT: &str = r#"You are an autonomous AI Agent operating in a tool-use environment.
Your core philosophy is: **Everything is a tool call.**

## OPERATIONAL GUIDELINES:
1. **Tool-First Thinking**: For every response, you MUST call at least one tool. If you are thinking, use 'think' internally but ensure the final output of the turn includes a tool call.
2. **Persistence**: Do not stop until the task is fully complete. Use `todo_*` tools to track your own progress.
3. **Structured Snapshot**: You will receive a <state_snapshot> in the context. Always respect the decisions and facts recorded there.
4. **Communication**: To talk to the user, you MUST use `answer_user` or `ask_user`. To finish, use `finish_task`.
5. **No Conversational Filler**: Do not provide conversational responses without tools. If you have nothing to do, call `finish_task` with a summary."#;

pub struct WorkflowExecutor {
    pub session_id: String,
    pub context: ContextManager,
    pub tool_manager: Arc<ToolManager>,
    pub global_tool_manager: Arc<ToolManager>,
    pub chat_state: Arc<ChatState>,
    pub gateway: Arc<dyn Gateway>,
    pub sub_agent_factory: Arc<dyn SubAgentFactory>,
    pub compressor: ContextCompressor,
    pub path_guard: Arc<PathGuard>,
    pub skill_scanner: SkillScanner,
    pub available_skills: HashMap<String, SkillManifest>,
    pub agent_config: Agent,
    pub state: WorkflowState,
    pub max_steps: usize,
    pub current_step: usize,
    pub auto_approve: HashSet<String>,
    pub signal_rx: tokio::sync::mpsc::Receiver<String>,
    pub tsid_generator: Arc<crate::libs::tsid::TsidGenerator>,
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
        tsid_generator: Arc<crate::libs::tsid::TsidGenerator>,
        global_tool_manager: Arc<ToolManager>,
    ) -> Self {
        let max_contexts = agent_config.max_contexts.unwrap_or(128000) as usize;
        let context = ContextManager::new(
            session_id.clone(),
            main_store,
            max_contexts,
            tsid_generator.clone(),
        );

        let models_val: serde_json::Value = if let Some(m) = &agent_config.models {
            serde_json::from_str(m).unwrap_or_default()
        } else {
            serde_json::json!({
                "plan": agent_config.plan_model.as_deref().and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok()),
                "act": agent_config.act_model.as_deref().and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok()),
                "vision": agent_config.vision_model.as_deref().and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok()),
            })
        };

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
            "Writing" => models_val
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
            global_tool_manager,
            chat_state,
            gateway,
            sub_agent_factory,
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
            tsid_generator,
            active_provider_id: provider_id,
            active_model_name: model_name,
        }
    }

    pub async fn init(&mut self) -> Result<(), WorkflowEngineError> {
        self.context.load_history().await?;
        self.available_skills = self.skill_scanner.scan()?;
        self.register_foundation_tools().await?;
        if let Some(last_msg) = self.context.messages.last() {
            self.current_step = last_msg.step_index as usize;
        }
        self.update_state(WorkflowState::Thinking).await?;
        Ok(())
    }

    async fn register_foundation_tools(&self) -> Result<(), WorkflowEngineError> {
        use crate::tools::*;
        let tm = &self.tool_manager;

        // Use global tool manager to discover available tools and check scopes
        let all_meta = self
            .global_tool_manager
            .get_all_native_tool_metadata()
            .await;

        // Helper to check if a tool is allowed in Workflow scope
        let is_allowed = |name: &str| {
            all_meta
                .iter()
                .find(|m| m["id"] == name)
                .map(|m| {
                    let scope = m["scope"].as_str().unwrap_or("both");
                    scope == "workflow" || scope == "both"
                })
                .unwrap_or(true) // Default to allowed for safety if not found in meta
        };

        // 1. Native FS & Search Tools
        if is_allowed(TOOL_READ_FILE) {
            tm.register_tool(Arc::new(ReadFile)).await?;
        }
        if is_allowed(TOOL_WRITE_FILE) {
            tm.register_tool(Arc::new(WriteFile)).await?;
        }
        if is_allowed(TOOL_EDIT_FILE) {
            tm.register_tool(Arc::new(EditFile)).await?;
        }
        if is_allowed(TOOL_LIST_DIR) {
            tm.register_tool(Arc::new(ListDir)).await?;
        }
        if is_allowed(TOOL_GLOB) {
            tm.register_tool(Arc::new(Glob)).await?;
        }
        if is_allowed(TOOL_GREP) {
            tm.register_tool(Arc::new(Grep)).await?;
        }

        // 2. Shell Tool (With session-aware policy)
        if is_allowed(TOOL_BASH) {
            let custom_rules: Vec<ShellPolicyRule> = self
                .agent_config
                .shell_policy
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_default();

            tm.register_tool(Arc::new(ShellExecute::new(
                self.path_guard.clone(),
                self.tsid_generator.clone(),
                custom_rules,
            )))
            .await?;
        }

        // 3. Interaction Tools
        if is_allowed(TOOL_ANSWER_USER) {
            tm.register_tool(Arc::new(AnswerUser)).await?;
        }
        if is_allowed(TOOL_ASK_USER) {
            tm.register_tool(Arc::new(AskUser)).await?;
        }
        if is_allowed(TOOL_FINISH_TASK) {
            tm.register_tool(Arc::new(FinishTask)).await?;
        }

        // 4. Multi-Agent & Skill Tools
        tm.register_tool(Arc::new(SkillExecute::new(self.available_skills.clone())))
            .await?;
        tm.register_tool(Arc::new(
            crate::workflow::react::orchestrator::TaskTool::new(
                self.sub_agent_factory.clone(),
                self.tsid_generator.clone(),
            ),
        ))
        .await?;
        tm.register_tool(Arc::new(
            crate::workflow::react::orchestrator::TaskOutputTool,
        ))
        .await?;
        tm.register_tool(Arc::new(crate::workflow::react::orchestrator::TaskStopTool))
            .await?;

        // 5. Todo Manager Tools (Session Persistent)
        tm.register_tool(Arc::new(TodoCreateTool {
            session_id: self.session_id.clone(),
            main_store: self.context.main_store.clone(),
        }))
        .await?;
        tm.register_tool(Arc::new(TodoListTool {
            session_id: self.session_id.clone(),
            main_store: self.context.main_store.clone(),
        }))
        .await?;
        tm.register_tool(Arc::new(TodoUpdateTool {
            session_id: self.session_id.clone(),
            main_store: self.context.main_store.clone(),
        }))
        .await?;
        tm.register_tool(Arc::new(TodoGetTool {
            session_id: self.session_id.clone(),
            main_store: self.context.main_store.clone(),
        }))
        .await?;

        Ok(())
    }

    pub async fn run_loop(&mut self) -> Result<(), WorkflowEngineError> {
        while self.state != WorkflowState::Completed && self.state != WorkflowState::Error {
            if self.state == WorkflowState::Paused {
                let signal_str =
                    self.signal_rx.recv().await.ok_or_else(|| {
                        WorkflowEngineError::Gateway("Signal channel closed".into())
                    })?;

                let signal_json: serde_json::Value = serde_json::from_str(&signal_str)
                    .unwrap_or(serde_json::json!({ "type": "message", "content": signal_str }));

                if signal_json["type"] == "approval" {
                    let approved = signal_json["approved"].as_bool().unwrap_or(false);
                    let tool_name = signal_json["tool_name"].as_str().unwrap_or("unknown");
                    let tool_args = signal_json["tool_args"].clone();

                    if approved {
                        log::info!(
                            "WorkflowExecutor {}: User APPROVED tool '{}'",
                            self.session_id,
                            tool_name
                        );
                        // Execution: Try local then global
                        let result = if let Ok(res) = self
                            .tool_manager
                            .tool_call(tool_name, tool_args.clone())
                            .await
                        {
                            Ok(res)
                        } else {
                            self.global_tool_manager
                                .tool_call(tool_name, tool_args)
                                .await
                        };
                        let observation = ObservationReinforcer::reinforce(tool_name, &result);
                        self.add_message_and_notify(
                            "tool".to_string(),
                            observation,
                            Some(StepType::Observe),
                            None,
                        )
                        .await?;
                    } else {
                        log::info!(
                            "WorkflowExecutor {}: User REJECTED tool '{}'",
                            self.session_id,
                            tool_name
                        );
                        let observation = format!("Error: User rejected the execution of tool '{}'. Please try an alternative approach or ask for clarification.", tool_name);
                        self.add_message_and_notify(
                            "tool".to_string(),
                            observation,
                            Some(StepType::Observe),
                            None,
                        )
                        .await?;
                    }
                    self.update_state(WorkflowState::Thinking).await?;
                    continue;
                } else {
                    let user_input = signal_json["content"].as_str().unwrap_or("").to_string();
                    self.add_message_and_notify("user".to_string(), user_input, None, None)
                        .await?;
                    self.update_state(WorkflowState::Thinking).await?;
                    continue;
                }
            }
            if self.current_step >= self.max_steps {
                self.update_state(WorkflowState::Error).await?;
                return Err(WorkflowEngineError::General(
                    "Max steps exceeded".to_string(),
                ));
            }
            self.current_step += 1;
            self.update_state(WorkflowState::Thinking).await?;
            log::debug!(
                "[Workflow Engine] Session {}: Step {} - Calling LLM",
                self.session_id,
                self.current_step
            );
            let (response, usage) = self.call_llm_with_retry().await?;
            log::debug!(
                "[Workflow Engine] Session {}: LLM responded ({} chars)",
                self.session_id,
                response.len()
            );
            self.add_message_and_notify(
                "assistant".to_string(),
                response.clone(),
                Some(StepType::Think),
                usage.map(|u| serde_json::json!({ "usage": u })),
            )
            .await?;

            self.update_state(WorkflowState::Executing).await?;
            log::debug!(
                "[Workflow Engine] Session {}: Executing tools",
                self.session_id
            );
            let tool_results = self.execute_tools(response).await?;

            if tool_results.is_empty() {
                log::warn!(
                    "WorkflowExecutor {}: No tool call detected. Forcing retry.",
                    self.session_id
                );
                self.add_message_and_notify(
                    "user".to_string(),
                    "Error: No tool call detected in your last response. You MUST call a tool to proceed, even if it is just 'answer_user' or 'finish_task'.".to_string(),
                    Some(StepType::Observe),
                    None
                ).await?;
                continue;
            }

            let mut needs_compression = false;
            for (tool_call_id, result) in tool_results {
                log::debug!(
                    "[Workflow Engine] Session {}: Recording observation for {}",
                    self.session_id,
                    tool_call_id
                );
                let compressed_signal = self
                    .add_message_and_notify(
                        "tool".to_string(),
                        result,
                        Some(StepType::Observe),
                        Some(serde_json::json!({ "tool_call_id": tool_call_id })),
                    )
                    .await?;
                if compressed_signal {
                    needs_compression = true;
                }
            }
            while let Ok(interrupt_str) = self.signal_rx.try_recv() {
                let sig_json: serde_json::Value = serde_json::from_str(&interrupt_str).unwrap_or(
                    serde_json::json!({ "type": "user_input", "content": interrupt_str }),
                );

                let content = if sig_json["type"] == "user_input" {
                    sig_json["content"]
                        .as_str()
                        .unwrap_or(&interrupt_str)
                        .to_string()
                } else {
                    interrupt_str
                };

                self.add_message_and_notify("user".to_string(), content, None, None)
                    .await?;
            }
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
            // --- 1. Check for explicit Stop/Cancel state ---
            if self.state == WorkflowState::Completed
                || self.state == WorkflowState::Error
                || self.state == WorkflowState::Cancelled
            {
                return Err(WorkflowEngineError::General(
                    "Execution stopped by user".into(),
                ));
            }

            // --- 2. Check signal channel for "stop" commands ---
            while let Ok(sig_str) = self.signal_rx.try_recv() {
                if sig_str.to_lowercase().contains("stop") || sig_str.contains("\"stop\"") {
                    log::info!(
                        "WorkflowExecutor {}: Received STOP signal during retry loop",
                        self.session_id
                    );
                    self.update_state(WorkflowState::Cancelled).await?;
                    return Err(WorkflowEngineError::General(
                        "Stopped by user signal".into(),
                    ));
                }
            }

            match self.call_llm().await {
                Ok(res) => return Ok(res),
                Err(e) => {
                    log::warn!(
                        "WorkflowExecutor {}: LLM call failed (attempt {}): {}",
                        self.session_id,
                        retry_count + 1,
                        e
                    );

                    // --- 3. Fatal Error Detection (401/403) ---
                    let err_msg = e.to_string();
                    if err_msg.contains("401")
                        || err_msg.contains("Unauthorized")
                        || err_msg.contains("403")
                    {
                        log::error!("WorkflowExecutor {}: Fatal authentication error detected. Terminating.", self.session_id);
                        self.update_state(WorkflowState::Error).await?;
                        return Err(e);
                    }

                    last_error = Some(e);
                    retry_count += 1;
                    if retry_count < max_retries {
                        sleep(Duration::from_secs(2u64.pow(retry_count as u32))).await;
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
        let raw_history = self.context.get_messages_for_llm();
        let mut history: Vec<serde_json::Value> = Vec::new();

        // --- 1. Context Sanitization & Normalization ---
        for m in raw_history {
            let role = m.role.clone();
            let mut content = m.message.clone();
            let mut tool_calls = m
                .metadata
                .as_ref()
                .and_then(|meta| meta.get("tool_calls").cloned());

            // Normalize Assistant JSON (if content is raw JSON, parse it)
            if role == "assistant" && content.trim().starts_with('{') {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(c) = parsed.get("content").and_then(|v| v.as_str()) {
                        content = c.to_string();
                    }
                    if let Some(tc) = parsed.get("tool_calls").cloned() {
                        tool_calls = Some(tc);
                    }
                }
            }

            // Standardize Tool Result messages
            let tool_call_id = if role == "tool" {
                m.metadata
                    .as_ref()
                    .and_then(|meta| meta.get("tool_call_id").and_then(|v| v.as_str()))
                    .map(|s| s.to_string())
            } else {
                None
            };

            // Logic: Merge consecutive roles
            if let Some(last) = history.last_mut() {
                if last["role"] == role && role != "tool" {
                    // Append content to last message of same role
                    let last_content = last["content"].as_str().unwrap_or("");
                    last["content"] = serde_json::json!(format!("{}\n\n{}", last_content, content));
                    continue;
                }
            }

            // Build structured message
            let mut msg = serde_json::json!({
                "role": role,
                "content": content,
            });

            if let Some(tc) = tool_calls {
                msg["tool_calls"] = tc;
            }
            if let Some(tid) = tool_call_id {
                msg["tool_call_id"] = serde_json::json!(tid);
            }

            history.push(msg);
        }

        // --- 2. Tool Chain Integrity Check ---
        // Some providers fail if assistant has tool_calls but no following tool messages.
        // We scan backwards and fix dangling assistant tool calls.
        let mut final_history = Vec::new();
        for i in 0..history.len() {
            let msg = &history[i];
            if msg["role"] == "assistant" && msg.get("tool_calls").is_some() {
                // Peek ahead: is next message a 'tool'?
                let has_results = history
                    .get(i + 1)
                    .map_or(false, |next| next["role"] == "tool");
                if !has_results {
                    // Strip tool_calls if no results follow, otherwise providers like ModelScope crash
                    let mut cleaned_msg = msg.clone();
                    cleaned_msg
                        .as_object_mut()
                        .map(|obj| obj.remove("tool_calls"));
                    final_history.push(cleaned_msg);
                    continue;
                }
            }
            final_history.push(msg.clone());
        }
        history = final_history;

        // --- 3. System Prompt & Environment Reminders ---
        let mut full_system_prompt = String::from(CORE_SYSTEM_PROMPT);
        full_system_prompt.push_str("\n\n## AGENT SPECIFIC INSTRUCTIONS:\n");
        full_system_prompt.push_str(&self.agent_config.system_prompt);
        if let Some(sys_msg) = history.iter_mut().find(|m| m["role"] == "system") {
            let original = sys_msg["content"].as_str().unwrap_or("");
            sys_msg["content"] =
                serde_json::json!(format!("{}\n\n{}", full_system_prompt, original));
        } else {
            history.insert(
                0,
                serde_json::json!({ "role": "system", "content": full_system_prompt }),
            );
        }

        let mut reminders = String::new();
        if !self.available_skills.is_empty() {
            reminders.push_str(
                "<system-reminder>\nThe following skills are available for use with the Skill tool:\n\n",
            );
            for skill in self.available_skills.values() {
                reminders.push_str(&format!("- {}: {}\n", skill.name, skill.description));
            }
            reminders.push_str("</system-reminder>\n");
        }

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

        let mut env_info = format!("Environment\nYou have been invoked in the following environment:\n - Primary working directory: {:?}\n  - Is a git repository: {}\n", cwd, is_git);
        let allowed_roots = self.path_guard.allowed_roots();
        if allowed_roots.len() > 1 || (allowed_roots.len() == 1 && allowed_roots[0] != cwd) {
            env_info.push_str(" - Additional working directories:\n");
            for root in allowed_roots {
                if root != cwd {
                    env_info.push_str(&format!("  - {:?}\n", root));
                }
            }
        }
        env_info.push_str(&format!(
            " - Platform: {}\n - Shell: {}\n - OS Version: {}\n",
            platform, shell, os_version
        ));

        reminders.push_str(&format!("<system-reminder>\n{}\n\nAs you answer the user's questions, you can use the following context:\n# currentDate\nToday's date is {}.\n\nIMPORTANT: this context may or may not be relevant to your tasks. You should not respond to this context unless it is highly relevant to your task.\n</system-reminder>\n", env_info, today));

        if let Some(first_user_msg) = history.iter_mut().find(|m| m["role"] == "user") {
            if let Some(content) = first_user_msg["content"].as_str() {
                first_user_msg["content"] = serde_json::json!(format!("{}{}", reminders, content));
            }
        } else {
            history.insert(
                0,
                serde_json::json!({ "role": "system", "content": reminders }),
            );
        }

        // Get tools from BOTH managers for LLM spec
        let mut tools = self.tool_manager.get_tool_calling_spec(None, None).await?;
        let global_tools = self
            .global_tool_manager
            .get_tool_calling_spec(None, None)
            .await?;
        tools.extend(global_tools);

        // // 1. Resolve Metadata (Provider vs Proxy)
        // let mut metadata = if self.active_provider_id > 0 {
        //     // Provider Mode: Fetch metadata from DB to pass along custom parameters
        //     let store = self
        //         .chat_state
        //         .main_store
        //         .read()
        //         .map_err(|e| WorkflowEngineError::General(e.to_string()))?;
        //     store
        //         .config
        //         .get_ai_model_by_id(self.active_provider_id)
        //         .map(|m| m.metadata.clone().unwrap_or(serde_json::json!({})))
        //         .unwrap_or(serde_json::json!({}))
        // } else {
        //     // Proxy Mode: Metadata is handled internally by ccproxy via group@alias
        //     serde_json::json!({})
        // };

        // // Always ensure streaming is enabled for workflow feedback
        // if let Some(obj) = metadata.as_object_mut() {
        //     obj.insert("stream".to_string(), serde_json::json!(true));
        //     // Ensure tools are enabled in the proxy layer
        //     obj.insert("toolsEnabled".to_string(), serde_json::json!(true));
        // }

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
        while let Some(chunk) = rx.recv().await {
            match chunk.r#type {
                crate::ai::traits::chat::MessageType::Text
                | crate::ai::traits::chat::MessageType::Reasoning => {
                    // Send chunk to UI immediately for "Thinking" feedback
                    self.gateway
                        .send(
                            &self.session_id,
                            GatewayPayload::Chunk {
                                content: chunk.chunk.clone(),
                            },
                        )
                        .await?;

                    full_content.push_str(&chunk.chunk);
                }
                crate::ai::traits::chat::MessageType::ToolCalls => {
                    // Collect tool calls silently, don't stream them as raw text to user
                    full_content.push_str(&chunk.chunk);
                }
                crate::ai::traits::chat::MessageType::Finished => {
                    break;
                }
                crate::ai::traits::chat::MessageType::Error => {
                    return Err(WorkflowEngineError::General(chunk.chunk.clone()))
                }
                _ => {}
            }
        }
        Ok((full_content, None))
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

        // --- 1. Graded Shell Auditing ---
        if name == TOOL_BASH {
            let command_str = args["command"].as_str().unwrap_or("");
            if !self.auto_approve.contains(name) {
                let custom_rules: Vec<crate::tools::ShellPolicyRule> = self
                    .agent_config
                    .shell_policy
                    .as_deref()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or_default();
                let policy_engine =
                    crate::tools::ShellPolicyEngine::new(self.path_guard.clone(), custom_rules);

                match policy_engine.check(command_str) {
                    crate::tools::ShellDecision::Allow => {}
                    crate::tools::ShellDecision::Deny(reason) => {
                        return Ok(format!(
                            "Error: Command blocked by security policy. Reason: {}",
                            reason
                        ));
                    }
                    crate::tools::ShellDecision::Review(reason) => {
                        self.gateway
                            .send(
                                &self.session_id,
                                GatewayPayload::Confirm {
                                    id: uuid::Uuid::new_v4().to_string(),
                                    action: TOOL_BASH.to_string(),
                                    details: format!("{} (Policy: {})", command_str, reason),
                                },
                            )
                            .await?;

                        self.update_state(WorkflowState::Paused).await?;
                        log::info!(
                            "WorkflowExecutor {}: Paused for shell command review",
                            self.session_id
                        );
                        return Ok("WAITING_FOR_USER_APPROVAL".to_string());
                    }
                }
            }
        }

        // --- 2. Generic Tool Safety (FS, etc.) ---
        if name == TOOL_WRITE_FILE || name == TOOL_EDIT_FILE {
            if !self.auto_approve.contains(name) {
                let details = args["file_path"].as_str().unwrap_or("").to_string();
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
                return Ok("WAITING_FOR_USER_APPROVAL".to_string());
            }
        }

        // --- 3. Path Guard for FS tools ---
        if name == TOOL_READ_FILE
            || name == TOOL_WRITE_FILE
            || name == TOOL_LIST_DIR
            || name == TOOL_EDIT_FILE
        {
            if let Some(path_str) = args["file_path"].as_str().or_else(|| args["path"].as_str()) {
                self.path_guard.validate(std::path::Path::new(path_str))?;
            }
        }

        // --- 3. Special Core Tools ---
        match name {
            n if n == TOOL_ANSWER_USER => return Ok("Summary delivered".into()),
            n if n == TOOL_ASK_USER => {
                self.update_state(WorkflowState::Paused).await?;
                return Ok("Waiting for user".into());
            }
            n if n == TOOL_FINISH_TASK => {
                self.update_state(WorkflowState::Completed).await?;
                return Ok("Finished".into());
            }
            _ => {}
        }

        // --- 4. Tool Call Dispatch (Local then Global) ---
        // First try the session-specific tool manager
        let result = if let Ok(res) = self.tool_manager.tool_call(name, args.clone()).await {
            Ok(res)
        } else {
            // Then try the global tool manager (for MCP, WebSearch, etc.)
            self.global_tool_manager.tool_call(name, args).await
        };

        // --- 5. Post-process: Auto-summarize large web_fetch content ---
        if name == TOOL_WEB_FETCH {
            if let Ok(val) = &result {
                if let Some(content) = val.get("content").and_then(|v| v.as_str()) {
                    if content.len() > 5000 {
                        log::info!("WorkflowExecutor {}: Content from web_fetch is too long ({} chars). Summarizing...", self.session_id, content.len());
                        match self.summarize_content(content).await {
                            Ok(summary) => {
                                return Ok(format!("[Auto-Summarized Result] {}\n\n<system-reminder>The original content was over 5000 characters and has been summarized to focus on relevant information.</system-reminder>", summary));
                            }
                            Err(e) => {
                                log::error!(
                                    "WorkflowExecutor {}: Summarization failed: {}",
                                    self.session_id,
                                    e
                                );
                                // Fallback to reinforced result (which will truncate)
                            }
                        }
                    }
                }
            }
        }

        Ok(ObservationReinforcer::reinforce(name, &result))
    }

    /// Summarizes or filters large text content with high-fidelity requirements
    async fn summarize_content(&mut self, content: &str) -> Result<String, WorkflowEngineError> {
        let user_query = self
            .context
            .messages
            .first()
            .map(|m| m.message.clone())
            .unwrap_or_else(|| "current task".to_string());

        let prompt = format!(
            "Analyze and filter the following web content relative to the user's initial query. \n\n\
            ## FILTERING CRITERIA:\n\
            1. **STRICT PRESERVATION**: You MUST extract the following information EXACTLY as it appears in the source:\n\
               - Financial data (Revenue, EBITDA, Ratios, Currency values).\n\
               - Specific dates, times, and quantitative metrics.\n\
               - Legal terms, specific names, and technical specifications.\n\
            2. **RELEVANT EXTRACTION**: For prose and explanations, extract the specific sentences or paragraphs most relevant to the query. Prefer verbatim quotes over rephrasing to maintain evidence quality.\n\
            3. **CULLING**: Discard all boilerplate, ads, navigation links, and repetitive background information not directly answering the query.\n\n\
            <cs:user_query>\n{}\n</cs:user_query>\n\n\
            <cs:source_content>\n{}\n</cs:source_content>",
            user_query, content
        );

        let messages = vec![
            serde_json::json!({
                "role": "system",
                "content": "You are a high-fidelity information filter. Your task is to process source content into a concise, evidence-dense summary while ensuring all critical numeric and factual data is preserved with 100% accuracy."
            }),
            serde_json::json!({ "role": "user", "content": prompt }),
        ];

        let chat_interface = {
            let mut chats_guard = self.chat_state.chats.lock().await;
            let protocol = crate::ccproxy::ChatProtocol::OpenAI;
            let chat_map = chats_guard
                .entry(protocol)
                .or_insert_with(std::collections::HashMap::new);
            chat_map
                .entry(self.session_id.clone() + "_summarizer")
                .or_insert_with(|| crate::create_chat!(self.context.main_store))
                .clone()
        };

        let (tx, mut rx) = tokio::sync::mpsc::channel(100);
        chat_interface
            .chat(
                self.active_provider_id,
                &self.active_model_name,
                self.session_id.clone() + "_summarizer",
                messages,
                None,
                None,
                move |chunk| {
                    let _ = tx.try_send(chunk);
                },
            )
            .await
            .map_err(WorkflowEngineError::Ai)?;

        let mut summary = String::new();
        while let Some(chunk) = rx.recv().await {
            if chunk.r#type == crate::ai::traits::chat::MessageType::Text {
                summary.push_str(&chunk.chunk);
            } else if chunk.r#type == crate::ai::traits::chat::MessageType::Finished {
                break;
            }
        }

        Ok(summary)
    }

    async fn update_state(&mut self, new_state: WorkflowState) -> Result<(), WorkflowEngineError> {
        self.state = new_state.clone();
        self.gateway
            .send(&self.session_id, GatewayPayload::State { state: new_state })
            .await?;
        Ok(())
    }

    async fn add_message_and_notify(
        &mut self,
        role: String,
        mut content: String,
        step_type: Option<StepType>,
        mut metadata: Option<serde_json::Value>,
    ) -> Result<bool, WorkflowEngineError> {
        // --- 1. Content Normalization (Handle JSON responses from LLM) ---
        if role == "assistant" && content.trim().starts_with('{') {
            if let Ok(json_msg) = serde_json::from_str::<serde_json::Value>(&content) {
                // Extract actual text content if it exists
                if let Some(text) = json_msg.get("content").and_then(|v| v.as_str()) {
                    content = text.to_string();
                }

                // Extract tool calls and merge into metadata
                if let Some(tool_calls) = json_msg.get("tool_calls").and_then(|v| v.as_array()) {
                    let mut meta_obj = metadata.unwrap_or(serde_json::json!({}));
                    meta_obj["tool_calls"] = serde_json::json!(tool_calls);
                    metadata = Some(meta_obj);
                }
            }
        }

        let needs_compression = self
            .context
            .add_message(
                role.clone(),
                content.clone(),
                step_type.clone(),
                self.current_step as i32,
                metadata.clone(),
            )
            .await?;

        // Emit to gateway for UI real-time update
        self.gateway
            .send(
                &self.session_id,
                GatewayPayload::Message {
                    role,
                    content,
                    step_type,
                    step_index: self.current_step as i32,
                    metadata,
                },
            )
            .await?;

        Ok(needs_compression)
    }
}
