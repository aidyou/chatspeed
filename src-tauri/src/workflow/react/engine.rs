use async_trait::async_trait;
use dirs;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tokio::time::{sleep, Duration};

use crate::ai::chat::openai::OpenAIChat;
use crate::ai::interaction::chat_completion::{AiChatEnum, ChatState};
use crate::db::{Agent, MainStore};
use crate::tools::{
    ToolManager, TOOL_ASK_USER, TOOL_BASH, TOOL_EDIT_FILE, TOOL_FINISH_TASK, TOOL_LIST_DIR,
    TOOL_READ_FILE, TOOL_SUBMIT_PLAN, TOOL_WEB_FETCH, TOOL_WRITE_FILE,
};
use crate::workflow::react::compression::ContextCompressor;
use crate::workflow::react::context::ContextManager;
use crate::workflow::react::error::WorkflowEngineError;
use crate::workflow::react::gateway::Gateway;
use crate::workflow::react::intelligence::IntelligenceManager;
use crate::workflow::react::llm::LlmProcessor;
use crate::workflow::react::loop_detector::LoopDetector;
use crate::workflow::react::observation::{ObservationReinforcer, ReinforcedResult};
use crate::workflow::react::orchestrator::SubAgentFactory;
use crate::workflow::react::policy::{ExecutionPhase, ExecutionPolicy, PathRestriction};
use crate::workflow::react::security::PathGuard;
use crate::workflow::react::skills::{SkillManifest, SkillScanner};
use crate::workflow::react::types::{GatewayPayload, StepType, WorkflowState};

/// Default maximum ReAct steps before the agent is forced to conclude.
const DEFAULT_MAX_STEPS: usize = 60;

/// Unified interface for ReAct executors (Planners and Runners).
#[async_trait]
pub trait ReActExecutor: Send + Sync {
    async fn init(&mut self) -> Result<(), WorkflowEngineError>;
    async fn run_loop(&mut self) -> Result<(), WorkflowEngineError>;
    async fn add_message_and_notify(
        &mut self,
        role: String,
        content: String,
        reasoning: Option<String>,
        step_type: Option<StepType>,
        is_error: bool,
        error_type: Option<String>,
        metadata: Option<serde_json::Value>,
    ) -> Result<bool, WorkflowEngineError>;

    // Property accessors
    fn session_id(&self) -> String;
    fn state(&self) -> WorkflowState;
    fn set_state(&mut self, state: WorkflowState);
    fn messages(&self) -> Vec<crate::db::WorkflowMessage>;
}

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
    pub intelligence_manager: IntelligenceManager,
    pub available_skills: HashMap<String, SkillManifest>,
    pub agent_config: Agent,
    pub state: WorkflowState,
    pub current_step: usize,
    /// Hard upper bound on ReAct iterations to prevent infinite loops.
    pub max_steps: usize,
    pub consecutive_no_tool_calls: u32,
    pub auto_approve: HashSet<String>,
    pub signal_rx: Option<tokio::sync::mpsc::Receiver<String>>,
    pub tsid_generator: Arc<crate::libs::tsid::TsidGenerator>,
    /// Rules and permissions for this ReAct session.
    pub policy: ExecutionPolicy,
    /// Detects repetitive tool calls within a sliding window.
    loop_detector: LoopDetector,
}

#[async_trait]
impl ReActExecutor for WorkflowExecutor {
    async fn init(&mut self) -> Result<(), WorkflowEngineError> {
        self.init_internal().await
    }

    async fn run_loop(&mut self) -> Result<(), WorkflowEngineError> {
        self.run_loop_internal().await
    }

    async fn add_message_and_notify(
        &mut self,
        role: String,
        content: String,
        reasoning: Option<String>,
        step_type: Option<StepType>,
        is_error: bool,
        error_type: Option<String>,
        metadata: Option<serde_json::Value>,
    ) -> Result<bool, WorkflowEngineError> {
        self.add_message_and_notify_internal(
            role, content, reasoning, step_type, is_error, error_type, metadata,
        )
        .await
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

    fn messages(&self) -> Vec<crate::db::WorkflowMessage> {
        self.context.messages.clone()
    }
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
        policy: ExecutionPolicy,
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
            chat_state.clone(),
            provider_id,
            model_name.clone(),
            reasoning,
        );

