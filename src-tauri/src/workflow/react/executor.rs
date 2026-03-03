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
use dirs;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tokio::time::{sleep, Duration};

/// Default maximum ReAct steps before the agent is forced to conclude.
const DEFAULT_MAX_STEPS: usize = 60;

/// Window size for the repetition detector (number of recent tool calls to inspect).
const LOOP_DETECT_WINDOW: usize = 8;

/// Minimum repeat count within the window that triggers a loop warning.
const LOOP_REPEAT_THRESHOLD: usize = 3;

/// Detects when the agent repeats the same tool call with identical arguments.
struct LoopDetector {
    /// Sliding window of (tool_name, args_hash) pairs.
    recent_calls: VecDeque<(String, u64)>,
}

impl LoopDetector {
    fn new() -> Self {
        Self {
            recent_calls: VecDeque::with_capacity(LOOP_DETECT_WINDOW),
        }
    }

    /// Records a tool call and returns a warning message if a loop is detected.
    fn record_and_check(&mut self, tool_name: &str, args: &serde_json::Value) -> Option<String> {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        args.to_string().hash(&mut hasher);
        let args_hash = hasher.finish();

        let key = (tool_name.to_string(), args_hash);
        let repeat_count = self
            .recent_calls
            .iter()
            .filter(|c| c.0 == key.0 && c.1 == key.1)
            .count();

        self.recent_calls.push_back(key);
        if self.recent_calls.len() > LOOP_DETECT_WINDOW {
            self.recent_calls.pop_front();
        }

        if repeat_count >= LOOP_REPEAT_THRESHOLD {
            Some(format!(
                "<system-reminder>LOOP DETECTED: You have called '{}' with identical arguments {} times \
                in the last {} steps. This is unproductive repetition. You MUST change your approach NOW:\n\
                1. If searching the web: try completely different keywords or a different data source.\n\
                2. If fetching a URL: the content may be unavailable — mark the task as 'data_missing' and continue.\n\
                3. If all alternatives are exhausted: accept the limitation and move to the next task.\n\
                Do NOT call '{}' with the same parameters again.</system-reminder>",
                tool_name,
                repeat_count + 1,
                LOOP_DETECT_WINDOW,
                tool_name
            ))
        } else {
            None
        }
    }
}

use crate::workflow::react::llm::LlmProcessor;
use crate::workflow::react::orchestrator::SubAgentFactory;
use crate::workflow::react::skills::{SkillManifest, SkillScanner};

pub struct WorkflowExecutor {
    pub session_id: String,
    pub context: ContextManager,
    pub tool_manager: Arc<ToolManager>,
    pub global_tool_manager: Arc<ToolManager>,
    pub chat_state: Arc<ChatState>,
    pub gateway: Arc<dyn Gateway>,
    pub sub_agent_factory: Arc<dyn SubAgentFactory>,
    pub compressor: ContextCompressor,
    pub path_guard: Arc<RwLock<PathGuard>>,
    pub skill_scanner: SkillScanner,
    pub llm_processor: LlmProcessor,
    pub available_skills: HashMap<String, SkillManifest>,
    pub agent_config: Agent,
    pub state: WorkflowState,
    pub current_step: usize,
    /// Hard upper bound on ReAct iterations to prevent infinite loops.
    pub max_steps: usize,
    pub auto_approve: HashSet<String>,
    pub signal_rx: Option<tokio::sync::mpsc::Receiver<String>>,
    pub tsid_generator: Arc<crate::libs::tsid::TsidGenerator>,
    pub active_provider_id: i64,
    pub active_model_name: String,
    /// Detects repetitive tool calls within a sliding window.
    loop_detector: LoopDetector,
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
        signal_rx: Option<tokio::sync::mpsc::Receiver<String>>,
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
        let model_name = active_model["model"].as_str().unwrap_or("").to_string();
        let reasoning = active_model["reasoning"].as_bool().unwrap_or(false);

        let compressor =
            ContextCompressor::new(chat_state.clone(), provider_id, model_name.clone());
        let skill_scanner = SkillScanner::new(app_data_dir.clone());

        // Auto-add system and application paths to allowed_paths
        let mut final_allowed_paths = allowed_paths;

        // 1. Add skill search paths
        for skill_path in skill_scanner.get_search_paths() {
            final_allowed_paths.push(skill_path);
        }

        // 2. Add ~/.chatspeed directory
        if let Some(home) = dirs::home_dir() {
            final_allowed_paths.push(home.join(".chatspeed"));
        }

