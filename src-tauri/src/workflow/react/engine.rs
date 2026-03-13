use async_trait::async_trait;
use rust_i18n::t;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tokio::time::{sleep, Duration};

use crate::ai::chat::openai::OpenAIChat;
use crate::ai::interaction::chat_completion::AiChatEnum;
use crate::ai::interaction::chat_completion::ChatState;
use crate::ccproxy::ChatProtocol;
use crate::db::{Agent, MainStore, ModelConfig, WorkflowMessage};
use crate::tools::{
    ToolCategory, ToolManager, ToolScope, MCP_TOOL_NAME_SPLIT, TOOL_ASK_USER, TOOL_FINISH_TASK,
    TOOL_SUBMIT_PLAN, TOOL_WEB_FETCH,
};
use crate::workflow::react::compression::ContextCompressor;
use crate::workflow::react::context::ContextManager;
use crate::workflow::react::error::WorkflowEngineError;
use crate::workflow::react::gateway::Gateway;
use crate::workflow::react::intelligence::IntelligenceManager;
use crate::workflow::react::llm::LlmProcessor;
use crate::workflow::react::loop_detector::LoopDetector;
use crate::workflow::react::memory::{MemoryManager, MemoryScope};
use crate::workflow::react::memory_analyzer::MemoryAnalyzer;
use crate::workflow::react::observation::{ObservationReinforcer, ReinforcedResult};
use crate::workflow::react::orchestrator::SubAgentFactory;
use crate::workflow::react::policy::{ExecutionPhase, ExecutionPolicy};
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
        attached_context: Option<String>,
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
    fn messages(&self) -> Vec<WorkflowMessage>;
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
    pub subagent_type: Option<String>,
    /// Rules and permissions for this ReAct session.
    pub policy: ExecutionPolicy,
    /// Memory manager for reading/writing memory files
    pub memory_manager: Arc<MemoryManager>,
    /// Detects repetitive tool calls within a sliding window.
    pub(crate) loop_detector: LoopDetector,
    /// Memory cache for tools awaiting user approval.
    pub(crate) pending_approvals: Arc<dashmap::DashMap<String, serde_json::Value>>,
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
        attached_context: Option<String>,
        reasoning: Option<String>,
        step_type: Option<StepType>,
        is_error: bool,
        error_type: Option<String>,
        metadata: Option<serde_json::Value>,
    ) -> Result<bool, WorkflowEngineError> {
        self.add_message_and_notify_internal(
            role,
            content,
            attached_context,
            reasoning,
            step_type,
            is_error,
            error_type,
            metadata,
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

    fn messages(&self) -> Vec<WorkflowMessage> {
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
        resource_path: Option<PathBuf>,
        subagent_type: Option<String>,
        signal_rx: Option<tokio::sync::mpsc::Receiver<String>>,
        tsid_generator: Arc<crate::libs::tsid::TsidGenerator>,
        global_tool_manager: Arc<ToolManager>,
        policy: ExecutionPolicy,
    ) -> Self {
        let session_id_clone = session_id.clone();
        let session_id_clone2 = session_id.clone();
        let session_id_clone3 = session_id.clone();
        let chat_state_clone = chat_state.clone();
        let chat_state_clone2 = chat_state.clone();
        let chat_state_clone3 = chat_state.clone();

        // Create skill_scanner first to get skill_paths
        let skill_scanner = SkillScanner::new(app_data_dir.clone(), resource_path);
        let skill_paths: Vec<PathBuf> = skill_scanner.get_search_paths();

        // Build sandbox_paths
        let mut sandbox_paths = vec![app_data_dir.join("planning").join(&session_id)];
        sandbox_paths.push(std::env::temp_dir());
        if let Some(home) = dirs::home_dir() {
            sandbox_paths.push(home.join(".chatspeed"));
        }

        let path_guard = Arc::new(RwLock::new(PathGuard::new(
            allowed_paths.clone(),
            sandbox_paths,
            skill_paths,
        )));
        let path_guard_clone = path_guard.clone();

        let max_contexts = agent_config.max_contexts.unwrap_or(128000);
        let max_steps = if policy.phase == ExecutionPhase::Planning {
            10
        } else {
            agent_config
                .max_contexts
                .map(|ctx| ((ctx as usize) / 2000).clamp(20, 200))
                .unwrap_or(DEFAULT_MAX_STEPS)
        };

        let mut auto_approve = HashSet::new();
        if let Some(s) = &agent_config.auto_approve {
            if let Ok(v) = serde_json::from_str::<Vec<String>>(s) {
                for tool in v {
                    auto_approve.insert(tool);
                }
            }
        }

        let models_val: serde_json::Value = if let Some(m) = &agent_config.models {
            serde_json::from_str(m).unwrap_or_else(|_| serde_json::json!({}))
        } else {
            serde_json::json!({})
        };

        let act_model = models_val.get("act").cloned().unwrap_or_else(|| {
            serde_json::json!({
                "id": agent_config.act_model.as_deref().and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok()).and_then(|v| v["id"].as_i64()),
                "model": agent_config.act_model.as_deref().and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok()).and_then(|v| v["model"].as_str().map(|s| s.to_string()))
            })
        });

        let initial_provider_id = act_model["id"].as_i64().unwrap_or(0);
        let initial_model_name = act_model["model"].as_str().unwrap_or_default().to_string();

        let mut executor = Self {
            session_id,
            context: ContextManager::new(
                session_id_clone,
                main_store,
                max_contexts as usize,
                tsid_generator.clone(),
            ),
            tool_manager: Arc::new(ToolManager::new()),
            global_tool_manager,
            chat_state,
            gateway,
            sub_agent_factory,
            compressor: ContextCompressor::new(
                chat_state_clone,
                initial_provider_id,
                initial_model_name.clone(),
            ),
            path_guard,
            skill_scanner,
            llm_processor: LlmProcessor::new(
                session_id_clone2,
                agent_config.clone(),
                HashMap::new(),
                path_guard_clone,
                chat_state_clone2,
                initial_provider_id,
                initial_model_name.clone(),
                false,  // Temporary, will be updated below
                vec![], // MCP tool summaries will be set in init_internal
                // Memory manager and project root
                {
                    let project_root = allowed_paths.first().cloned();
                    Arc::new(MemoryManager::new(project_root))
                },
                allowed_paths.first().cloned(),
            ),
            intelligence_manager: IntelligenceManager::new(
                session_id_clone3,
                chat_state_clone3,
                initial_provider_id,
                initial_model_name.clone(),
            ),
            available_skills: HashMap::new(),
            agent_config,
            state: WorkflowState::Pending,
            current_step: 0,
            max_steps,
            consecutive_no_tool_calls: 0,
            auto_approve,
            signal_rx,
            tsid_generator,
            subagent_type,
            policy,
            memory_manager: {
                let project_root = allowed_paths.first().cloned();
                Arc::new(MemoryManager::new(project_root))
            },
            loop_detector: LoopDetector::new(),
            pending_approvals: Arc::new(dashmap::DashMap::new()),
        };

        // Initialize reasoning flag by piercing proxy if necessary
        let actual_config =
            executor.resolve_actual_model_config(initial_provider_id, &initial_model_name);
        if let Some(crate::db::ModelConfig {
            reasoning: Some(true),
            ..
        }) = actual_config
        {
            executor.llm_processor.reasoning = true;
        } else {
            executor.llm_processor.reasoning =
                crate::ai::util::is_reasoning_supported(&initial_model_name.to_lowercase());
        }

        executor
    }

    pub(crate) async fn init_internal(&mut self) -> Result<(), WorkflowEngineError> {
        self.context.load_history().await?;
        self.available_skills = self.skill_scanner.scan()?;
        self.llm_processor.available_skills = self.available_skills.clone();

        // Get MCP tool summaries for workflow
        self.llm_processor.mcp_tool_summaries = self
            .global_tool_manager
            .get_tool_calling_spec(Some(ToolScope::Workflow), None)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|t| t.name.contains(MCP_TOOL_NAME_SPLIT))
            .map(|mut t| {
                t.input_schema = serde_json::json!({});
                t
            })
            .collect();

        self.register_foundation_tools().await?;

        // Sync TODO list on initialization
        let _ = self.sync_todo_list().await;

        if let Some(last_msg) = self.context.messages.last() {
            self.current_step = last_msg.step_index as usize;

            // Restore AwaitingApproval state: Rebuild pending_approvals and re-trigger UI signals
            if self.state == WorkflowState::AwaitingApproval {
                log::info!(
                    "WorkflowExecutor {}: Re-broadcasting pending approvals to UI",
                    self.session_id
                );

                // 1. Find all 'tool' messages that are currently in an awaiting approval state
                for msg in &self.context.messages {
                    if msg.role == "tool" {
                        if let Some(meta) = &msg.metadata {
                            let summary =
                                meta.get("summary").and_then(|v| v.as_str()).unwrap_or("");
                            // Match both localized and raw versions of the awaiting status
                            let is_waiting = summary == "Awaiting approval"
                                || summary == rust_i18n::t!("workflow.awaiting_approval");

                            if is_waiting {
                                if let Some(id) = meta.get("tool_call_id").and_then(|v| v.as_str())
                                {
                                    // 2. Find the corresponding assistant message to restore original arguments
                                    let tool_info = self.context.messages.iter().rev().find_map(|m| {
                                        if m.role == "assistant" {
                                            if let Some(m_meta) = &m.metadata {
                                                if let Some(calls) = m_meta.get("tool_calls").and_then(|v| v.as_array()) {
                                                    for call in calls {
                                                        let call_id = call.get("id").and_then(|v| v.as_str());
                                                        if call_id == Some(id) {
                                                            let name = call.get("name").or_else(|| call.get("function").and_then(|f| f.get("name"))).and_then(|v| v.as_str());
                                                            let args = call.get("arguments").or_else(|| call.get("function").and_then(|f| f.get("arguments")));
                                                            if let (Some(n), Some(a)) = (name, args) {
                                                                return Some(json!({ "name": n, "arguments": a }));
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        None
                                    });

                                    if let Some(info) = tool_info {
                                        log::info!("WorkflowExecutor {}: Restored and Re-notifying UI for tool: {} (ID: {})", self.session_id, info["name"], id);
                                        self.pending_approvals.insert(id.to_string(), info.clone());

                                        // 3. Re-trigger the Gateway signal so the UI automatically displays the confirmation block
                                        let _ = self
                                            .gateway
                                            .send(
                                                &self.session_id,
                                                GatewayPayload::Confirm {
                                                    id: id.to_string(),
                                                    action: info["name"]
                                                        .as_str()
                                                        .unwrap_or("unknown")
                                                        .to_string(),
                                                    details: msg.message.clone(), // Use preserved preview content (Diff/Summary)
                                                },
                                            )
                                            .await;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Generate title if current workflow has no title set
        let should_generate_title = {
            let store = self
                .context
                .main_store
                .read()
                .map_err(|e| WorkflowEngineError::General(e.to_string()))?;
            store
                .get_workflow_snapshot(&self.session_id)
                .ok()
                .and_then(|snapshot| snapshot.workflow.title)
                .map_or(true, |t| t.trim().is_empty())
        };

        if should_generate_title {
            let user_query = self.context.get_initial_query();
            if !user_query.is_empty() {
                log::info!(
                    "WorkflowExecutor {}: Generating title for workflow with empty title",
                    self.session_id
                );
                let im = IntelligenceManager::new(
                    self.session_id.clone(),
                    self.chat_state.clone(),
                    self.llm_processor.active_provider_id,
                    self.llm_processor.active_model_name.clone(),
                );
                if let Ok(title) = im.generate_workflow_title(&user_query).await {
                    let store = self
                        .context
                        .main_store
                        .read()
                        .map_err(|e| WorkflowEngineError::General(e.to_string()))?;
                    let _ = store.update_workflow_title(&self.session_id, &title);
                }
            }
        }
        Ok(())
    }

    async fn register_foundation_tools(&self) -> Result<(), WorkflowEngineError> {
        use crate::tools::*;
        let tm: &Arc<ToolManager> = &self.tool_manager;
        let _ = tm.clear(true).await;

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
                self.policy.phase == ExecutionPhase::Planning,
            )))
            .await?;
        }

        // 3. Interaction Tools
        if self
            .policy
            .allowed_categories
            .contains(&ToolCategory::Interaction)
        {
            if is_allowed(TOOL_ASK_USER) {
                tm.register_tool(Arc::new(AskUser)).await?;
            }
            if is_allowed(TOOL_SUBMIT_PLAN) {
                tm.register_tool(Arc::new(SubmitPlan)).await?;
            }
            if self.policy.phase != ExecutionPhase::Planning && is_allowed(TOOL_FINISH_TASK) {
                tm.register_tool(Arc::new(FinishTask)).await?;
            }
        }

        // 4. Multi-Agent & Skill Tools
        if self
            .policy
            .allowed_categories
            .contains(&ToolCategory::Skill)
        {
            tm.register_tool(Arc::new(SkillExecute::new(self.available_skills.clone())))
                .await?;
            tm.register_tool(Arc::new(SkillListReferences::new(
                self.available_skills.clone(),
            )))
            .await?;
            tm.register_tool(Arc::new(SkillLoadReference::new(
                self.available_skills.clone(),
            )))
            .await?;

            // CRITICAL: Prevent infinite recursion by only allowing the TaskTool (Sub-agent creation)
            // if the current executor is NOT itself a sub-agent.
            if self.subagent_type.is_none() {
                tm.register_tool(Arc::new(
                    crate::workflow::react::orchestrator::TaskTool::new(
                        self.sub_agent_factory.clone(),
                        self.tsid_generator.clone(),
                    ),
                ))
                .await?;
            }

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

        // 7. MCP Tool Loader (if MCP tools are available)
        if self
            .policy
            .allowed_categories
            .contains(&ToolCategory::System)
        {
            // Check if there are any MCP tools
            let has_mcp_tools = self
                .global_tool_manager
                .get_tool_calling_spec(None, None)
                .await
                .map(|specs| specs.iter().any(|s| s.name.contains(MCP_TOOL_NAME_SPLIT)))
                .unwrap_or(false);

            if has_mcp_tools {
                tm.register_tool(Arc::new(McpToolLoad {
                    tool_manager: self.global_tool_manager.clone(),
                }))
                .await?;
            }
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
        {
            // Check stop signal at loop start
            if self.check_stop_signal(&mut signal_rx).await? {
                break;
            }

            // Handle Paused or AwaitingApproval state - wait for user signal
            if self.state == WorkflowState::Paused || self.state == WorkflowState::AwaitingApproval
            {
                let signal_str = signal_rx
                    .recv()
                    .await
                    .ok_or_else(|| WorkflowEngineError::General("Signal channel closed".into()))?;
                // ... (existing signal handling logic)
                let signal_json: Value = serde_json::from_str(&signal_str)
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
                    let signal_id = signal_json["id"].as_str().unwrap_or("unknown");

                    if approved {
                        // 1. Retrieve the stashed tool details from the server-side map
                        let (tool_name, tool_args) = if let Some(stashed) =
                            self.pending_approvals.get(signal_id)
                        {
                            let name = stashed["name"].as_str().unwrap_or("unknown").to_string();
                            let args = stashed["arguments"].clone();
                            (name, args)
                        } else {
                            log::warn!(
                                "WorkflowExecutor {}: Approval received for unknown ID: {}",
                                self.session_id,
                                signal_id
                            );
                            // Fallback to signal payload if map lookup fails (legacy/edge case)
                            let name = signal_json["tool_name"]
                                .as_str()
                                .unwrap_or("unknown")
                                .to_string();
                            let args = signal_json["tool_args"].clone();
                            (name, args)
                        };

                        log::info!(
                            "WorkflowExecutor {}: User APPROVED tool '{}'{} (ID: {})",
                            self.session_id,
                            tool_name,
                            if approve_all { " (Approve All)" } else { "" },
                            signal_id
                        );

                        if approve_all {
                            self.auto_approve.insert(tool_name.to_string());
                        }

                        // 2. Execution: Try local then global
                        let result = if let Ok(res) = self
                            .tool_manager
                            .tool_call(&tool_name, tool_args.clone())
                            .await
                        {
                            Ok(res)
                        } else {
                            self.global_tool_manager
                                .tool_call(&tool_name, tool_args.clone())
                                .await
                        };

                        let tool_call_obj = serde_json::json!({
                            "id": signal_id,
                            "name": tool_name,
                            "arguments": tool_args
                        });

                        // 3. Post-processing and Notification
                        let reinforced = self
                            .post_process_tool_result(
                                &tool_name,
                                &tool_args,
                                &tool_call_obj,
                                result,
                            )
                            .await?;

                        self.add_message_and_notify_internal(
                            "tool".to_string(),
                            reinforced.content,
                            None,
                            None,
                            Some(StepType::Observe),
                            reinforced.is_error,
                            reinforced.error_type.clone(),
                            Some(serde_json::json!({
                                "tool_call_id": signal_id,
                                "tool_name": tool_name,
                                "title": reinforced.title,
                                "summary": reinforced.summary,
                                "is_error": reinforced.is_error,
                                "error_type": reinforced.error_type,
                                "display_type": reinforced.display_type
                            })),
                        )
                        .await?;
                    } else {
                        // Handle Rejection
                        let (tool_name, tool_args) = if let Some(stashed) =
                            self.pending_approvals.get(signal_id)
                        {
                            let name = stashed["name"].as_str().unwrap_or("unknown").to_string();
                            let args = stashed["arguments"].clone();
                            (name, args)
                        } else {
                            (
                                signal_json["tool_name"]
                                    .as_str()
                                    .unwrap_or("unknown")
                                    .to_string(),
                                signal_json["tool_args"].clone(),
                            )
                        };

                        log::info!(
                            "WorkflowExecutor {}: User REJECTED tool '{}' (ID: {})",
                            self.session_id,
                            tool_name,
                            signal_id
                        );

                        let pretty_title = {
                            let primary_root = self
                                .path_guard
                                .read()
                                .unwrap()
                                .get_primary_root()
                                .map(|p| p.to_path_buf());
                            ObservationReinforcer::generate_title(
                                &tool_name,
                                &tool_args,
                                None,
                                primary_root.as_deref(),
                            )
                        };
                        let observation = format!("The user doesn't want to proceed with this tool use. The tool '{}' was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait for the user to tell you how to proceed.", tool_name);

                        self.add_message_and_notify_internal(
                            "tool".to_string(),
                            observation,
                            None,
                            None,
                            Some(StepType::Observe),
                            true,
                            Some("UserRejected".to_string()),
                            Some(serde_json::json!({
                                "tool_call_id": signal_id,
                                "tool_name": tool_name,
                                "title": pretty_title, // Restored the missing title
                                "summary": "User rejected",
                                "is_error": true,
                                "error_type": "UserRejected"
                            })),
                        )
                        .await?;
                    }

                    // 4. Clean up the stashed approval entry
                    self.pending_approvals.remove(signal_id);

                    self.update_state(WorkflowState::Thinking).await?;
                    continue;
                } else {
                    let user_input = signal_json["content"].as_str().unwrap_or("").to_string();
                    self.add_message_and_notify_internal(
                        "user".to_string(),
                        user_input,
                        None,
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

            // Handle AwaitingAutoApproval state - trigger automatic transition
            if self.state == WorkflowState::AwaitingAutoApproval {
                log::info!(
                    "WorkflowExecutor {}: Processing internal auto-approval signal...",
                    self.session_id
                );

                // 1. Find the latest SubmitPlan call in history to extract the finalized plan
                let plan_content = self.context.messages.iter().rev().find_map(|m| {
                    if let Some(meta) = &m.metadata {
                        if let Some(tc) = meta.get("tool_calls").and_then(|v| v.as_array()) {
                            for call in tc {
                                if call["name"] == "submit_plan"
                                    || call["function"]["name"] == "submit_plan"
                                {
                                    let args = call.get("arguments").or_else(|| {
                                        call.get("function").and_then(|f| f.get("arguments"))
                                    });
                                    if let Some(val) = args {
                                        let parsed = if let Some(s) = val.as_str() {
                                            serde_json::from_str::<Value>(s).unwrap_or(val.clone())
                                        } else {
                                            val.clone()
                                        };
                                        return parsed
                                            .get("plan")
                                            .and_then(|p| p.as_str())
                                            .map(|s| s.to_string());
                                    }
                                }
                            }
                        }
                    }
                    None
                });

                if let Some(plan) = plan_content {
                    log::info!(
                        "WorkflowExecutor {}: Plan found, executing atomic transition to Implementation phase",
                        self.session_id
                    );

                    // A. Context Pruning: Keep only Initial Query + Final Plan + Todo List
                    self.context.prune_for_execution(plan).await?;

                    // B. Policy & State Reset
                    let mut new_policy = ExecutionPolicy::implementation();
                    new_policy.approval_level = self.policy.approval_level.clone();
                    self.policy = new_policy;

                    // Reset step count for the new phase to ensure clear history separation
                    self.current_step = 0;
                    self.consecutive_no_tool_calls = 0;

                    // C. Toolset Refresh: CRITICAL for giving the AI the right tools for the new phase
                    self.register_foundation_tools().await?;

                    self.update_state(WorkflowState::Thinking).await?;
                    continue;
                } else {
                    log::warn!(
                        "WorkflowExecutor {}: AwaitingAutoApproval triggered but no plan found in history. Reverting to thinking.",
                        self.session_id
                    );
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
                            "<SYSTEM_REMINDER>STEP BUDGET EXHAUSTED: You have used {} out of {} \
                            allowed steps. You MUST conclude the task immediately. Do NOT perform \
                            any more research. Call 'finish_task' with a summary of what you have \
                            found so far, clearly noting any incomplete sections.</SYSTEM_REMINDER>",
                            self.current_step, self.max_steps
                        ),
                        None,
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
                            "<SYSTEM_REMINDER>STEP BUDGET WARNING: You are at step {} of {}. \
                            Only {} steps remain. Start wrapping up: complete your most critical \
                            pending tasks and prepare a final answer. Avoid starting new research \
                            threads.</SYSTEM_REMINDER>",
                            self.current_step,
                            self.max_steps,
                            self.max_steps - self.current_step
                        ),
                        None,
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

            let agent_config = self.agent_config.clone();
            let models_val: serde_json::Value = if let Some(m) = &agent_config.models {
                serde_json::from_str(m).unwrap_or_else(|_| serde_json::json!({}))
            } else {
                serde_json::json!({})
            };

            let act_model = models_val.get("act").cloned().unwrap_or_else(|| {
                serde_json::json!({
                    "id": agent_config.act_model.as_deref().and_then(|s| serde_json::from_str::<Value>(s).ok()).and_then(|v| v["id"].as_i64()),
                    "model": agent_config.act_model.as_deref().and_then(|s| serde_json::from_str::<Value>(s).ok()).and_then(|v| v["model"].as_str().map(|s| s.to_string()))
                })
            });

            let plan_model = models_val.get("plan").cloned().unwrap_or_else(|| {
                serde_json::json!({
                    "id": agent_config.plan_model.as_deref().and_then(|s| serde_json::from_str::<Value>(s).ok()).and_then(|v| v["id"].as_i64()),
                    "model": agent_config.plan_model.as_deref().and_then(|s| serde_json::from_str::<Value>(s).ok()).and_then(|v| v["model"].as_str().map(|s| s.to_string()))
                })
            });

            let target_type = self
                .subagent_type
                .as_ref()
                .cloned()
                .unwrap_or_else(|| "General".to_string());
            let model_config = if self.policy.phase == ExecutionPhase::Planning {
                if !plan_model["model"].is_null() {
                    plan_model
                } else {
                    act_model.clone()
                }
            } else {
                match target_type.as_str() {
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
                }
            };

            let _model_name = model_config["model"].as_str().unwrap_or("").to_string();
            let provider_id = model_config["id"].as_i64().unwrap_or(0);

            // Pierce proxy to get actual model capabilities if needed
            let actual_config = self.resolve_actual_model_config(provider_id, &_model_name);

            self.llm_processor.active_provider_id = provider_id;
            self.llm_processor.active_model_name = _model_name.clone();

            // Priority: Model Config (reasoning field) > Auto-detection
            if let Some(crate::db::ModelConfig {
                reasoning: Some(true),
                ..
            }) = actual_config
            {
                self.llm_processor.reasoning = true;
            } else {
                self.llm_processor.reasoning =
                    crate::ai::util::is_reasoning_supported(&_model_name.to_lowercase());
            }

            self.intelligence_manager.active_provider_id = self.llm_processor.active_provider_id;
            self.intelligence_manager.active_model_name =
                self.llm_processor.active_model_name.clone();
            self.compressor.provider_id = self.llm_processor.active_provider_id;
            self.compressor.model = self.llm_processor.active_model_name.clone();

            let chat_interface = {
                let mut chats_guard = self.chat_state.chats.lock().await;
                let protocol = ChatProtocol::OpenAI;
                let chat_map = chats_guard
                    .entry(protocol)
                    .or_insert_with(std::collections::HashMap::new);
                chat_map
                    .entry(self.session_id.clone())
                    .or_insert_with(|| crate::create_chat!(self.context.main_store))
                    .clone()
            };

            let mut available_tools = self
                .tool_manager
                .get_tool_calling_spec(None, None)
                .await
                .unwrap_or_default();

            // 5. Include MCP tools from global manager if allowed by policy
            if self.policy.allowed_categories.contains(&ToolCategory::Mcp) {
                if let Ok(global_specs) = self
                    .global_tool_manager
                    .get_tool_calling_spec(None, None)
                    .await
                {
                    for spec in global_specs {
                        // Only add if it's an MCP tool (identified by name pattern)
                        if spec.name.contains(crate::tools::MCP_TOOL_NAME_SPLIT) {
                            available_tools.push(spec);
                        }
                    }
                }
            }

            let (full_response, tool_calls_json, response_reasoning, usage) = self
                .llm_processor
                .call(
                    &mut self.context,
                    self.current_step,
                    chat_interface,
                    self.gateway.clone(),
                    available_tools,
                    self.max_steps,
                    &self.policy,
                    &mut signal_rx,
                )
                .await?;

            let mut needs_compression = false;
            let mut assistant_metadata = usage.unwrap_or_else(|| serde_json::json!({}));
            if !tool_calls_json.is_empty() {
                if let Ok(tc_val) = serde_json::from_str::<serde_json::Value>(&tool_calls_json) {
                    // Extract strictly the array of tool calls to comply with OpenAI/Claude protocols.
                    let calls_array = if let Some(array) = tc_val.as_array() {
                        array.clone()
                    } else if let Some(array) = tc_val.get("tool_calls").and_then(|v| v.as_array())
                    {
                        array.clone()
                    } else if let Some(tool_obj) = tc_val.get("tool") {
                        vec![tool_obj.clone()]
                    } else if tc_val.is_object() && tc_val.get("name").is_some() {
                        vec![tc_val]
                    } else {
                        vec![]
                    };

                    if !calls_array.is_empty() {
                        assistant_metadata["tool_calls"] = serde_json::json!(calls_array);
                    }
                }
            }

            let compressed_signal = self
                .add_message_and_notify_internal(
                    "assistant".to_string(),
                    full_response.clone(),
                    None,
                    Some(response_reasoning),
                    Some(StepType::Think),
                    false,
                    None,
                    Some(assistant_metadata),
                )
                .await?;
            if compressed_signal {
                needs_compression = true;
            }

            self.update_state(WorkflowState::Executing).await?;
            let (results, has_todo) = self
                .execute_tools(full_response, tool_calls_json, &mut signal_rx)
                .await?;

            for (id, reinforced, original_call) in &results {
                // Extract tool name from the original call for the metadata
                let tool_name = original_call
                    .get("name")
                    .or_else(|| original_call.get("function").and_then(|f| f.get("name")))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");

                let _ = self
                    .add_message_and_notify_internal(
                        "tool".to_string(),
                        reinforced.content.clone(),
                        None,
                        None,
                        Some(StepType::Observe),
                        reinforced.is_error,
                        reinforced.error_type.clone(),
                        Some(serde_json::json!({
                            "tool_call_id": id,
                            "tool_name": tool_name, // CRITICAL: Added for LlmProcessor's recovery logic
                            "title": reinforced.title,
                            "summary": reinforced.summary,
                            "is_error": reinforced.is_error,
                            "error_type": reinforced.error_type,
                            "display_type": reinforced.display_type
                        })),
                    )
                    .await?;
            }

            if results.is_empty() {
                self.consecutive_no_tool_calls += 1;
                log::warn!(
                    "WorkflowExecutor {}: No tool calls in response (consecutive: {})",
                    self.session_id,
                    self.consecutive_no_tool_calls
                );
                let error_msg = if self.consecutive_no_tool_calls >= 3 {
                    let remaining = self.max_steps.saturating_sub(self.current_step);
                    format!(
                        "<SYSTEM_REMINDER>NO TOOL CALLS DETECTED: You have not called any tools for {} consecutive responses. \
                        This is wasting your limited step budget ({}/{} steps used, {} remaining). \
                        If you are truly finished, provide a summary and call 'finish_task'.</SYSTEM_REMINDER>",
                        self.consecutive_no_tool_calls, self.current_step, self.max_steps, remaining
                    )
                } else {
                    "<SYSTEM_REMINDER>Error: No tool call detected in your last response. You MUST call a tool to proceed. If you have finished your task, call 'finish_task' AFTER providing a summary in plain text.</SYSTEM_REMINDER>".to_string()
                };
                let _ = self
                    .add_message_and_notify_internal(
                        "user".to_string(),
                        error_msg,
                        None,
                        None,
                        Some(StepType::Observe),
                        false,
                        None,
                        None,
                    )
                    .await?;
                if self.consecutive_no_tool_calls >= 3 {
                    self.consecutive_no_tool_calls = 0;
                }
                // Skip further processing since there are no tool results
                sleep(Duration::from_millis(100)).await;
                continue;
            } else {
                self.consecutive_no_tool_calls = 0;
            }

            if results
                .iter()
                .any(|r| r.1.title == "Finish Task" && !r.1.is_error)
            {
                self.update_state(WorkflowState::Completed).await?;
                break;
            }

            if has_todo {
                self.sync_todo_list().await?;
            }

            // --- TRIGGER CONTEXT COMPRESSION IF NEEDED ---
            if needs_compression {
                // Notify frontend: compression starting
                self.gateway
                    .send(
                        &self.session_id,
                        GatewayPayload::CompressionStatus {
                            is_compressing: true,
                            message: t!("workflow.compression_in_progress").to_string(),
                        },
                    )
                    .await?;

                self.gateway
                    .send(
                        &self.session_id,
                        GatewayPayload::Notification {
                            message: t!("workflow.compression_in_progress").to_string(),
                            category: Some("info".to_string()),
                        },
                    )
                    .await?;

                let compression_result = self.compressor.compress(&self.context.messages).await;

                // Notify frontend: compression ended
                self.gateway
                    .send(
                        &self.session_id,
                        GatewayPayload::CompressionStatus {
                            is_compressing: false,
                            message: String::new(),
                        },
                    )
                    .await?;

                self.gateway
                    .send(
                        &self.session_id,
                        GatewayPayload::Notification {
                            message: String::new(),
                            category: Some("info".to_string()),
                        },
                    )
                    .await?;

                if let Ok(summary) = compression_result {
                    let _ = self
                        .context
                        .compress(summary, self.current_step as i32)
                        .await;
                }
            }

            sleep(Duration::from_millis(50)).await;
        }

        // === Memory Analysis After Workflow Completion ===
        // Analyze user inputs and update memories if needed
        if let Err(e) = self.analyze_and_update_memories().await {
            log::warn!("Memory analysis failed: {}", e);
            // Don't fail the workflow if memory analysis fails
        }

        self.signal_rx = Some(signal_rx);
        Ok(())
    }

    /// Analyzes user inputs and updates memory files.
    async fn analyze_and_update_memories(&self) -> Result<(), WorkflowEngineError> {
        // 1. Collect user inputs (filter out observe type)
        let user_inputs: Vec<String> = self
            .context
            .messages
            .iter()
            .filter(|m| m.role == "user" && m.step_type.as_ref().map_or(true, |st| st != "observe"))
            .map(|m| m.message.clone())
            .collect();

        // Skip if no user inputs
        if user_inputs.is_empty() {
            log::debug!("No user inputs to analyze for memory");
            return Ok(());
        }

        // 2. Read current memories (using Arc directly, no lock needed)
        let current_global = self.memory_manager.read(MemoryScope::Global).ok().flatten();
        let current_project = self
            .memory_manager
            .read(MemoryScope::Project)
            .ok()
            .flatten();

        // 3. Call memory analyzer
        let analysis = MemoryAnalyzer::analyze(
            user_inputs,
            current_global.clone(),
            current_project.clone(),
            self.chat_state.clone(),
            &self.session_id,
            self.llm_processor.active_provider_id,
            &self.llm_processor.active_model_name,
        )
        .await
        .map_err(|e| WorkflowEngineError::General(format!("Memory analysis failed: {}", e)))?;

        // 4. Update memories if changed
        if let Some(new_global) = analysis.global_memory {
            let old_global = current_global.unwrap_or_default();
            if new_global.trim() != old_global.trim() {
                self.memory_manager
                    .write(MemoryScope::Global, &new_global)
                    .map_err(|e| {
                        WorkflowEngineError::General(format!(
                            "Failed to write global memory: {}",
                            e
                        ))
                    })?;
                log::info!("Global memory updated");
            }
        }

        if let Some(new_project) = analysis.project_memory {
            let old_project = current_project.unwrap_or_default();
            if new_project.trim() != old_project.trim() {
                if self.memory_manager.has_project_memory() {
                    self.memory_manager
                        .write(MemoryScope::Project, &new_project)
                        .map_err(|e| {
                            WorkflowEngineError::General(format!(
                                "Failed to write project memory: {}",
                                e
                            ))
                        })?;
                    log::info!("Project memory updated");
                }
            }
        }

        Ok(())
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
        let mut tool_calls = vec![];

        // --- 1. Parse Tool Calls from JSON ---
        if !json_part.is_empty() {
            let cleaned_json = crate::libs::util::format_json_str(&json_part);
            if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&cleaned_json) {
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
            }
        }

        if tool_calls.is_empty() {
            return Ok((Vec::new(), false));
        }

        // Quick stop check before audit
        if self.check_stop_signal(signal_rx).await? {
            return Err(WorkflowEngineError::Cancelled(
                t!("workflow.cancelled").to_string(),
            ));
        }

        let mut has_todo_call = false;

        // Use a map to collect results and a list to maintain original AI call order
        let mut result_map: HashMap<String, (ReinforcedResult, serde_json::Value)> = HashMap::new();
        let mut call_order: Vec<String> = Vec::new();

        let mut parallel_execution_queue = Vec::new();
        let mut sequential_execution_queue = Vec::new();

        // --- 2. Stage 1: Audit & Partitioning (The 'Channel' Logic) ---
        // We determine what can run NOW and what must WAIT BEFORE performing any actions.
        for (_idx, call) in tool_calls.into_iter().enumerate() {
            // Use the ID already provided in the call if available, otherwise generate one
            let id = call
                .get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| crate::ccproxy::get_tool_id());

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

            call_order.push(id.clone());

            // --- CAUSAL BLOCKING CHECK ---
            // If a PREVIOUS tool in this TURN has already transitioned the engine to a blocking state
            // (like AwaitingApproval or Paused), we MUST NOT run or even audit subsequent tools.
            // They are simply postponed until the previous turn's block is resolved.
            if self.state == WorkflowState::AwaitingApproval || self.state == WorkflowState::Paused
            {
                log::info!(
                    "WorkflowExecutor {}: Postponing tool '{}' due to Turn-Level block (Causality)",
                    self.session_id,
                    name
                );
                result_map.insert(id, (ReinforcedResult {
                    content: "<SYSTEM_REMINDER>Action postponed. A preceding tool in this turn is awaiting user intervention. Please re-issue this command if still necessary once the previous action is resolved.</SYSTEM_REMINDER>".into(),
                    title: format!("Postponed: {}", name),
                    summary: "Turn blocked".to_string(),
                    is_error: false,
                    error_type: None,
                    display_type: "text".to_string(),
                }, call));
                continue;
            }

            // --- SEMANTIC AUDIT ---
            match self.pre_dispatch_check(&id, &name, &args, &text_part).await {
                Ok(Some(early_result)) => {
                    // Tool was intercepted (Approval needed, Loop detected, etc.)
                    result_map.insert(id, (early_result, call));
                    // Note: If pre_dispatch_check changed state to AwaitingApproval,
                    // the next iterations will hit the 'CAUSAL BLOCKING CHECK' above.
                }
                Ok(None) => {
                    // Safe to proceed to physical execution!
                    if name.starts_with("todo_") {
                        has_todo_call = true;
                        sequential_execution_queue.push((id, name, args, call));
                    } else {
                        parallel_execution_queue.push((id, name, args, call));
                    }
                }
                Err(e) => return Err(e),
            }
        }

        // --- 3. Stage 2: Safe Physical Execution ---
        // Final stop check before starting physical tool execution
        if self.check_stop_signal(signal_rx).await? {
            return Err(WorkflowEngineError::Cancelled(
                t!("workflow.cancelled").to_string(),
            ));
        }

        // We now execute only the tools that cleared the audit phase.

        // Phase A: Parallel Batch (I/O heavy tools like read_file, web_fetch)
        if !parallel_execution_queue.is_empty() {
            use futures::stream::{FuturesOrdered, StreamExt};
            let mut tool_futures = FuturesOrdered::new();
            let tm = self.tool_manager.clone();
            let gtm = self.global_tool_manager.clone();
            let semaphore = self.context.semaphore.clone();

            for (id, name, args, call) in parallel_execution_queue {
                let tm_clone = tm.clone();
                let gtm_clone = gtm.clone();
                let semaphore_clone = semaphore.clone();
                tool_futures.push_back(async move {
                    let _permit = semaphore_clone.acquire().await.ok();

                    let final_res = if name.contains(crate::tools::MCP_TOOL_NAME_SPLIT) {
                        // MCP tools are managed globally
                        gtm_clone.tool_call(&name, args.clone()).await
                    } else {
                        // Native tools are managed session-locally. No fallback.
                        tm_clone.tool_call(&name, args.clone()).await
                    };
                    (id, name, args, call, final_res)
                });
            }

            while let Some((id, name, args, call, res)) = tool_futures.next().await {
                let reinforced = self
                    .post_process_tool_result(&name, &args, &call, res)
                    .await?;
                result_map.insert(id, (reinforced, call));
            }
        }

        // Phase B: Sequential Batch (State-sensitive tools like todo_*)
        for (id, name, args, call) in sequential_execution_queue {
            let final_res = if name.contains(crate::tools::MCP_TOOL_NAME_SPLIT) {
                self.global_tool_manager
                    .tool_call(&name, args.clone())
                    .await
            } else {
                self.tool_manager.tool_call(&name, args.clone()).await
            };

            let reinforced = self
                .post_process_tool_result(&name, &args, &call, final_res)
                .await?;
            result_map.insert(id, (reinforced, call));
        }

        // --- 4. Stage 3: Protocol Finalization ---
        // Reassemble all results (executed + postponed) in the order AI requested them.
        let final_results = call_order
            .into_iter()
            .filter_map(|id| {
                let (reinforced, call) = result_map.remove(&id)?;
                Some((id, reinforced, call))
            })
            .collect();

        Ok((final_results, has_todo_call))
    }

    /// Optimized version of post-processing that ensures reinforced results reflect the system state.
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
                title: "Finish Task".to_string(),
                summary: t!("workflow.task_finished").to_string(),
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
                                    content: format!("<webpage>\n<url>{}</url>\n<content>\n{}\n</content>\n\n<SYSTEM_REMINDER>\n[Auto-Summarized] High-fidelity filtered content.\n</SYSTEM_REMINDER>\n</webpage>", url, summary),
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

        // 3. Reinforce with Todo Context (Freshly fetched from DB)
        let primary_root = self
            .path_guard
            .read()
            .unwrap()
            .get_primary_root()
            .map(|p| p.to_path_buf());
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
                primary_root.as_deref(),
            ))
        } else {
            Ok(ObservationReinforcer::reinforce_with_context(
                tool_call,
                &result,
                None,
                primary_root.as_deref(),
            ))
        }
    }

    /// Performs safety and logic checks BEFORE a tool is executed.
    /// Returns Some(ReinforcedResult) if the check fails or requires an early return (e.g. Paused for confirmation).
    async fn pre_dispatch_check(
        &mut self,
        id: &str,
        name: &str,
        args: &serde_json::Value,
        text_part: &str,
    ) -> Result<Option<ReinforcedResult>, WorkflowEngineError> {
        // --- 1. Workflow Control Interception (Submit, Finish, Ask) ---
        match name {
            TOOL_SUBMIT_PLAN => return self.handle_submit_plan_intercept(text_part).await,
            TOOL_FINISH_TASK => return self.handle_finish_task_intercept(text_part).await,
            TOOL_ASK_USER => return self.handle_ask_user_intercept(args).await,
            _ => {}
        }

        // --- 2. Security & Runtime Checks (Bash, FS, Loops) ---
        // CRITICAL: These must happen BEFORE approval checks to ensure hard security boundaries
        // are never bypassed by user approval (especially in sensitive phases like Planning).

        // 2.1 Loop Detection
        if let Some(warning) = self.loop_detector.record_and_check(name, args) {
            log::warn!(
                "WorkflowExecutor {}: Loop detected for tool '{}'. Intercepting...",
                self.session_id,
                name
            );
            return Ok(Some(ReinforcedResult {
                content: warning,
                title: format!("Loop Check: {}", name),
                summary: "Loop detected".to_string(),
                is_error: true,
                error_type: Some("LoopDetected".to_string()),
                display_type: "text".to_string(),
            }));
        }

        // 2.2 Shell Auditing
        if name == crate::tools::TOOL_BASH {
            if let Some(result) = self.handle_bash_security_intercept(id, args).await? {
                return Ok(Some(result));
            }
        }

        // 2.3 FS Path Guard
        if [
            crate::tools::TOOL_READ_FILE,
            crate::tools::TOOL_WRITE_FILE,
            crate::tools::TOOL_LIST_DIR,
            crate::tools::TOOL_EDIT_FILE,
            crate::tools::TOOL_GLOB,
            crate::tools::TOOL_GREP,
        ]
        .contains(&name)
        {
            if let Some(result) = self.handle_fs_path_guard_intercept(name, args)? {
                return Ok(Some(result));
            }
        }

        // --- 3. Approval Policy Enforcement ---
        // Only if security checks pass do we consider asking the user for permission.
        if self.should_intercept_for_approval(name, args) {
            return self
                .handle_approval_interception(id, name, args, None)
                .await;
        }

        Ok(None)
    }

    pub(crate) async fn update_state(
        &mut self,
        new_state: WorkflowState,
    ) -> Result<(), WorkflowEngineError> {
        self.state = new_state.clone();

        // Cleanup pending approvals when transitioning away from approval-waiting states
        if matches!(
            new_state,
            WorkflowState::Thinking | WorkflowState::Executing | WorkflowState::Completed
        ) {
            self.pending_approvals.clear();
        }

        self.gateway
            .send(
                &self.session_id,
                GatewayPayload::State {
                    state: new_state.clone(),
                },
            )
            .await?;
        let store_res = self
            .context
            .main_store
            .write()
            .map_err(|e| WorkflowEngineError::General(e.to_string()));
        if let Ok(store) = store_res {
            let _ = store.update_workflow_status(&self.session_id, &new_state.to_string());
        }
        Ok(())
    }

    pub(crate) async fn add_message_and_notify_internal(
        &mut self,
        role: String,
        mut content: String,
        attached_context: Option<String>,
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

        let (msg, needs_compression) = self
            .context
            .add_message(
                role.clone(),
                content.clone(),
                attached_context,
                reasoning.clone(),
                step_type.clone(),
                self.current_step as i32,
                is_error,
                error_type,
                metadata.clone(),
            )
            .await?;

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
                    error_type: None,
                    metadata,
                },
            )
            .await?;

        Ok(needs_compression
            || msg
                .metadata
                .as_ref()
                .map_or(false, |m| m["type"] == "summary"))
    }

    /// Checks if a stop signal is pending in the channel
    async fn check_stop_signal(
        &mut self,
        rx: &mut tokio::sync::mpsc::Receiver<String>,
    ) -> Result<bool, WorkflowEngineError> {
        while let Ok(s) = rx.try_recv() {
            let sig_json: Value = serde_json::from_str(&s).unwrap_or_default();
            if sig_json["type"] == "stop" || s.contains("stop") {
                self.update_state(WorkflowState::Cancelled).await?;
                return Ok(true);
            } else if sig_json["type"] == "user_input" {
                let user_input = sig_json["content"].as_str().unwrap_or("").to_string();

                // 1. Reset counters for the NEW sub-task
                log::info!(
                    "WorkflowExecutor {}: Resetting step budget for new user input",
                    self.session_id
                );
                self.current_step = 0;
                self.consecutive_no_tool_calls = 0;

                // 2. Combine user input with the system reminder for a cleaner UI and consistent context
                let combined_content = format!(
                    "{}\n\n<SYSTEM_REMINDER>To ensure sufficient reasoning depth for your follow-up request, the task step budget has been reset. Current step: 0.</SYSTEM_REMINDER>",
                    user_input
                );

                self.add_message_and_notify_internal(
                    "user".to_string(),
                    combined_content,
                    None,
                    None,
                    None,
                    false,
                    None,
                    None,
                )
                .await?;
            } else if sig_json["type"] == "update_allowed_paths" {
                if let Some(paths_arr) = sig_json.get("paths").and_then(|v| v.as_array()) {
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
        Ok(false)
    }

    async fn sync_todo_list(&self) -> Result<(), WorkflowEngineError> {
        let todos = {
            let store = self
                .context
                .main_store
                .read()
                .map_err(|e| WorkflowEngineError::General(e.to_string()))?;
            store.get_todo_list_for_workflow(&self.session_id)?
        };
        self.gateway
            .send(
                &self.session_id,
                GatewayPayload::SyncTodo {
                    todo_list: json!(todos),
                },
            )
            .await?;
        Ok(())
    }

    /// Resolves the actual ModelConfig by piercing through proxy aliases if necessary.
    /// Returns the ModelConfig of the first target in a proxy group, or the direct model config.
    fn resolve_actual_model_config(
        &self,
        provider_id: i64,
        model_name: &str,
    ) -> Option<ModelConfig> {
        let store = self.context.main_store.read().ok()?;

        if provider_id == 0 {
            // Proxy mode: parse "group@alias"
            let (group, alias) = if let Some(pos) = model_name.find('@') {
                (&model_name[..pos], &model_name[pos + 1..])
            } else {
                ("default", model_name)
            };

            let proxy_config: crate::ccproxy::ChatCompletionProxyConfig =
                store.get_config(crate::constants::CFG_CHAT_COMPLETION_PROXY, HashMap::new());

            let target = proxy_config.get(group)?.get(alias)?.first()?;

            // Recurse once to get the actual model config from the first target
            self.resolve_actual_model_config(target.id, &target.model)
        } else {
            // Provider mode: get model by provider_id
            let ai_model = store.config.get_ai_model_by_id(provider_id).ok()?;
            ai_model.models.into_iter().find(|m| m.id == model_name)
        }
    }
}