        let intelligence_manager = IntelligenceManager::new(
            session_id.clone(),
            chat_state.clone(),
            provider_id,
            model_name.clone(),
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
            intelligence_manager,
            available_skills: HashMap::new(),
            agent_config,
            state: WorkflowState::Pending,
            current_step: 0,
            max_steps,
            consecutive_no_tool_calls: 0,
            auto_approve,
            signal_rx,
            tsid_generator,
            policy,
            loop_detector: LoopDetector::new(),
        }
    }

    pub(crate) async fn init_internal(&mut self) -> Result<(), WorkflowEngineError> {
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
        if is_allowed(TOOL_READ_FILE)
            && self
                .policy
                .allowed_categories
                .contains(&ToolCategory::FileSystem)
        {
            tm.register_tool(Arc::new(ReadFile)).await?;
        }
        if is_allowed(TOOL_WRITE_FILE)
            && self
                .policy
                .allowed_categories
                .contains(&ToolCategory::FileSystem)
        {
            tm.register_tool(Arc::new(WriteFile)).await?;
        }
        if is_allowed(TOOL_EDIT_FILE)
            && self
                .policy
                .allowed_categories
                .contains(&ToolCategory::FileSystem)
        {
            tm.register_tool(Arc::new(EditFile)).await?;
        }
        if is_allowed(TOOL_LIST_DIR)
            && self
                .policy
                .allowed_categories
                .contains(&ToolCategory::FileSystem)
        {
            tm.register_tool(Arc::new(ListDir)).await?;
        }
        if is_allowed(TOOL_GLOB)
            && self
                .policy
                .allowed_categories
                .contains(&ToolCategory::FileSystem)
        {
            tm.register_tool(Arc::new(Glob)).await?;
        }
        if is_allowed(TOOL_GREP)
            && self
                .policy
                .allowed_categories
                .contains(&ToolCategory::FileSystem)
        {
            tm.register_tool(Arc::new(Grep)).await?;
        }

        // 2. Shell Tool (With session-aware policy)
        if is_allowed(TOOL_BASH)
            && self
                .policy
                .allowed_categories
                .contains(&ToolCategory::System)
        {
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
                self.policy.path_restriction == PathRestriction::SandboxOnly,
            )))
            .await?;
        }

        // 3. Interaction Tools
        if is_allowed(TOOL_ASK_USER)
            && self
                .policy
                .allowed_categories
                .contains(&ToolCategory::Interaction)
        {
            tm.register_tool(Arc::new(AskUser)).await?;
        }
        if self.policy.phase == ExecutionPhase::Planning {
            tm.register_tool(Arc::new(SubmitPlan)).await?;
        } else if is_allowed(TOOL_FINISH_TASK)
            && self
                .policy
                .allowed_categories
                .contains(&ToolCategory::Interaction)
        {
            tm.register_tool(Arc::new(FinishTask)).await?;
        }

        // 4. Multi-Agent & Skill Tools
        if self
            .policy
            .allowed_categories
            .contains(&ToolCategory::Skill)
        {
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
        }

        // 5. Todo Manager Tools (Session Persistent)
        if self
            .policy
            .allowed_categories
            .contains(&ToolCategory::Skill)
        {
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
        }

        Ok(())
    }

    pub(crate) async fn run_loop_internal(&mut self) -> Result<(), WorkflowEngineError> {
        let mut signal_rx = self
            .signal_rx
            .take()
            .ok_or_else(|| WorkflowEngineError::General("Signal receiver already taken".into()))?;

        while self.state != WorkflowState::Completed
            && self.state != WorkflowState::Error
            && self.state != WorkflowState::Cancelled
            && self.state != WorkflowState::AwaitingApproval
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
                    self.add_message_and_notify_internal(
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

                        self.add_message_and_notify_internal(
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
                        self.add_message_and_notify_internal(
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
                    self.add_message_and_notify_internal(
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
            let (text_part, json_part, response_reasoning, usage) =
                self.call_llm_with_retry(&mut signal_rx).await?;

            // --- 3. Check for stop signal after LLM call ---
            if self.check_stop_signal(&mut signal_rx).await? {
                self.signal_rx = Some(signal_rx);
                return Ok(());
            }

            log::debug!(
                "[Workflow Engine] Session {}: LLM responded (text: {} chars, json: {} chars)",
                self.session_id,
                text_part.len(),
                json_part.len()
            );

            // Construct full message for history (Text + JSON if present)
            let full_response = if json_part.is_empty() {
                text_part.clone()
            } else {
                format!("{}\n\n{}", text_part, json_part)
            };

            self.add_message_and_notify_internal(
                "assistant".to_string(),
                full_response,
                Some(response_reasoning),
                Some(StepType::Think),
                false,
                None,
                usage, // Use the metadata directly
            )
            .await?;

            self.update_state(WorkflowState::Executing).await?;
            log::debug!(
                "[Workflow Engine] Session {}: Executing tools",
                self.session_id
            );
            let (tool_results, has_todo_call) = self
                .execute_tools(text_part, json_part, &mut signal_rx)
                .await?;

            if tool_results.is_empty() {
                self.consecutive_no_tool_calls += 1;
                log::warn!(
                    "WorkflowExecutor {}: No tool call detected (consecutive: {}). Forcing retry.",
                    self.session_id,
                    self.consecutive_no_tool_calls
                );

                let error_msg = if self.consecutive_no_tool_calls >= 3 {
                    let remaining = self.max_steps.saturating_sub(self.current_step);
                    format!(
                        "<system-reminder>CRITICAL ERROR: You have failed to call a tool for {} consecutive turns. \
                        This is wasting your limited step budget ({}/{} steps used, {} remaining). \
                        To maintain execution state and avoid eventual task failure, your next response MUST conclude with a valid tool call. \
                        If you are truly finished, provide a summary and call 'finish_task'.</system-reminder>",
                        self.consecutive_no_tool_calls, self.current_step, self.max_steps, remaining
                    )
                } else {
                    "<system-reminder>Error: No tool call detected in your last response. You MUST call a tool to proceed. If you have finished your task, call 'finish_task' AFTER providing a summary in plain text.</system-reminder>".to_string()
                };

                // Avoid appending multiple identical reminders if AI is stuck
                let should_add = if let Some(last_msg) = self.context.get_messages_for_llm().last()
                {
                    !(last_msg.role == "user" && last_msg.message.contains("No tool call detected"))
                } else {
                    true
                };

                if should_add {
                    self.context
                        .add_message(
                            "user".to_string(),
                            error_msg,
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
                }

                // If AI is extremely stubborn, back off and wait or slightly reduce step count to allow recovery
                continue;
            }

            // Reset counter on any successful tool call
            self.consecutive_no_tool_calls = 0;

            let mut needs_compression = false;
            let mut final_completion_detected = false;

            for (tool_call_id, reinforced, tool_call) in tool_results {
                log::debug!(
                    "[Workflow Engine] Session {}: Recording observation for {}",
                    self.session_id,
                    tool_call_id
                );

                // Track if finish_task was among the results AND was successful
                let func = tool_call.get("function").unwrap_or(&tool_call);
                let name = func.get("name").and_then(|v| v.as_str()).unwrap_or("");
                if name == TOOL_FINISH_TASK && !reinforced.is_error {
                    final_completion_detected = true;
                }

                // Use 'user' role for system-level summary errors to force compliance
                let role = if reinforced.is_error
                    && reinforced.error_type.as_deref() == Some("NoSummary")
                {
                    "user".to_string()
                } else {
                    "tool".to_string()
                };

                // protocol compliance: if the role is 'user', we MUST still provide a 'tool' response first
                // to satisfy the OpenAI requirement that assistant tool_calls are followed by tool results.
                if role == "user" {
                    let _ = self
                        .add_message_and_notify(
                            "tool".to_string(),
                            format!("Error: Execution of '{}' failed or was rejected.", name),
                            None,
                            Some(StepType::Observe),
                            true,
                            reinforced.error_type.clone(),
                            Some(serde_json::json!({
                                "tool_call_id": tool_call_id,
                                "tool_call": tool_call,
                                "title": "System Check",
                                "summary": "Call rejected"
                            })),
                        )
                        .await?;
                }

                let compressed_signal = self
                    .add_message_and_notify(
                        role.clone(),
                        reinforced.content,
                        None,
                        Some(StepType::Observe),
                        reinforced.is_error,
                        reinforced.error_type.clone(),
                        Some(serde_json::json!({
                            "tool_call_id": if role == "tool" { Some(&tool_call_id) } else { None },
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

                // If finish_task detected, don't process further tool results in this batch
                if final_completion_detected {
                    break;
                }
            }

            // --- SYNC TODO LIST IF ANY TODO TOOL WAS CALLED ---
            if has_todo_call {
                let _ = self.sync_todo_list().await;
            }

            // --- CHECK FOR USER INTERRUPT OR INPUT ---
            while let Ok(interrupt_str) = signal_rx.try_recv() {
                let sig_json: serde_json::Value = serde_json::from_str(&interrupt_str).unwrap_or(
                    serde_json::json!({ "type": "user_input", "content": interrupt_str }),
                );

                if sig_json["type"] == "stop" {
                    log::info!(
                        "WorkflowExecutor {}: STOP signal will be caught at next iteration start",
                        self.session_id
                    );
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

                self.add_message_and_notify_internal(
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

            // --- TRIGGER CONTEXT COMPRESSION IF NEEDED ---
            if needs_compression {
                let summary = self.compressor.compress(&self.context.messages).await?;
                self.context
                    .add_summary(summary, self.current_step as i32)
                    .await?;
            }

            // --- IMMEDIATE EXIT IF FINISHED ---
            if final_completion_detected {
                log::info!(
                    "WorkflowExecutor {}: Concluding session due to finish_task.",
                    self.session_id
                );
                self.update_state(WorkflowState::Completed).await?;
                // Break the main run_loop immediately
                break;
            }
        }

        self.signal_rx = Some(signal_rx);
        Ok(())
    }

    async fn call_llm_with_retry(
        &mut self,
        signal_rx: &mut tokio::sync::mpsc::Receiver<String>,
    ) -> Result<(String, String, String, Option<serde_json::Value>), WorkflowEngineError> {
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
                Ok((text, json, reasoning, usage)) => return Ok((text, json, reasoning, usage)),
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
                        let wait_duration = Duration::from_secs(2u64.pow(retry_count as u32));
                        log::info!(
                            "WorkflowExecutor {}: Waiting {:?} before retry...",
                            self.session_id,
                            wait_duration
                        );

                        tokio::select! {
                            _ = sleep(wait_duration) => {}
                            msg = signal_rx.recv() => {
                                if let Some(sig_str) = msg {
                                    let sig_json: serde_json::Value = serde_json::from_str(&sig_str).unwrap_or_default();
                                    if sig_json["type"] == "stop" || sig_str.to_lowercase().contains("stop") {
                                        log::info!("WorkflowExecutor {}: Received STOP signal during retry wait", self.session_id);
                                        self.update_state(WorkflowState::Cancelled).await?;
                                        return Err(WorkflowEngineError::General("Execution stopped by user".into()));
                                    }
                                }
                            }
                        }
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
    ) -> Result<(String, String, String, Option<serde_json::Value>), WorkflowEngineError> {
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
                &self.policy,
            )
            .await
    }

    async fn execute_tools(
        &mut self,
        text_part: String,
        json_part: String,
        signal_rx: &mut tokio::sync::mpsc::Receiver<String>,
    ) -> Result<
        (
            Vec<(String, ReinforcedResult, serde_json::Value)>,
            bool, // has_todo_call
        ),
        WorkflowEngineError,
    > {
        // If json_part is empty, there are no tool calls.
        if json_part.is_empty() {
            return Ok((Vec::new(), false));
        }

        let cleaned_json = crate::libs::util::format_json_str(&json_part);
        let json_val: serde_json::Value = match serde_json::from_str(&cleaned_json) {
            Ok(v) => v,
            Err(e) => {
                log::warn!(
                    "WorkflowExecutor {}: Failed to parse JSON from AI response: {}. JSON: {}",
                    self.session_id,
                    e,
                    json_part
                );
                return Ok((Vec::new(), false));
            }
        };
        let mut tool_calls = vec![];
        if let Some(tool_obj) = json_val.get("tool") {
            tool_calls.push(tool_obj.clone());
        } else if let Some(calls) = json_val.get("tool_calls").and_then(|v| v.as_array()) {
            tool_calls.extend(calls.iter().cloned());
        } else if let Some(calls) = json_val.as_array() {
            tool_calls.extend(calls.iter().cloned());
        } else if json_val.get("name").is_some()
            || (json_val.get("function").is_some()
                && json_val
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .is_some())
        {
            tool_calls.push(json_val.clone());
        }

        if tool_calls.is_empty() {
            return Ok((Vec::new(), false));
        }

        let mut has_todo_call = false;
        let mut results = vec![];

        use futures::stream::{FuturesOrdered, StreamExt};

        let mut tool_futures = FuturesOrdered::new();

        for call in tool_calls {
            let id = call
                .get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(crate::ccproxy::get_tool_id);

            let func = call.get("function").unwrap_or(&call);
            let name = func
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
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

            // --- 1. Sequential Pre-check (Loop detection, Security, etc.) ---
            match self.pre_dispatch_check(&name, &args, &text_part).await {
                Ok(Some(early_result)) => {
                    results.push((id, early_result, call));
                    continue;
                }
                Err(e) => return Err(e),
                _ => {}
            }

            if name.starts_with("todo_") {
                has_todo_call = true;
            }

            // --- 2. Queue for Parallel Execution (Limit to 3 concurrent calls) ---
            let tm = self.tool_manager.clone();
            let gtm = self.global_tool_manager.clone();
            let semaphore = self.context.semaphore.clone();

            tool_futures.push_back(async move {
                let _permit = semaphore.acquire().await.ok();
                let call_name = name.clone();
                let call_args = args.clone();

                let res = if let Ok(res) = tm.tool_call(&call_name, call_args.clone()).await {
                    Ok(res)
                } else {
                    gtm.tool_call(&call_name, call_args).await
                };

                (id, name, args, call, res)
            });
        }

        // --- 3. Sequential Post-processing (Summarization, Todo Sync) ---
        while !tool_futures.is_empty() {
            tokio::select! {
                result_opt = tool_futures.next() => {
                    if let Some((id, name, args, call, res)) = result_opt {
                        let reinforced = self
                            .post_process_tool_result(&name, &args, &call, res)
                            .await?;
                        results.push((id, reinforced, call));
                    }
                }
                msg = signal_rx.recv() => {
                    if let Some(sig_str) = msg {
                        let sig_json: serde_json::Value = serde_json::from_str(&sig_str).unwrap_or_default();
                        if sig_json["type"] == "stop" || sig_str.to_lowercase().contains("stop") {
                            log::info!("WorkflowExecutor {}: Received STOP signal during tool execution", self.session_id);
                            self.update_state(WorkflowState::Cancelled).await?;
                            return Err(WorkflowEngineError::General("Execution stopped by user".into()));
                        }
                    }
                }
            }
        }

        Ok((results, has_todo_call))
    }

    /// Performs safety and logic checks BEFORE a tool is executed.
    /// Returns Some(ReinforcedResult) if the check fails or requires an early return (e.g. Paused for confirmation).
    async fn pre_dispatch_check(
        &mut self,
        name: &str,
        args: &serde_json::Value,
        text_part: &str,
    ) -> Result<Option<ReinforcedResult>, WorkflowEngineError> {
        // 1. Plan Submission Guard
        if name == TOOL_SUBMIT_PLAN {
            if self.policy.phase != ExecutionPhase::Planning {
                return Err(WorkflowEngineError::Security("Tool 'submit_plan' is only allowed in Planning phase.".into()));
            }

            if text_part.trim().is_empty() {
                return Ok(Some(ReinforcedResult {
                    content: "<system-reminder>Error: You called 'submit_plan' but your plain text response was empty. You MUST provide a summary of your findings and why this plan is recommended in plain text BEFORE the tool call block.</system-reminder>".into(),
                    title: "SubmitPlan Error".to_string(),
                    summary: "Missing summary".to_string(),
                    is_error: true,
                    error_type: Some("NoSummary".into()),
                    display_type: "text".to_string(),
                }));
            }

            // A. Update internal and DB status
            self.update_state(WorkflowState::AwaitingApproval).await?;
            {
                let store = self.context.main_store.write().map_err(|e| {
                    WorkflowEngineError::Db(crate::db::error::StoreError::LockError(e.to_string()))
                })?;
                store.update_workflow_status(&self.session_id, "awaiting_approval")?;
            }
        }

        // 2. Finish Task Guard & Self-Reflection Audit
        if name == TOOL_FINISH_TASK {
            // A. Physical Presence Check
            if text_part.trim().is_empty() {
                return Ok(Some(ReinforcedResult {
                    content: "<system-reminder>Error: You called 'finish_task' but your plain text response was empty. You MUST provide a comprehensive summary or report in plain text BEFORE the tool call block to inform the user of your results.</system-reminder>".into(),
                    title: "FinishTask Error".to_string(),
                    summary: "Missing summary".to_string(),
                    is_error: true,
                    error_type: Some("NoSummary".into()),
                    display_type: "text".to_string(),
                }));
            }

            // B. Hard-check: Todo Items Status
            if let Ok(store) = self.context.main_store.read() {
                if let Ok(todos) = store.get_todo_list_for_workflow(&self.session_id) {
                    let has_failures = todos.iter().any(|t| {
                        let s = t["status"].as_str().unwrap_or("");
                        s == "failed" || s == "data_missing"
                    });

                    // If there are failures, we allow exiting (consistent with 'Fail Fast' principle)
                    if !has_failures {
                        let active_tasks: Vec<String> = todos
                            .iter()
                            .filter(|t| {
                                let s = t["status"].as_str().unwrap_or("");
                                s == "pending" || s == "in_progress"
                            })
                            .map(|t| t["subject"].as_str().unwrap_or("Untitled").to_string())
                            .collect();

                        if !active_tasks.is_empty() {
                            return Ok(Some(ReinforcedResult {
                                content: format!("<system-reminder>Block: You still have active tasks: {}. You MUST either complete them, or mark them as 'failed' or 'data_missing' if they cannot be fulfilled, before calling finish_task.</system-reminder>", active_tasks.join(", ")),
                                title: "Tasks Pending".to_string(),
                                summary: "Incomplete todos".to_string(),
                                is_error: true,
                                error_type: Some("PendingTodos".into()),
                                display_type: "text".to_string(),
                            }));
                        }
                    }
                }
            }

            // C. Semantic Quality Audit
            if let Some(audit_feedback) = self
                .intelligence_manager
                .run_final_audit(&self.context)
                .await?
            {
                log::warn!(
                    "WorkflowExecutor {}: Final audit rejected the conclusion. Feedback: {}",
                    self.session_id,
                    audit_feedback
                );
                return Ok(Some(ReinforcedResult {
                    content: format!("<system-reminder>Audit Rejected: Your conclusion was deemed incomplete. Feedback: {}\n\nYou MUST address these points before you can call finish_task.</system-reminder>", audit_feedback),
                    title: "Audit Rejected".to_string(),
                    summary: "Audit failed".to_string(),
                    is_error: true,
                    error_type: Some("AuditRejected".into()),
                    display_type: "text".to_string(),
                }));
            }
        }

        // 2. Loop Detection
        let loop_warning = self.loop_detector.record_and_check(name, args);
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

        // 3. Security: Shell Auditing
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

                match policy_engine.check(
                    command_str,
                    self.policy.path_restriction == PathRestriction::SandboxOnly,
                ) {
                    crate::tools::ShellDecision::Allow => {}
                    crate::tools::ShellDecision::Deny(reason) => {
                        return Ok(Some(ReinforcedResult {
                            content: format!(
                                "Error: Command blocked by security policy. Reason: {}",
                                reason
                            ),
                            title: format!("Bash({})", command_str),
                            summary: "Blocked".to_string(),
                            is_error: true,
                            error_type: Some("Security".to_string()),
                            display_type: "text".to_string(),
                        }));
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
                        return Ok(Some(ReinforcedResult {
                            content: "WAITING_FOR_USER_APPROVAL".to_string(),
                            title: format!("Bash({})", command_str),
                            summary: "Waiting for approval".to_string(),
                            is_error: false,
                            error_type: None,
                            display_type: "text".to_string(),
                        }));
                    }
                }
            }
        }

        // 4. Security: FS Path Guard
        if [
            TOOL_READ_FILE,
            TOOL_WRITE_FILE,
            TOOL_LIST_DIR,
            TOOL_EDIT_FILE,
        ]
        .contains(&name)
        {
            if let Some(path_str) = args["file_path"].as_str().or_else(|| args["path"].as_str()) {
                let guard = self.path_guard.read().map_err(|e| {
                    WorkflowEngineError::General(format!("PathGuard lock poisoned: {}", e))
                })?;
                guard.validate(
                    std::path::Path::new(path_str),
                    self.policy.path_restriction == PathRestriction::SandboxOnly,
                )?;
            }
        }

        // 5. User Interaction Pause
        if name == TOOL_ASK_USER {
            self.update_state(WorkflowState::Paused).await?;
            return Ok(Some(ReinforcedResult {
                content: "Waiting for user".into(),
                title: "AskUser".to_string(),
                summary: "Asked user".to_string(),
                is_error: false,
                error_type: None,
                display_type: "text".to_string(),
            }));
        }

        Ok(None)
    }

    /// Handles summarization, Todo context enhancement, and result reinforcement after a tool has run.
    async fn post_process_tool_result(
        &mut self,
        name: &str,
        args: &serde_json::Value,
        tool_call: &serde_json::Value,
        result: Result<serde_json::Value, crate::tools::ToolError>,
    ) -> Result<ReinforcedResult, WorkflowEngineError> {
        // 1. Finish Task Early Return
        if name == TOOL_FINISH_TASK {
            return Ok(ReinforcedResult {
                content: "Finished".into(),
                title: "FinishTask".to_string(),
                summary: "Task completed".to_string(),
                is_error: false,
                error_type: None,
                display_type: "text".to_string(),
            });
        }

        // 2. Auto-summarization for web_fetch
        if name == TOOL_WEB_FETCH {
            if let Ok(val) = &result {
                let content_opt = val
                    .get("structured_content")
                    .and_then(|sc| sc.get("content"))
                    .and_then(|v| v.as_str())
                    .or_else(|| val.get("content").and_then(|v| v.as_str()));

                if let Some(content) = content_opt {
                    if content.len() > 15000 {
                        match self
                            .intelligence_manager
                            .summarize_web_content(content, &self.context)
                            .await
                        {
                            Ok(summary) if !summary.trim().is_empty() => {
                                let url = args["url"].as_str().unwrap_or("");
                                return Ok(ReinforcedResult {
                                    content: format!("<webpage>\n<url>{}</url>\n<content>\n{}\n</content>\n\n<system-reminder>\n[Auto-Summarized] High-fidelity filtered content.\n</system-reminder>\n</webpage>", url, summary),
                                    title: format!("Fetch({})", url),
                                    summary: "Content summarized (XML)".to_string(),
                                    is_error: false,
                                    error_type: None,
                                    display_type: "text".to_string(),
                                });
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        // 3. Reinforce with Todo Context
        if name.starts_with("todo_") {
            let todos = if let Ok(store) = self.context.main_store.read() {
                store
                    .get_todo_list_for_workflow(&self.session_id)
                    .unwrap_or_default()
            } else {
                vec![]
            };
            Ok(ObservationReinforcer::reinforce_with_context(
                tool_call,
                &result,
                Some(serde_json::json!(todos)),
            ))
        } else {
            Ok(ObservationReinforcer::reinforce(tool_call, &result))
        }
    }

    async fn update_state(&mut self, new_state: WorkflowState) -> Result<(), WorkflowEngineError> {
        self.state = new_state.clone();
        self.gateway
            .send(&self.session_id, GatewayPayload::State { state: new_state })
            .await?;
        Ok(())
    }

    pub(crate) async fn add_message_and_notify_internal(
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
                }

                // Merge tool calls into metadata (Support multiple formats)
                let mut tool_calls_to_track = Vec::new();
                if let Some(tool_calls) = json_msg.get("tool_calls").and_then(|v| v.as_array()) {
                    tool_calls_to_track.extend(tool_calls.clone());
                } else if let Some(tool) = json_msg.get("tool") {
                    tool_calls_to_track.push(tool.clone());
                } else if json_msg.get("name").is_some() {
                    // Single tool object format
                    tool_calls_to_track.push(json_msg.clone());
                }

                if !tool_calls_to_track.is_empty() {
                    let mut meta_obj = metadata.unwrap_or(serde_json::json!({}));
                    meta_obj["tool_calls"] = serde_json::json!(tool_calls_to_track);
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
                self.add_message_and_notify_internal(
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
                                // Validate that the path exists and is a directory
                                if abs_p.exists() && abs_p.is_dir() {
                                    abs_p.canonicalize().ok().or(Some(abs_p))
                                } else {
                                    log::warn!(
                                        "WorkflowExecutor {}: Invalid path ignored: {:?}",
                                        self.session_id,
                                        abs_p
                                    );
                                    None
                                }
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
}