        // 3. Add system temp directory
        final_allowed_paths.push(std::env::temp_dir());

        // 4. Canonicalize and Deduplicate
        let mut unique_paths = HashSet::new();
        let processed_paths: Vec<PathBuf> = final_allowed_paths
            .into_iter()
            .filter_map(|p| {
                let abs_p = if p.is_absolute() {
                    p
                } else {
                    std::env::current_dir().unwrap_or_default().join(p)
                };
                // We use canonicalize if it exists, otherwise just normalize
                abs_p.canonicalize().ok().or(Some(abs_p))
            })
            .filter(|p| unique_paths.insert(p.clone()))
            .collect();

        let path_guard = Arc::new(RwLock::new(PathGuard::new(processed_paths)));
        let tool_manager = Arc::new(ToolManager::new());

        let auto_approve: HashSet<String> =
            serde_json::from_str(agent_config.auto_approve.as_deref().unwrap_or("[]"))
                .unwrap_or_default();

        let llm_processor = LlmProcessor::new(
            session_id.clone(),
            agent_config.clone(),
            HashMap::new(), // available_skills will be populated later
            path_guard.clone(),
            provider_id,
            model_name.clone(),
            reasoning,
        );

        // Derive max_steps from agent config: use max_contexts as a hint (divide by 2000
        // tokens/step heuristic), clamped between 20 and 200, defaulting to DEFAULT_MAX_STEPS.
        let max_steps = agent_config
            .max_contexts
            .map(|ctx| ((ctx as usize) / 2000).clamp(20, 200))
            .unwrap_or(DEFAULT_MAX_STEPS);

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
            llm_processor,
            available_skills: HashMap::new(),
            agent_config,
            state: WorkflowState::Pending,
            current_step: 0,
            max_steps,
            auto_approve,
            signal_rx,
            tsid_generator,
            active_provider_id: provider_id,
            active_model_name: model_name,
            loop_detector: LoopDetector::new(),
        }
    }

    pub async fn init(&mut self) -> Result<(), WorkflowEngineError> {
        self.context.load_history().await?;
        self.available_skills = self.skill_scanner.scan()?;
        self.llm_processor.available_skills = self.available_skills.clone();
        self.register_foundation_tools().await?;

        // Sync TODO list on initialization
        let _ = self.sync_todo_list().await;

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
        let mut signal_rx = self
            .signal_rx
            .take()
            .ok_or_else(|| WorkflowEngineError::General("Signal receiver already taken".into()))?;

        while self.state != WorkflowState::Completed
            && self.state != WorkflowState::Error
            && self.state != WorkflowState::Cancelled
        {
            // --- Check signal channel for "stop" commands ---
            while let Ok(sig_str) = signal_rx.try_recv() {
                let sig_json: serde_json::Value =
                    serde_json::from_str(&sig_str).unwrap_or_default();
                if sig_json["type"] == "stop" || sig_str.to_lowercase().contains("stop") {
                    log::info!(
                        "WorkflowExecutor {}: Received STOP signal in main loop",
                        self.session_id
                    );
                    self.update_state(WorkflowState::Cancelled).await?;
                    self.signal_rx = Some(signal_rx);
                    return Ok(());
                } else if sig_json["type"] == "user_input" {
                    // Push user input to messages
                    let user_input = sig_json["content"].as_str().unwrap_or("").to_string();
                    self.add_message_and_notify(
                        "user".to_string(),
                        user_input,
                        None,
                        None,
                        false,
                        None,
                        None,
                    )
                    .await?;
                }
            }

            if self.state == WorkflowState::Paused {
                let signal_str = signal_rx
                    .recv()
                    .await
                    .ok_or_else(|| WorkflowEngineError::Gateway("Signal channel closed".into()))?;

                let signal_json: serde_json::Value = serde_json::from_str(&signal_str)
                    .unwrap_or(serde_json::json!({ "type": "message", "content": signal_str }));

                if signal_json["type"] == "stop" {
                    log::info!(
                        "WorkflowExecutor {}: Received STOP signal while PAUSED",
                        self.session_id
                    );
                    self.update_state(WorkflowState::Cancelled).await?;
                    self.signal_rx = Some(signal_rx);
                    return Ok(());
                }

                if signal_json["type"] == "approval" {
                    let approved = signal_json["approved"].as_bool().unwrap_or(false);
                    let approve_all = signal_json["approve_all"].as_bool().unwrap_or(false);
                    let tool_name = signal_json["tool_name"].as_str().unwrap_or("unknown");
                    let tool_args = signal_json["tool_args"].clone();

                    if approved {
                        log::info!(
                            "WorkflowExecutor {}: User APPROVED tool '{}'{}",
                            self.session_id,
                            tool_name,
                            if approve_all { " (Approve All)" } else { "" }
                        );

                        if approve_all {
                            self.auto_approve.insert(tool_name.to_string());
                        }

                        // Execution: Try local then global
                        let result = if let Ok(res) = self
                            .tool_manager
                            .tool_call(tool_name, tool_args.clone())
                            .await
                        {
                            Ok(res)
                        } else {
                            self.global_tool_manager
                                .tool_call(tool_name, tool_args.clone())
                                .await
                        };
                        let tool_call_obj = serde_json::json!({
                            "name": tool_name,
                            "arguments": tool_args
                        });

                        let reinforced = if tool_name.starts_with("todo_") {
                            let todos = if let Ok(store) = self.context.main_store.read() {
                                store
                                    .get_todo_list_for_workflow(&self.session_id)
                                    .unwrap_or_default()
                            } else {
                                vec![]
                            };
                            ObservationReinforcer::reinforce_with_context(
                                &tool_call_obj,
                                &result,
                                Some(serde_json::json!(todos)),
                            )
                        } else {
                            ObservationReinforcer::reinforce(&tool_call_obj, &result)
                        };

                        self.add_message_and_notify(
                            "tool".to_string(),
                            reinforced.content,
                            None,
                            Some(StepType::Observe),
                            reinforced.is_error,
                            reinforced.error_type.clone(),
                            Some(serde_json::json!({
                                "title": reinforced.title,
                                "summary": reinforced.summary,
                                "is_error": reinforced.is_error,
                                "error_type": reinforced.error_type,
                                "display_type": reinforced.display_type
                            })),
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
                            None,
                            Some(StepType::Observe),
                            true,
                            Some("UserRejected".to_string()),
                            Some(serde_json::json!({
                                "summary": "User rejected",
                                "is_error": true,
                                "error_type": "UserRejected"
                            })),
                        )
                        .await?;
                    }
                    self.update_state(WorkflowState::Thinking).await?;
                    continue;
                } else {
                    let user_input = signal_json["content"].as_str().unwrap_or("").to_string();
                    self.add_message_and_notify(
                        "user".to_string(),
                        user_input,
                        None,
                        None,
                        false,
                        None,
                        None,
                    )
                    .await?;
                    self.update_state(WorkflowState::Thinking).await?;
                    continue;
                }
            }
            self.current_step += 1;

            // --- Max-step budget guard ---
            if self.current_step > self.max_steps {
                log::warn!(
                    "WorkflowExecutor {}: Reached max steps ({}). Forcing conclusion.",
                    self.session_id,
                    self.max_steps
                );
                self.context
                    .add_message(
                        "user".to_string(),
                        format!(
                            "<system-reminder>STEP BUDGET EXHAUSTED: You have used {} out of {} \
                            allowed steps. You MUST conclude the task immediately. Do NOT perform \
                            any more research. Call 'finish_task' with a summary of what you have \
                            found so far, clearly noting any incomplete sections.</system-reminder>",
                            self.current_step, self.max_steps
                        ),
                        None,
                        Some(StepType::Observe),
                        self.current_step as i32,
                        true,
                        Some("StepBudgetExhausted".to_string()),
                        None,
                    )
                    .await?;
            } else if self.current_step == (self.max_steps * 4 / 5) {
                // 80% warning — give the LLM a chance to wrap up gracefully
                self.context
                    .add_message(
                        "user".to_string(),
                        format!(
                            "<system-reminder>STEP BUDGET WARNING: You are at step {} of {}. \
                            Only {} steps remain. Start wrapping up: complete your most critical \
                            pending tasks and prepare a final answer. Avoid starting new research \
                            threads.</system-reminder>",
                            self.current_step,
                            self.max_steps,
                            self.max_steps - self.current_step
                        ),
                        None,
                        Some(StepType::Observe),
                        self.current_step as i32,
                        false,
                        None,
                        None,
                    )
                    .await?;
            }

            self.update_state(WorkflowState::Thinking).await?;
            log::debug!(
                "[Workflow Engine] Session {}: Step {} - Calling LLM",
                self.session_id,
                self.current_step
            );
            let (response, response_reasoning, usage) =
                self.call_llm_with_retry(&mut signal_rx).await?;

            // --- 3. Check for stop signal after LLM call ---
            if self.check_stop_signal(&mut signal_rx).await? {
                self.signal_rx = Some(signal_rx);
                return Ok(());
            }

            log::debug!(
                "[Workflow Engine] Session {}: LLM responded ({} chars)",
                self.session_id,
                response.len()
            );
            self.add_message_and_notify(
                "assistant".to_string(),
                response.clone(),
                Some(response_reasoning),
                Some(StepType::Think),
                false,
                None,
                usage.map(|u| serde_json::json!({ "usage": u })),
            )
            .await?;

            self.update_state(WorkflowState::Executing).await?;
            log::debug!(
                "[Workflow Engine] Session {}: Executing tools",
                self.session_id
            );
            let (tool_results, has_todo_call) =
                self.execute_tools(response, &mut signal_rx).await?;

            if tool_results.is_empty() {
                log::warn!(
                    "WorkflowExecutor {}: No tool call detected. Forcing retry.",
                    self.session_id
                );
                // ADD TO CONTEXT BUT DO NOT NOTIFY GATEWAY (UI)
                self.context
                    .add_message(
                        "user".to_string(),
                        "Error: No tool call detected in your last response. You MUST call a tool to proceed, even if it is just 'answer_user' or 'finish_task'."
                            .to_string(),
                        None,
                        Some(StepType::Observe),
                        self.current_step as i32,
                        true,
                        Some("NoToolCall".to_string()),
                        Some(serde_json::json!({
                            "summary": "No tool detected",
                            "is_error": true,
                            "error_type": "NoToolCall"
                        })),
                    )
                    .await?;
                continue;
            }

            let mut needs_compression = false;
            for (tool_call_id, reinforced, tool_call) in tool_results {
                log::debug!(
                    "[Workflow Engine] Session {}: Recording observation for {}",
                    self.session_id,
                    tool_call_id
                );
                let compressed_signal = self
                    .add_message_and_notify(
                        "tool".to_string(),
                        reinforced.content,
                        None,
                        Some(StepType::Observe),
                        reinforced.is_error,
                        reinforced.error_type.clone(),
                        Some(serde_json::json!({
                            "tool_call_id": tool_call_id,
                            "tool_call": tool_call,
                            "title": reinforced.title,
                            "summary": reinforced.summary,
                            "is_error": reinforced.is_error,
                            "error_type": reinforced.error_type,
                            "display_type": reinforced.display_type
                        })),
                    )
                    .await?;
                if compressed_signal {
                    needs_compression = true;
                }
            }

            // --- SYNC TODO LIST IF ANY TODO TOOL WAS CALLED ---
            if has_todo_call {
                let todo_list_opt = if let Ok(store) = self.context.main_store.read() {
                    store.get_todo_list_for_workflow(&self.session_id).ok()
                } else {
                    None
                };

                if let Some(todos) = todo_list_opt {
                    self.gateway
                        .send(
                            &self.session_id,
                            GatewayPayload::SyncTodo {
                                todo_list: serde_json::json!(todos),
                            },
                        )
                        .await?;
                }
            }

            while let Ok(interrupt_str) = signal_rx.try_recv() {
                let sig_json: serde_json::Value = serde_json::from_str(&interrupt_str).unwrap_or(
                    serde_json::json!({ "type": "user_input", "content": interrupt_str }),
                );

                if sig_json["type"] == "stop" {
                    log::info!("WorkflowExecutor {}: Ignoring STOP signal at end of loop (will be caught at start)", self.session_id);
                    continue;
                }

                let content = if sig_json["type"] == "user_input" {
                    sig_json["content"]
                        .as_str()
                        .unwrap_or(&interrupt_str)
                        .to_string()
                } else {
                    interrupt_str
                };

                self.add_message_and_notify(
                    "user".to_string(),
                    content,
                    None,
                    None,
                    false,
                    None,
                    None,
                )
                .await?;
            }
            if needs_compression {
                let summary = self.compressor.compress(&self.context.messages).await?;
                self.context
                    .add_summary(summary, self.current_step as i32)
                    .await?;
            }
        }

        self.signal_rx = Some(signal_rx);
        Ok(())
    }

    async fn call_llm_with_retry(
        &mut self,
        signal_rx: &mut tokio::sync::mpsc::Receiver<String>,
    ) -> Result<(String, String, Option<serde_json::Value>), WorkflowEngineError> {
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
            // We use select! to allow immediate interruption of the LLM call
            let res = {
                let llm_call = self.call_llm();
                tokio::pin!(llm_call);

                tokio::select! {
                    result = &mut llm_call => result,
                    signal = signal_rx.recv() => {
                        if let Some(sig_str) = signal {
                            let sig_json: serde_json::Value = serde_json::from_str(&sig_str).unwrap_or_default();
                            if sig_json["type"] == "stop" || sig_str.to_lowercase().contains("stop") {
                                Err(WorkflowEngineError::General("STOP_SIGNAL_RECEIVED".into()))
                            } else {
                                // For other signals like user_input, we currently just wait for LLM
                                llm_call.await
                            }
                        } else {
                            llm_call.await
                        }
                    }
                }
            };

            // Handle the result outside the select block to avoid multiple borrows of self
            let res = match res {
                Ok(r) => Ok(r),
                Err(e) => {
                    if let WorkflowEngineError::General(ref msg) = e {
                        if msg == "STOP_SIGNAL_RECEIVED" {
                            log::info!(
                                "WorkflowExecutor {}: Received STOP signal during LLM call",
                                self.session_id
                            );
                            self.update_state(WorkflowState::Cancelled).await?;
                            return Err(WorkflowEngineError::General(
                                "Stopped by user signal".into(),
                            ));
                        }
                    }
                    Err(e)
                }
            };

            match res {
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
    ) -> Result<(String, String, Option<serde_json::Value>), WorkflowEngineError> {
        let mut tools = self.tool_manager.get_tool_calling_spec(None, None).await?;
        let global_tools = self
            .global_tool_manager
            .get_tool_calling_spec(None, None)
            .await?;
        tools.extend(global_tools);

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

        self.llm_processor
            .call(
                &mut self.context,
                self.current_step,
                chat_interface,
                self.gateway.clone(),
                tools,
                self.max_steps,
            )
            .await
    }

    async fn execute_tools(
        &mut self,
        raw_response: String,
        signal_rx: &mut tokio::sync::mpsc::Receiver<String>,
    ) -> Result<
        (
            Vec<(
                String,
                crate::workflow::react::observation::ReinforcedResult,
                serde_json::Value,
            )>,
            bool, // has_todo_call
        ),
        WorkflowEngineError,
    > {
        if self.check_stop_signal(signal_rx).await? {
            return Ok((Vec::new(), false));
        }
        let cleaned_response = crate::libs::util::format_json_str(&raw_response);
        let json_val: serde_json::Value = match serde_json::from_str(&cleaned_response) {
            Ok(v) => v,
            Err(e) => {
                log::warn!(
                    "WorkflowExecutor {}: Failed to parse JSON from AI response: {}. Raw: {}",
                    self.session_id,
                    e,
                    raw_response
                );
                // Return empty results to trigger the "No tool call detected" logic in run_loop
                return Ok((Vec::new(), false));
            }
        };
        let mut results = vec![];
        let mut has_todo_call = false;

        if let Some(tool_obj) = json_val.get("tool") {
            let name = tool_obj.get("name").and_then(|v| v.as_str()).unwrap_or("");

            if name.starts_with("todo_") {
                has_todo_call = true;
            }

            results.push((
                crate::ccproxy::get_tool_id(),
                self.dispatch_tool(tool_obj.clone()).await?,
                tool_obj.clone(),
            ));
        } else if let Some(tool_calls) = json_val.get("tool_calls").and_then(|v| v.as_array()) {
            for call in tool_calls {
                let id = call.get("id").and_then(|v| v.as_str()).unwrap_or("unknown");
                let func = call.get("function").unwrap_or(call);
                let name = func.get("name").and_then(|v| v.as_str()).unwrap_or("");

                if name.starts_with("todo_") {
                    has_todo_call = true;
                }

                results.push((
                    id.to_string(),
                    self.dispatch_tool(call.clone()).await?,
                    call.clone(),
                ));
            }
        } else if let Some(calls) = json_val.as_array() {
            // Handle direct array of tool calls
            for call in calls {
                let id = call.get("id").and_then(|v| v.as_str()).unwrap_or("unknown");
                let func = call.get("function").unwrap_or(call);
                let name = func.get("name").and_then(|v| v.as_str()).unwrap_or("");

                if name.starts_with("todo_") {
                    has_todo_call = true;
                }

                results.push((
                    id.to_string(),
                    self.dispatch_tool(call.clone()).await?,
                    call.clone(),
                ));
            }
        }
 else if json_val.get("name").is_some() || (json_val.get("function").is_some() && json_val.get("function").and_then(|f| f.get("name")).is_some()) {
            // Handle single tool call object without wrapper
            let id = json_val.get("id").and_then(|v| v.as_str()).unwrap_or("unknown");
            let name = json_val.get("name").and_then(|v| v.as_str())
                .or_else(|| json_val.get("function").and_then(|f| f.get("name")).and_then(|v| v.as_str()))
                .unwrap_or("");

            if name.starts_with("todo_") {
                has_todo_call = true;
            }

            results.push((
                id.to_string(),
                self.dispatch_tool(json_val.clone()).await?,
                json_val.clone(),
            ));
        }

        if has_todo_call {
            let _ = self.sync_todo_list().await;
        }

        Ok((results, has_todo_call))
    }

    async fn sync_todo_list(&self) -> Result<(), WorkflowEngineError> {
        let todo_list = {
            let store = self
                .context
                .main_store
                .read()
                .map_err(|e| WorkflowEngineError::General(e.to_string()))?;
            let snapshot = store
                .get_workflow_snapshot(&self.session_id)
                .map_err(|e| WorkflowEngineError::General(e.to_string()))?;
            let todo_list_str = snapshot
                .workflow
                .todo_list
                .unwrap_or_else(|| "[]".to_string());
            serde_json::from_str::<serde_json::Value>(&todo_list_str)
                .unwrap_or_else(|_| serde_json::json!([]))
        };

        self.gateway
            .send(&self.session_id, GatewayPayload::SyncTodo { todo_list })
            .await?;
        Ok(())
    }

    async fn dispatch_tool(
        &mut self,
        tool_call: serde_json::Value,
    ) -> Result<crate::workflow::react::observation::ReinforcedResult, WorkflowEngineError> {
        // 1. Extract name and args for internal execution logic
        let func = tool_call.get("function").unwrap_or(&tool_call);
        let name = func.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let args_raw = func
            .get("arguments")
            .cloned()
            .or_else(|| func.get("input").cloned())
            .unwrap_or(serde_json::json!({}));
        let args = if let serde_json::Value::String(ref s) = args_raw {
            serde_json::from_str(s).unwrap_or(args_raw)
        } else {
            args_raw
        };

        // --- Loop detection: track this call and warn if repetitive ---
        let loop_warning = self.loop_detector.record_and_check(name, &args);
        if let Some(ref warning) = loop_warning {
            log::warn!(
                "WorkflowExecutor {}: Loop detected for tool '{}'",
                self.session_id,
                name
            );
            // Inject as a user-side reminder so it shows up in LLM context
            let _ = self
                .context
                .add_message(
                    "user".to_string(),
                    warning.clone(),
                    None,
                    Some(StepType::Observe),
                    self.current_step as i32,
                    true,
                    Some("LoopDetected".to_string()),
                    None,
                )
                .await;
        }

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
                        return Ok(crate::workflow::react::observation::ReinforcedResult {
                            content: format!(
                                "Error: Command blocked by security policy. Reason: {}",
                                reason
                            ),
                            title: format!("Bash({})", command_str),
                            summary: "Blocked".to_string(),
                            is_error: true,
                            error_type: Some("Security".to_string()),
                            display_type: "text".to_string(),
                        });
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
                        return Ok(crate::workflow::react::observation::ReinforcedResult {
                            content: "WAITING_FOR_USER_APPROVAL".to_string(),
                            title: format!("Bash({})", command_str),
                            summary: "Waiting for approval".to_string(),
                            is_error: false,
                            error_type: None,
                            display_type: "text".to_string(),
                        });
                    }
                }
            }
        }

        // --- 2. Generic Tool Safety (FS, etc.) ---
        if name == TOOL_WRITE_FILE || name == TOOL_EDIT_FILE {
            if !self.auto_approve.contains(name) {
                let file_path = args["file_path"]
                    .as_str()
                    .or_else(|| args["path"].as_str())
                    .unwrap_or("unknown");
                let mut details = format!("Operation: {}\nFile: {}\n", name, file_path);

                if name == TOOL_EDIT_FILE {
                    let old = args["old_string"].as_str().unwrap_or("");
                    let new = args["new_string"].as_str().unwrap_or("");
                    details.push_str(&format!(
                        "\n--- OLD STRING ---\n{}\n\n--- NEW STRING ---\n{}",
                        old, new
                    ));
                } else if name == TOOL_WRITE_FILE {
                    let content = args["content"].as_str().unwrap_or("");
                    if content.len() > 1000 {
                        details.push_str(&format!("\nContent: ({} bytes)", content.len()));
                    } else {
                        details.push_str(&format!("\nContent:\n{}", content));
                    }
                }

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
                return Ok(crate::workflow::react::observation::ReinforcedResult {
                    content: "WAITING_FOR_USER_APPROVAL".to_string(),
                    title: format!("{}({})", name, file_path),
                    summary: "Waiting for approval".to_string(),
                    is_error: false,
                    error_type: None,
                    display_type: if name == TOOL_EDIT_FILE {
                        "diff".to_string()
                    } else {
                        "text".to_string()
                    },
                });
            }
        }

        // --- 3. Path Guard for FS tools ---
        if name == TOOL_READ_FILE
            || name == TOOL_WRITE_FILE
            || name == TOOL_LIST_DIR
            || name == TOOL_EDIT_FILE
        {
            if let Some(path_str) = args["file_path"].as_str().or_else(|| args["path"].as_str()) {
                if let Ok(guard) = self.path_guard.read() {
                    guard.validate(std::path::Path::new(path_str))?;
                }
            }
        }

        // --- 3. Special Core Tools ---
        match name {
            n if n == TOOL_ANSWER_USER => {
                return Ok(crate::workflow::react::observation::ReinforcedResult {
                    content: "Summary delivered".into(),
                    title: "AnswerUser".to_string(),
                    summary: "Answered user".to_string(),
                    is_error: false,
                    error_type: None,
                    display_type: "text".to_string(),
                })
            }
            n if n == TOOL_ASK_USER => {
                self.update_state(WorkflowState::Paused).await?;
                return Ok(crate::workflow::react::observation::ReinforcedResult {
                    content: "Waiting for user".into(),
                    title: "AskUser".to_string(),
                    summary: "Asked user".to_string(),
                    is_error: false,
                    error_type: None,
                    display_type: "text".to_string(),
                });
            }
            n if n == TOOL_FINISH_TASK => {
                self.update_state(WorkflowState::Completed).await?;
                return Ok(crate::workflow::react::observation::ReinforcedResult {
                    content: "Finished".into(),
                    title: "FinishTask".to_string(),
                    summary: "Task completed".to_string(),
                    is_error: false,
                    error_type: None,
                    display_type: "text".to_string(),
                });
            }
            _ => {}
        }

        // --- 4. Tool Call Dispatch (Local then Global) ---
        // First try the session-specific tool manager
        let result = if let Ok(res) = self.tool_manager.tool_call(name, args.clone()).await {
            Ok(res)
        } else {
            // Then try the global tool manager (for MCP, WebSearch, etc.)
            self.global_tool_manager.tool_call(name, args.clone()).await
        };

        // --- 5. Post-process: Auto-summarize large web_fetch content ---
        if name == TOOL_WEB_FETCH {
            if let Ok(val) = &result {
                if let Some(content) = val.get("content").and_then(|v| v.as_str()) {
                    if content.len() > 15000 {
                        log::info!("WorkflowExecutor {}: Content from web_fetch is too long ({} chars). Summarizing...", self.session_id, content.len());
                        match self.summarize_content(content).await {
                            Ok(summary) => {
                                return Ok(crate::workflow::react::observation::ReinforcedResult {
                                    content: format!("[Auto-Summarized Result] {}\n\n<system-reminder>The original content was over 50000 characters and has been summarized to focus on relevant information.</system-reminder>", summary),
                                    title: format!("Fetch({})", args["url"].as_str().unwrap_or("")),
                                    summary: "Content summarized".to_string(),
                                    is_error: false,
                                    error_type: None,
                                    display_type: "text".to_string(),

                                });
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

        if name.starts_with("todo_") {
            let todos = if let Ok(store) = self.context.main_store.read() {
                store
                    .get_todo_list_for_workflow(&self.session_id)
                    .unwrap_or_default()
            } else {
                vec![]
            };
            Ok(
                crate::workflow::react::observation::ObservationReinforcer::reinforce_with_context(
                    &tool_call,
                    &result,
                    Some(serde_json::json!(todos)),
                ),
            )
        } else {
            Ok(
                crate::workflow::react::observation::ObservationReinforcer::reinforce(
                    &tool_call, &result,
                ),
            )
        }
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

    pub(crate) async fn add_message_and_notify(
        &mut self,
        role: String,
        mut content: String,
        reasoning: Option<String>,
        step_type: Option<StepType>,
        is_error: bool,
        error_type: Option<String>,
        mut metadata: Option<serde_json::Value>,
    ) -> Result<bool, WorkflowEngineError> {
        // --- 1. Content Normalization (Handle mixed Text + JSON responses) ---
        if role == "assistant" {
            let trimmed = content.trim();

            // Try to find a JSON block if it doesn't start with {
            let (text_part, json_val) = if trimmed.starts_with('{') {
                (
                    None,
                    serde_json::from_str::<serde_json::Value>(trimmed).ok(),
                )
            } else {
                // Look for the last { ... } block
                if let (Some(start), Some(end)) = (trimmed.find('{'), trimmed.rfind('}')) {
                    if start < end {
                        let text = trimmed[..start].trim().to_string();
                        let json = trimmed[start..=end].trim();
                        (
                            Some(text),
                            serde_json::from_str::<serde_json::Value>(json).ok(),
                        )
                    } else {
                        (None, None)
                    }
                } else {
                    (None, None)
                }
            };

            if let Some(json_msg) = json_val {
                // Case A: Extraction from JSON structure
                if let Some(text) = json_msg.get("content").and_then(|v| v.as_str()) {
                    content = text.to_string();
                } else if let Some(text) = text_part {
                    content = text;
                } else {
                    // ReAct fallback for answer_user/finish_task
                    if let Some(tool_obj) = json_msg.get("tool") {
                        let name = tool_obj.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        let args = tool_obj
                            .get("arguments")
                            .unwrap_or(&serde_json::Value::Null);
                        if name == "answer_user" {
                            content = args
                                .get("text")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                                .unwrap_or(content);
                        } else if name == "finish_task" {
                            content = args
                                .get("summary")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                                .unwrap_or(content);
                        }
                    }
                }

                // Merge tool calls into metadata
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
                reasoning.clone(),
                step_type.clone(),
                self.current_step as i32,
                is_error,
                error_type.clone(),
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
                    reasoning,
                    step_type,
                    step_index: self.current_step as i32,
                    is_error,
                    error_type,
                    metadata,
                },
            )
            .await?;

        Ok(needs_compression)
    }

    /// Checks if a stop signal is pending in the channel
    async fn check_stop_signal(
        &mut self,
        signal_rx: &mut tokio::sync::mpsc::Receiver<String>,
    ) -> Result<bool, WorkflowEngineError> {
        while let Ok(sig_str) = signal_rx.try_recv() {
            let sig_json: serde_json::Value = serde_json::from_str(&sig_str).unwrap_or_default();
            if sig_json["type"] == "stop" || sig_str.to_lowercase().contains("stop") {
                log::info!("WorkflowExecutor {}: Received STOP signal", self.session_id);
                self.update_state(WorkflowState::Cancelled).await?;
                return Ok(true);
            } else if sig_json["type"] == "user_input" {
                let user_input = sig_json["content"].as_str().unwrap_or("").to_string();
                self.add_message_and_notify(
                    "user".to_string(),
                    user_input,
                    None,
                    None,
                    false,
                    None,
                    None,
                )
                .await?;
            } else if sig_json["type"] == "update_allowed_paths" {
                if let Some(paths_val) = sig_json.get("paths") {
                    if let Some(paths_arr) = paths_val.as_array() {
                        let mut unique_paths = HashSet::new();
                        let paths: Vec<PathBuf> = paths_arr
                            .iter()
                            .filter_map(|v| v.as_str())
                            .map(PathBuf::from)
                            .filter_map(|p| {
                                let abs_p = if p.is_absolute() {
                                    p
                                } else {
                                    std::env::current_dir().unwrap_or_default().join(p)
                                };
                                abs_p.canonicalize().ok().or(Some(abs_p))
                            })
                            .filter(|p| unique_paths.insert(p.clone()))
                            .collect();
                        log::info!(
                            "WorkflowExecutor {}: Updating allowed paths to {:?}",
                            self.session_id,
                            paths
                        );
                        if let Ok(mut guard) = self.path_guard.write() {
                            guard.update_allowed_roots(paths);
                        }
                    }
                }
            }
        }
        Ok(false)
    }
}
