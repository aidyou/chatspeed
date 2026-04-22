use async_trait::async_trait;
use rust_i18n::t;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tokio::time::{sleep, Duration};

use crate::ai::chat::openai::OpenAIChat;
use crate::ai::interaction::chat_completion::AiChatEnum;
use crate::ai::interaction::chat_completion::ChatState;
use crate::ccproxy::ChatProtocol;
use crate::db::{Agent, MainStore, ModelConfig, WorkflowMessage};
use crate::tools::{
    ToolManager, ToolScope, MCP_TOOL_NAME_SPLIT, TOOL_ASK_USER,
    TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY, TOOL_SUBMIT_PLAN, TOOL_SUBMIT_RESULT, TOOL_WEB_FETCH,
};
use crate::workflow::react::policy::ApprovalLevel;
use crate::workflow::react::{
    compression::ContextCompressor,
    context::ContextManager,
    dispatcher::{Dispatcher, DispatcherConfig},
    error::WorkflowEngineError,
    events::WorkflowEvent,
    gateway::Gateway,
    intelligence::IntelligenceManager,
    llm::LlmProcessor,
    loop_detector::LoopDetector,
    manager::WorkflowManager,
    memory::{MemoryManager, MemoryScope},
    memory_analyzer::MemoryAnalyzer,
    observation::{ObservationKind, ObservationReinforcer, ReinforcedResult},
    orchestrator::SubAgentFactory,
    policy::{ExecutionPhase, ExecutionPolicy},
    runtime_observation::{
        enrich_runtime_observation_metadata, runtime_observation_metadata, RuntimeObservationType,
    },
    security::PathGuard,
    signals::{parse_runtime_signal, take_stashed_user_messages, RuntimeSignal, SignalType},
    sinks::{DBSink, Sink, TauriSink},
    skills::{SkillManifest, SkillScanner},
    types::{
        ExecutionContext, GatewayPayload, PendingTool, RuntimeState, StepType, SubAgentCompletion,
        WaitReason, WorkflowSignal, WorkflowState,
    },
};

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
    pub last_compression_step: usize,
    pub last_compression_boundary_id: Option<i64>,
    pub background_compression_boundary_id: Option<i64>,
    pub background_compression_retry_state: HashMap<i64, (u32, usize)>,
    /// Rules and permissions for this ReAct session.
    pub policy: ExecutionPolicy,
    /// Memory manager for reading/writing memory files
    pub memory_manager: Arc<MemoryManager>,
    /// Detects repetitive tool calls within a sliding window.
    pub(crate) loop_detector: LoopDetector,
    /// Memory cache for tools awaiting user approval.
    pub(crate) pending_approvals: Arc<dashmap::DashMap<String, serde_json::Value>>,
    /// One-shot cache of bash commands approved by Smart AI review to prevent
    /// the generic approval path from re-intercepting the same command.
    pub(crate) smart_approved_bash_commands: HashSet<String>,
    /// One-shot cache of tool call ids approved by Smart AI review to prevent
    /// the generic approval path from re-intercepting the same tool call.
    pub(crate) smart_approved_tool_call_ids: HashSet<String>,
    /// Flag indicating recovery failed - session is in safe-failed read-only state.
    pub(crate) recovery_failed: bool,
    /// Error message if recovery failed.
    pub(crate) recovery_error: Option<String>,
    /// Optional dispatcher for event distribution (Phase 6)
    pub dispatcher: Option<Arc<Dispatcher>>,
    /// Buffered user messages received during non-waiting execution stages.
    pub queued_user_messages: VecDeque<(String, String)>,
    /// Phase 7: ID of sub-agent this session is waiting for (Call model)
    pub sub_agent_id: Option<String>,
    /// Phase 7: All sub-agent session IDs created by this parent session.
    pub sub_agent_sessions: Vec<String>,
    /// Durable sub-agent completions restored from snapshot but not yet consumed.
    pub pending_sub_agent_completions: Vec<SubAgentCompletion>,
}

impl WorkflowExecutor {
    fn extract_call_mode_sub_agent_task_id(
        args: &serde_json::Value,
        result: &serde_json::Value,
    ) -> Option<String> {
        if args
            .get("execution_mode")
            .and_then(|value| value.as_str())
            .unwrap_or("call")
            != "call"
        {
            return None;
        }

        let parsed_content = result
            .get("content")
            .and_then(|value| value.as_str())
            .and_then(|content| serde_json::from_str::<serde_json::Value>(content).ok());
        let structured = result.get("structured_content");
        let status = structured
            .and_then(|value| value.get("status"))
            .and_then(|value| value.as_str())
            .or_else(|| {
                parsed_content
                    .as_ref()
                    .and_then(|value| value.get("status"))
                    .and_then(|value| value.as_str())
            });

        if status.is_some() && status != Some("waiting") {
            return None;
        }

        structured
            .and_then(|value| value.get("task_id"))
            .and_then(|value| value.as_str())
            .or_else(|| {
                parsed_content
                    .as_ref()
                    .and_then(|value| value.get("task_id"))
                    .and_then(|value| value.as_str())
            })
            .map(|task_id| task_id.to_string())
    }

    fn is_child_agent_workflow(&self) -> bool {
        self.agent_config.role.as_deref() == Some("child")
    }

    fn enrich_tool_observation_metadata(
        tool_name: &str,
        metadata: &mut serde_json::Value,
        reinforced: &ReinforcedResult,
    ) {
        if tool_name == TOOL_SUBMIT_RESULT {
            metadata["structured_content"] = serde_json::json!({
                "status": "completed",
                "result": reinforced.content,
                "summary": reinforced.summary,
            });
        }
    }

    fn build_rejection_observation(tool_name: &str, rejection_message: Option<&str>) -> String {
        let user_prefix = rejection_message
            .map(str::trim)
            .filter(|msg| !msg.is_empty())
            .map(|msg| format!("{}\n", msg))
            .unwrap_or_default();

        format!(
            "{}<SYSTEM_REMINDER>\nThe user has declined the execution of the tool '{}'. No changes were applied.\n\nSince your proposed action was rejected, you should re-evaluate your strategy. Use the 'ask_user' tool to understand the reason for the rejection or to ask the user for alternative instructions before proceeding.\n</SYSTEM_REMINDER>",
            user_prefix,
            tool_name
        )
    }

    fn sanitize_assistant_metadata_for_storage(
        metadata: Option<serde_json::Value>,
    ) -> serde_json::Value {
        let mut sanitized = serde_json::json!({});

        if let Some(meta) = metadata {
            if let Some(tokens) = meta.get("tokens") {
                sanitized["tokens"] = tokens.clone();
            }
        }

        sanitized
    }

    fn parse_approval_level_from_signal(sig_json: &Value) -> Option<(String, ApprovalLevel)> {
        let level_str = sig_json
            .get("approvalLevel")
            .and_then(|v| v.as_str())
            .or_else(|| sig_json.get("level").and_then(|v| v.as_str()))
            .or_else(|| sig_json.get("approval_level").and_then(|v| v.as_str()))?;

        use std::str::FromStr;
        ApprovalLevel::from_str(level_str)
            .ok()
            .map(|level| (level_str.to_string(), level))
    }

    fn phase_model_context_limit(agent_config: &Agent, phase: &ExecutionPhase) -> Option<usize> {
        let models = agent_config.models.as_ref()?;
        let selected_model = match phase {
            ExecutionPhase::Planning => models.plan.as_ref(),
            ExecutionPhase::Implementation | ExecutionPhase::Standard => models.act.as_ref(),
        }?;

        selected_model
            .context_size
            .filter(|value| *value > 0)
            .map(|value| value as usize)
    }

    fn effective_context_limit(agent_config: &Agent, phase: &ExecutionPhase) -> usize {
        Self::phase_model_context_limit(agent_config, phase)
            .or_else(|| {
                agent_config
                    .max_contexts
                    .filter(|value| *value > 0)
                    .map(|value| value as usize)
            })
            .unwrap_or(128000)
    }

    fn effective_max_steps(agent_config: &Agent, phase: &ExecutionPhase) -> usize {
        if *phase == ExecutionPhase::Planning {
            10
        } else {
            (Self::effective_context_limit(agent_config, phase) / 2000).clamp(20, 200)
        }
    }

    fn sync_runtime_limits(&mut self) {
        self.context.max_tokens =
            Self::effective_context_limit(&self.agent_config, &self.policy.phase);
        self.max_steps = Self::effective_max_steps(&self.agent_config, &self.policy.phase);
    }

    async fn persist_compression_summary(
        &mut self,
        summary: String,
        compressed_until_message_id: i64,
    ) -> Result<(), WorkflowEngineError> {
        self.context
            .compress(
                summary,
                self.current_step as i32,
                compressed_until_message_id,
            )
            .await?;
        self.last_compression_step = self.current_step;
        self.last_compression_boundary_id = Some(compressed_until_message_id);
        self.background_compression_boundary_id = None;
        log::info!(
            "[Workflow][session={}][phase=compression] Persisted summary through boundary {}. current_context_tokens={}",
            self.session_id,
            compressed_until_message_id,
            self.context.current_token_estimate()
        );
        self.dispatch_context_usage().await?;
        Ok(())
    }

    fn spawn_background_compression(
        &mut self,
        compression_candidate: Vec<WorkflowMessage>,
        compressed_until_message_id: i64,
    ) {
        self.background_compression_boundary_id = Some(compressed_until_message_id);
        let compressor = self.compressor.clone();
        let session_id = self.session_id.clone();

        tokio::spawn(async move {
            match compressor.compress(&compression_candidate).await {
                Ok(summary) => {
                    let signal = serde_json::to_string(&WorkflowSignal::CompressionReady {
                        compressed_until_message_id,
                        summary,
                    });

                    match signal {
                        Ok(payload) => {
                            if let Err(err) =
                                WorkflowManager::send_signal_to_session(&session_id, payload)
                            {
                                log::warn!(
                                    "[Workflow][session={}][phase=compression] Failed to deliver background compression result for boundary {}: {}",
                                    session_id,
                                    compressed_until_message_id,
                                    err
                                );
                            }
                        }
                        Err(err) => {
                            log::warn!(
                                "[Workflow][session={}][phase=compression] Failed to serialize background compression result for boundary {}: {}",
                                session_id,
                                compressed_until_message_id,
                                err
                            );
                        }
                    }
                }
                Err(err) => {
                    let error_message = err.to_string();
                    let signal = serde_json::to_string(&WorkflowSignal::CompressionFailed {
                        compressed_until_message_id,
                        error: error_message.clone(),
                    });
                    if let Ok(payload) = signal {
                        if let Err(delivery_err) =
                            WorkflowManager::send_signal_to_session(&session_id, payload)
                        {
                            log::warn!(
                                "[Workflow][session={}][phase=compression] Background compression failed for boundary {} and failure signal could not be delivered: {}; original_error={}",
                                session_id,
                                compressed_until_message_id,
                                delivery_err,
                                error_message
                            );
                        }
                    } else {
                        log::warn!(
                            "[Workflow][session={}][phase=compression] Background compression request failed for boundary {}: {}",
                            session_id,
                            compressed_until_message_id,
                            error_message
                        );
                    }
                }
            }
        });
    }

    async fn apply_background_compression_ready(
        &mut self,
        compressed_until_message_id: i64,
        summary: String,
    ) -> Result<(), WorkflowEngineError> {
        if self.last_compression_boundary_id == Some(compressed_until_message_id) {
            self.background_compression_boundary_id = None;
            self.background_compression_retry_state
                .remove(&compressed_until_message_id);
            return Ok(());
        }

        if self.background_compression_boundary_id != Some(compressed_until_message_id) {
            log::info!(
                "[Workflow][session={}][phase=compression] Ignoring stale background compression result for boundary {}. in_flight={:?}",
                self.session_id,
                compressed_until_message_id,
                self.background_compression_boundary_id
            );
            return Ok(());
        }

        self.background_compression_retry_state
            .remove(&compressed_until_message_id);
        self.persist_compression_summary(summary, compressed_until_message_id)
            .await
    }

    fn clear_background_compression_if_matches(&mut self, compressed_until_message_id: i64) {
        if self.background_compression_boundary_id == Some(compressed_until_message_id) {
            self.background_compression_boundary_id = None;
        }
    }

    fn should_retry_background_compression(&self, compressed_until_message_id: i64) -> bool {
        self.background_compression_retry_state
            .get(&compressed_until_message_id)
            .map(|(_, next_retry_step)| self.current_step >= *next_retry_step)
            .unwrap_or(true)
    }

    fn record_background_compression_failure(&mut self, compressed_until_message_id: i64) {
        let (failures, _) = self
            .background_compression_retry_state
            .get(&compressed_until_message_id)
            .copied()
            .unwrap_or((0, self.current_step));
        let next_failures = failures.saturating_add(1);
        let cooldown_steps = match next_failures {
            1 => 2,
            2 => 4,
            3 => 8,
            _ => 16,
        };
        let next_retry_step = self.current_step.saturating_add(cooldown_steps);
        self.background_compression_retry_state.insert(
            compressed_until_message_id,
            (next_failures, next_retry_step),
        );
    }
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
        let skill_scanner = SkillScanner::new(app_data_dir.clone());
        let skill_paths: Vec<PathBuf> = skill_scanner.get_search_paths();

        // Build sandbox_paths
        let mut sandbox_paths = vec![app_data_dir.join("planning").join(&session_id)];
        sandbox_paths.push(std::env::temp_dir());
        if let Some(home) = dirs::home_dir() {
            sandbox_paths.push(home.join(".chatspeed"));
        }

        // Ensure sandbox directories exist before creating PathGuard
        for path in &sandbox_paths {
            if !path.exists() {
                if let Err(e) = std::fs::create_dir_all(path) {
                    log::warn!(
                        "[Workflow][session={}][phase=init] Failed to create sandbox directory {:?}: {}",
                        session_id,
                        path,
                        e
                    );
                }
            }
        }

        let path_guard = Arc::new(RwLock::new(PathGuard::new(
            allowed_paths.clone(),
            sandbox_paths,
            skill_paths,
        )));
        let path_guard_clone = path_guard.clone();

        let max_contexts = Self::effective_context_limit(&agent_config, &policy.phase);
        let max_steps = Self::effective_max_steps(&agent_config, &policy.phase);

        let mut auto_approve = HashSet::new();
        if let Some(s) = &agent_config.auto_approve {
            if let Ok(v) = serde_json::from_str::<Vec<String>>(s) {
                for tool in v {
                    auto_approve.insert(tool);
                }
            }
        }

        // Extract model configs from AgentModels structure
        let act_model_config = agent_config.models.as_ref().and_then(|m| m.act.as_ref());
        let plan_model_config = agent_config.models.as_ref().and_then(|m| m.plan.as_ref());

        let initial_provider_id = act_model_config.map(|m| m.id).unwrap_or(0);
        let initial_model_name = act_model_config
            .map(|m| m.model.clone())
            .unwrap_or_default();

        // 1. Prioritize 'plan' model for IntelligenceManager (Grader/Audit tasks)
        let (audit_provider_id, audit_model_name) = if let Some(plan_mc) = plan_model_config {
            (plan_mc.id, plan_mc.model.clone())
        } else {
            (initial_provider_id, initial_model_name.clone())
        };

        let child_agents_for_llm = main_store
            .read()
            .ok()
            .and_then(|store| store.get_child_agents(&agent_config.id).ok())
            .unwrap_or_default();

        let mut executor = Self {
            session_id,
            context: ContextManager::new(
                session_id_clone,
                main_store.clone(),
                max_contexts,
                tsid_generator.clone(),
            ),
            tool_manager: Arc::new(ToolManager::new()),
            global_tool_manager,
            chat_state,
            gateway: gateway.clone(),
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
                child_agents_for_llm,
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
                audit_provider_id,
                audit_model_name,
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
            last_compression_step: 0,
            last_compression_boundary_id: None,
            background_compression_boundary_id: None,
            background_compression_retry_state: HashMap::new(),
            policy,
            memory_manager: {
                let project_root = allowed_paths.first().cloned();
                Arc::new(MemoryManager::new(project_root))
            },
            loop_detector: LoopDetector::new(),
            pending_approvals: Arc::new(dashmap::DashMap::new()),
            smart_approved_bash_commands: HashSet::new(),
            smart_approved_tool_call_ids: HashSet::new(),
            recovery_failed: false,
            recovery_error: None,
            dispatcher: None,
            queued_user_messages: VecDeque::new(),
            sub_agent_id: None,
            sub_agent_sessions: Vec::new(),
            pending_sub_agent_completions: Vec::new(),
        };

        // Phase 6: wire default dispatcher with UI + DB sinks for all executors.
        let sinks: Vec<Arc<dyn Sink>> = vec![
            Arc::new(TauriSink::new(gateway.clone())) as Arc<dyn Sink>,
            Arc::new(DBSink::new(main_store.clone())) as Arc<dyn Sink>,
        ];
        let dispatcher = Arc::new(Dispatcher::new(sinks, DispatcherConfig::default()));
        Dispatcher::register_session_dispatcher(executor.session_id.clone(), dispatcher.clone());
        executor.dispatcher = Some(dispatcher);

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

    async fn dispatch_ui_payload(
        &self,
        payload: GatewayPayload,
    ) -> Result<(), WorkflowEngineError> {
        if let Some(ref dispatcher) = self.dispatcher {
            if let Err(e) = dispatcher
                .dispatch_ui(self.session_id.clone(), payload.clone())
                .await
            {
                log::warn!(
                    "[Workflow][session={}][phase=dispatcher] UI dispatch failed: {}, falling back to gateway",
                    self.session_id,
                    e
                );
                self.gateway.send(&self.session_id, payload).await?;
            }
            Ok(())
        } else {
            self.gateway.send(&self.session_id, payload).await
        }
    }

    async fn dispatch_sub_agent_progress(&self) {
        let parent_session_id = {
            let store = match self.context.main_store.read() {
                Ok(store) => store,
                Err(e) => {
                    log::warn!(
                        "[Workflow][session={}][phase=sub_agent_progress] Cannot read store: {}",
                        self.session_id,
                        e
                    );
                    return;
                }
            };

            match store.get_workflow_snapshot(&self.session_id) {
                Ok(snapshot) => snapshot.workflow.parent_session_id,
                Err(e) => {
                    log::warn!(
                        "[Workflow][session={}][phase=sub_agent_progress] Cannot read sub-agent workflow: {}",
                        self.session_id,
                        e
                    );
                    None
                }
            }
        };

        let Some(parent_session_id) = parent_session_id else {
            return;
        };

        let wait_reason = match &self.state {
            WorkflowState::Paused => Some(WaitReason::Confirmation),
            WorkflowState::AwaitingUser => Some(WaitReason::UserInput),
            WorkflowState::AwaitingApproval | WorkflowState::AwaitingAutoApproval => {
                Some(WaitReason::Approval)
            }
            WorkflowState::AwaitingSubAgent => Some(WaitReason::SubAgent),
            _ => None,
        };
        let latest_summary = self
            .context
            .messages
            .iter()
            .rev()
            .find(|message| message.role == "assistant" || message.role == "tool")
            .map(|message| {
                message
                    .metadata
                    .as_ref()
                    .and_then(|meta| meta.get("summary").and_then(|value| value.as_str()))
                    .unwrap_or(&message.message)
                    .to_string()
            });
        let tool_calls_count = self
            .context
            .messages
            .iter()
            .filter(|message| message.role == "tool")
            .count();
        let delegated_task = self.context.get_initial_query();
        let payload = GatewayPayload::SubAgentProgress {
            sub_agent_id: self.session_id.clone(),
            parent_session_id: parent_session_id.clone(),
            status: RuntimeState::from(&self.state),
            workflow_state: self.state.clone(),
            wait_reason,
            agent_name: self.subagent_type.clone(),
            task: (!delegated_task.is_empty()).then_some(delegated_task),
            title: self.subagent_type.clone(),
            summary: latest_summary,
            tool_calls_count,
            current_context_tokens: Some(self.context.current_token_estimate()),
            max_context_tokens: Some(self.context.max_tokens),
            is_error: matches!(self.state, WorkflowState::Error | WorkflowState::Cancelled),
            updated_at_ms: chrono::Utc::now().timestamp_millis(),
        };

        let dispatch_result = if let Some(ref dispatcher) = self.dispatcher {
            dispatcher
                .dispatch_ui(parent_session_id.clone(), payload.clone())
                .await
        } else {
            Ok(())
        };

        if let Err(e) = dispatch_result {
            log::warn!(
                "[Workflow][session={}][parent={}][phase=sub_agent_progress] Dispatcher failed: {}, falling back to gateway",
                self.session_id,
                parent_session_id,
                e
            );
            if let Err(e) = self.gateway.send(&parent_session_id, payload).await {
                log::warn!(
                    "[Workflow][session={}][parent={}][phase=sub_agent_progress] Gateway fallback failed: {}",
                    self.session_id,
                    parent_session_id,
                    e
                );
            }
        } else if self.dispatcher.is_none() {
            if let Err(e) = self.gateway.send(&parent_session_id, payload).await {
                log::warn!(
                    "[Workflow][session={}][parent={}][phase=child_progress] Dispatch failed: {}",
                    self.session_id,
                    parent_session_id,
                    e
                );
            }
        }
    }

    async fn dispatch_terminal_with_fallback(
        &self,
        terminal_state: &str,
        fallback_event: &WorkflowEvent,
    ) {
        if let Some(ref dispatcher) = self.dispatcher {
            if let Err(e) = dispatcher
                .dispatch_terminal(self.session_id.clone(), terminal_state.to_string())
                .await
            {
                log::warn!(
                    "[Workflow][session={}] dispatcher.terminal failed: {}",
                    self.session_id,
                    e
                );
                if let Err(e2) = self.append_event(fallback_event) {
                    log::error!(
                        "[Workflow][session={}] workflow.event.append_failed - error={}",
                        self.session_id,
                        e2
                    );
                }
            }
        } else if let Err(e) = self.append_event(fallback_event) {
            log::error!(
                "[Workflow][session={}] workflow.event.append_failed - error={}",
                self.session_id,
                e
            );
        }
    }

    pub(crate) async fn init_internal(&mut self) -> Result<(), WorkflowEngineError> {
        self.context.load_history().await?;
        self.available_skills = self.skill_scanner.scan()?;
        self.llm_processor.available_skills = self.available_skills.clone();

        // Load current state from database
        {
            let store = self
                .context
                .main_store
                .read()
                .map_err(|e| WorkflowEngineError::General(e.to_string()))?;
            if let Ok(snapshot) = store.get_workflow_snapshot(&self.session_id) {
                if let Ok(state) = snapshot.workflow.status.parse::<WorkflowState>() {
                    self.state = state;
                }
            }
        }

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
            if self.state == WorkflowState::Completed || self.state == WorkflowState::Error {
                log::info!(
                    "WorkflowExecutor {}: Resetting current_step for resumed workflow (previous state: {})",
                    self.session_id,
                    self.state
                );
                self.current_step = 0;
                self.update_state(WorkflowState::Thinking).await?;
            } else if self.state == WorkflowState::Cancelled {
                // Reset cancelled state to Thinking so workflow can resume execution
                log::info!(
                    "WorkflowExecutor {}: Resuming from cancelled state, resetting to Thinking",
                    self.session_id
                );
                self.current_step = 0;
                self.update_state(WorkflowState::Thinking).await?;
            } else if self.state == WorkflowState::Paused {
                log::info!(
                    "WorkflowExecutor {}: Workflow was paused, waiting for user to resume",
                    self.session_id
                );
            } else if self.state == WorkflowState::AwaitingUser
                || self.state == WorkflowState::AwaitingApproval
                || self.state == WorkflowState::AwaitingSubAgent
            {
                // Use the new recovery mechanism for AwaitingUser and AwaitingApproval states
                let recovery_result = crate::workflow::react::replay::restore_execution_context(
                    self.context.main_store.clone(),
                    &self.session_id,
                );

                match recovery_result {
                    crate::workflow::react::replay::RecoveryResult::SnapshotHit { context }
                    | crate::workflow::react::replay::RecoveryResult::ReplayFallback { context } => {
                        log::info!(
                            "[Workflow][session={}][phase=restore] Restoring from recovery result: state={:?}, wait_reason={:?}, pending_tools={}",
                            self.session_id,
                            context.state,
                            context.wait_reason,
                            context.pending_tools.len()
                        );

                        self.sub_agent_id = context.waiting_on_sub_agent_id.clone();
                        self.sub_agent_sessions = context.sub_agent_sessions.clone();
                        self.pending_sub_agent_completions =
                            context.pending_sub_agent_completions.clone();

                        if self.state == WorkflowState::AwaitingApproval
                            && context.pending_tools.is_empty()
                        {
                            log::warn!(
                                "[Workflow][session={}][phase=restore] Recovery returned empty pending_tools for AwaitingApproval state",
                                self.session_id
                            );
                        } else {
                            for tool in &context.pending_tools {
                                let info_with_details = json!({
                                    "name": tool.tool_name.clone(),
                                    "arguments": tool.arguments.clone(),
                                    "details": tool.details.clone().unwrap_or_default(),
                                    "display_type": tool.display_type.clone().unwrap_or_else(|| "text".to_string())
                                });
                                self.pending_approvals
                                    .insert(tool.tool_call_id.clone(), info_with_details);

                                let details_value = tool
                                    .details
                                    .as_ref()
                                    .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                                    .unwrap_or(serde_json::Value::Null);
                                let _ = self
                                    .dispatch_ui_payload(GatewayPayload::Confirm {
                                        id: tool.tool_call_id.clone(),
                                        action: tool.tool_name.clone(),
                                        details: details_value,
                                        display_type: tool.display_type.clone(),
                                    })
                                    .await;
                            }
                        }

                        if self.state == WorkflowState::AwaitingSubAgent {
                            log::info!(
                                "[Workflow][session={}][phase=restore] Restored sub-agent waiting: waiting_on_sub_agent_id={:?}, sub_agent_sessions={}",
                                self.session_id,
                                self.sub_agent_id,
                                self.sub_agent_sessions.len()
                            );
                            if let Some(completion) = self
                                .pending_sub_agent_completions
                                .iter()
                                .find(|completion| {
                                    !completion.consumed
                                        && Some(completion.sub_agent_id.as_str())
                                            == self.sub_agent_id.as_deref()
                                })
                                .cloned()
                            {
                                log::info!(
                                    "[Workflow][session={}][phase=restore][event=sub_agent_completion_replay] Replaying durable sub-agent completion {}",
                                    self.session_id,
                                    completion.sub_agent_id
                                );
                                let _ = self
                                    .apply_sub_agent_completion(
                                        completion.sub_agent_id.clone(),
                                        completion.to_signal_result(),
                                    )
                                    .await?;
                            }
                        }
                    }
                    crate::workflow::react::replay::RecoveryResult::SafeFailed {
                        session_id: _,
                        error,
                    } => {
                        log::error!(
                            "[Workflow][session={}][phase=restore] Recovery failed: {}",
                            self.session_id,
                            error
                        );
                        self.recovery_failed = true;
                        self.recovery_error = Some(error.to_string());

                        // Enter safe-failed state (use Error state as safe-failed)
                        self.state = WorkflowState::Error;
                        let _ = self
                            .dispatch_ui_payload(GatewayPayload::Error {
                                message: format!(
                                    "Workflow recovery failed: {}. Session is in read-only safe mode.",
                                    error
                                ),
                            })
                            .await;
                    }
                }
            } else {
                self.current_step = last_msg.step_index as usize;
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
                .map_or(true, |title| title.trim().is_empty())
        };

        if should_generate_title {
            let user_query = self.context.get_initial_query();
            if !user_query.is_empty() {
                log::info!(
                    "WorkflowExecutor {}: Spawning background task to generate workflow title",
                    self.session_id
                );
                let im = IntelligenceManager::new(
                    self.session_id.clone(),
                    self.chat_state.clone(),
                    self.llm_processor.active_provider_id,
                    self.llm_processor.active_model_name.clone(),
                );
                tokio::spawn(async move {
                    let _ = im.generate_workflow_title(&user_query).await;
                });
            }
        }

        if self.state == WorkflowState::Pending {
            let event = WorkflowEvent::workflow_started(
                self.session_id.clone(),
                self.agent_config.id.clone(),
            );
            if let Err(e) = self.append_event(&event) {
                log::error!(
                    "[Workflow][session={}] workflow.event.append_failed - error={}",
                    self.session_id,
                    e
                );
            }
        }

        Ok(())
    }

    async fn dispatch_context_usage(&self) -> Result<(), WorkflowEngineError> {
        self.dispatch_ui_payload(GatewayPayload::ContextUsage {
            total_tokens: self.context.current_token_estimate(),
        })
        .await
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
        let configured_tools = self
            .agent_config
            .available_tools
            .as_deref()
            .and_then(|tools| serde_json::from_str::<Vec<String>>(tools).ok())
            .map(|tools| tools.into_iter().collect::<HashSet<_>>());
        let is_sub_agent = self.subagent_type.is_some();

        // Helper to check if a tool is allowed in Workflow scope
        let is_allowed = |name: &str| {
            if is_sub_agent
                && matches!(
                    name,
                    TOOL_BASH | TOOL_SUB_AGENT_RUN | TOOL_SUB_AGENT_OUTPUT | TOOL_SUB_AGENT_STOP
                )
            {
                return false;
            }

            let scope_allowed = all_meta
                .iter()
                .find(|m| m["id"] == name)
                .map(|m| {
                    let scope = m["scope"].as_str().unwrap_or(ToolScope::Both.as_str());
                    scope == ToolScope::Workflow.as_str() || scope == ToolScope::Both.as_str()
                })
                .unwrap_or(true); // Default to allowed for safety if not found in meta

            let config_allowed = configured_tools.as_ref().map_or(true, |tools| {
                matches!(
                    name,
                    TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY
                        | TOOL_SUB_AGENT_RUN
                        | TOOL_SUB_AGENT_OUTPUT
                        | TOOL_SUB_AGENT_STOP
                ) || tools.contains(name)
                    || (name == "mcp_tool_load"
                        && tools.iter().any(|tool| tool.contains(MCP_TOOL_NAME_SPLIT)))
            });

            scope_allowed && config_allowed
        };

        // 1. Register Web tool
        if self.policy.allowed_categories.contains(&ToolCategory::Web) {
            if is_allowed(TOOL_WEB_SEARCH) {
                if let Ok(ws) = self.global_tool_manager.get_tool(TOOL_WEB_SEARCH).await {
                    tm.register_tool(ws.clone()).await?;
                }
            }
            if is_allowed(TOOL_WEB_FETCH) {
                if let Ok(wf) = self.global_tool_manager.get_tool(TOOL_WEB_FETCH).await {
                    tm.register_tool(wf.clone()).await?;
                }
            }
        }

        // 2. Native FS & Search Tools
        if self
            .policy
            .allowed_categories
            .contains(&ToolCategory::FileSystem)
        {
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
        }

        // 3. Shell Tool (With session-aware policy)
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

            tm.register_tool(Arc::new(
                ShellExecute::new(
                    self.path_guard.clone(),
                    self.tsid_generator.clone(),
                    custom_rules,
                    self.policy.phase == ExecutionPhase::Planning,
                )
                .with_gateway(self.gateway.clone(), self.session_id.clone()),
            ))
            .await?;
        }

        // 4. Interaction Tools
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
            if self.policy.phase != ExecutionPhase::Planning {
                if self.is_child_agent_workflow() {
                    tm.register_tool(Arc::new(SubmitResult)).await?;
                } else if is_allowed(TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY) {
                    tm.register_tool(Arc::new(FinishTask)).await?;
                }
            }
        }

        // 4. Multi-Agent
        if self
            .policy
            .allowed_categories
            .contains(&ToolCategory::System)
        {
            if self.agent_config.skill_enabled.unwrap_or(true) {
                tm.register_tool(Arc::new(SkillExecute::new(self.available_skills.clone())))
                    .await?;
            }

            // CRITICAL: Prevent infinite recursion by only allowing the TaskTool (Sub-agent creation)
            // if the current executor is NOT itself a sub-agent.
            if self.subagent_type.is_none() && is_allowed(TOOL_SUB_AGENT_RUN) {
                let child_agents = self
                    .context
                    .main_store
                    .read()
                    .ok()
                    .and_then(|store| store.get_child_agents(&self.agent_config.id).ok())
                    .unwrap_or_default();

                if !child_agents.is_empty() {
                    tm.register_tool(Arc::new(
                        crate::workflow::react::orchestrator::TaskTool::new(
                            self.sub_agent_factory.clone(),
                            self.context.main_store.clone(),
                            self.gateway.clone(),
                            self.tsid_generator.clone(),
                        )
                        .with_parent_session(self.session_id.clone())
                        .with_child_agents(child_agents),
                    ))
                    .await?;
                }
            }

            if is_allowed(TOOL_SUB_AGENT_OUTPUT) {
                tm.register_tool(Arc::new(
                    crate::workflow::react::orchestrator::TaskOutputTool::new(
                        self.session_id.clone(),
                        self.context.main_store.clone(),
                    ),
                ))
                .await?;
            }
            if is_allowed(TOOL_SUB_AGENT_STOP) {
                tm.register_tool(Arc::new(
                    crate::workflow::react::orchestrator::TaskStopTool::new(
                        self.session_id.clone(),
                    ),
                ))
                .await?;
            }
        }

        // 5. Todo Manager Tools (Session Persistent)
        if self
            .policy
            .allowed_categories
            .contains(&ToolCategory::System)
        {
            if is_allowed(TOOL_TODO_CREATE) {
                tm.register_tool(Arc::new(TodoCreateTool {
                    session_id: self.session_id.clone(),
                    main_store: self.context.main_store.clone(),
                }))
                .await?;
            }
            if is_allowed(TOOL_TODO_LIST) {
                tm.register_tool(Arc::new(TodoListTool {
                    session_id: self.session_id.clone(),
                    main_store: self.context.main_store.clone(),
                }))
                .await?;
            }
            if is_allowed(TOOL_TODO_UPDATE) {
                tm.register_tool(Arc::new(TodoUpdateTool {
                    session_id: self.session_id.clone(),
                    main_store: self.context.main_store.clone(),
                }))
                .await?;
            }
            if is_allowed(TOOL_TODO_GET) {
                tm.register_tool(Arc::new(TodoGetTool {
                    session_id: self.session_id.clone(),
                    main_store: self.context.main_store.clone(),
                }))
                .await?;
            }
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

            if has_mcp_tools && is_allowed("mcp_tool_load") {
                tm.register_tool(Arc::new(McpToolLoad {
                    tool_manager: self.global_tool_manager.clone(),
                }))
                .await?;
            }
        }
        //  MCP is loaded like skills, so we don't register it here.
        // if self.policy.allowed_categories.contains(&ToolCategory::Mcp) {
        //     if let Ok(global_specs) = self
        //         .global_tool_manager
        //         .get_tool_calling_spec(None, None)
        //         .await
        //     {
        //         for spec in global_specs {
        //             // Only add if it's an MCP tool (identified by name pattern)
        //             if spec.name.contains(crate::tools::MCP_TOOL_NAME_SPLIT) {
        //                 available_tools.push(spec);
        //             }
        //         }
        //     }
        // }

        Ok(())
    }

    fn split_shell_command_segments(command: &str) -> Vec<String> {
        let mut segments = Vec::new();
        let mut current = String::new();
        let mut chars = command.chars().peekable();
        let mut in_single_quote = false;
        let mut in_double_quote = false;
        let mut escaped = false;

        while let Some(ch) = chars.next() {
            if escaped {
                current.push(ch);
                escaped = false;
                continue;
            }

            if ch == '\\' && !in_single_quote {
                current.push(ch);
                escaped = true;
                continue;
            }

            if ch == '\'' && !in_double_quote {
                in_single_quote = !in_single_quote;
                current.push(ch);
                continue;
            }

            if ch == '"' && !in_single_quote {
                in_double_quote = !in_double_quote;
                current.push(ch);
                continue;
            }

            if !in_single_quote && !in_double_quote {
                match ch {
                    ';' => {
                        if !current.trim().is_empty() {
                            segments.push(current.trim().to_string());
                        }
                        current.clear();
                        continue;
                    }
                    '&' | '|' => {
                        if chars.peek() == Some(&ch) {
                            let _ = chars.next();
                            if !current.trim().is_empty() {
                                segments.push(current.trim().to_string());
                            }
                            current.clear();
                            continue;
                        }
                    }
                    _ => {}
                }
            }

            current.push(ch);
        }

        if !current.trim().is_empty() {
            segments.push(current.trim().to_string());
        }

        segments
    }

    fn contains_unquoted_shell_redirection(command: &str) -> bool {
        let mut chars = command.chars().peekable();
        let mut in_single_quote = false;
        let mut in_double_quote = false;
        let mut escaped = false;

        while let Some(ch) = chars.next() {
            if escaped {
                escaped = false;
                continue;
            }

            if ch == '\\' && !in_single_quote {
                escaped = true;
                continue;
            }

            if ch == '\'' && !in_double_quote {
                in_single_quote = !in_single_quote;
                continue;
            }

            if ch == '"' && !in_single_quote {
                in_double_quote = !in_double_quote;
                continue;
            }

            if !in_single_quote && !in_double_quote && matches!(ch, '>' | '<') {
                return true;
            }
        }

        false
    }

    fn extract_approvable_shell_stage(command: &str) -> Option<Vec<String>> {
        let trimmed = command.trim();
        if trimmed.is_empty() || trimmed.contains('\n') {
            return None;
        }
        if Self::contains_unquoted_shell_redirection(trimmed) {
            return None;
        }

        let tokens = shlex::split(trimmed)?;
        if tokens.is_empty() {
            return None;
        }

        let mut index = 0;
        while index < tokens.len() {
            let token = tokens[index].as_str();
            if token.contains('=') && !token.starts_with('=') && !token.ends_with('=') {
                let mut parts = token.splitn(2, '=');
                let key = parts.next().unwrap_or("");
                if !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    index += 1;
                    continue;
                }
            }
            break;
        }

        if index >= tokens.len() {
            return None;
        }

        let base = tokens[index].as_str();
        if matches!(base, "cd" | "pushd" | "popd") {
            return None;
        }

        let args = &tokens[index + 1..];
        let inline_script_runners = [
            "python", "python3", "node", "bash", "sh", "zsh", "ruby", "perl",
        ];
        if inline_script_runners.contains(&base)
            && args
                .iter()
                .any(|arg| matches!(arg.as_str(), "-c" | "-e" | "-"))
        {
            return None;
        }

        Some(tokens[index..].to_vec())
    }

    fn approvable_shell_pattern_for_stage(stage_tokens: &[String]) -> Option<String> {
        if stage_tokens.is_empty() {
            return None;
        }

        let base_cmd = stage_tokens[0].as_str();

        let destructive_commands = [
            "rm",
            "rmdir",
            "del",
            "mv",
            "move",
            "chmod",
            "chown",
            "chgrp",
            "kill",
            "killall",
            "pkill",
            "ln",
            "crontab",
            "curl",
            "wget",
            "apt",
            "apt-get",
            "yum",
            "dnf",
            "pacman",
            "brew",
            "docker",
            "podman",
            "systemctl",
            "service",
        ];
        if destructive_commands.contains(&base_cmd) {
            return None;
        }

        let simple_safe_commands = [
            "ls", "dir", "tree", "pwd", "whoami", "id", "groups", "hostname", "date", "uptime",
            "uname", "arch", "env", "printenv", "ps", "df", "free", "mount", "lsblk", "cat",
            "head", "tail", "less", "more", "stat", "file", "wc", "du",
        ];
        if simple_safe_commands.contains(&base_cmd) {
            return Some(format!("^{}($| .*)", regex::escape(base_cmd)));
        }

        let allow_base_with_subcommand = ["git", "cargo", "go", "pnpm", "npm", "yarn"];
        if allow_base_with_subcommand.contains(&base_cmd) {
            let subcommand = stage_tokens
                .iter()
                .skip(1)
                .find(|token| !token.starts_with('-'))
                .map(|token| token.as_str());
            if let Some(subcommand) = subcommand {
                return Some(format!(
                    "^{} {}($| .*)",
                    regex::escape(base_cmd),
                    regex::escape(subcommand)
                ));
            }
            return Some(format!("^{}($| .*)", regex::escape(base_cmd)));
        }

        None
    }

    fn generate_shell_approval_patterns(command: &str) -> Vec<String> {
        let mut patterns = Vec::new();
        for segment in Self::split_shell_command_segments(command) {
            let Some(stage_tokens) = Self::extract_approvable_shell_stage(&segment) else {
                continue;
            };

            if let Some(pattern) = Self::approvable_shell_pattern_for_stage(&stage_tokens) {
                if !patterns.iter().any(|existing| existing == &pattern) {
                    patterns.push(pattern);
                }
            }
        }
        patterns
    }

    fn build_shell_policy_with_patterns(
        existing_policy: &[crate::tools::ShellPolicyRule],
        patterns: &[String],
    ) -> Vec<crate::tools::ShellPolicyRule> {
        let mut updated_policy = existing_policy.to_vec();
        for pattern in patterns {
            if updated_policy.iter().any(|item| item.pattern == *pattern) {
                continue;
            }
            updated_policy.push(crate::tools::ShellPolicyRule {
                pattern: pattern.clone(),
                decision: crate::tools::ShellDecision::Allow,
                description: None,
            });
        }
        updated_policy
    }

    pub(crate) async fn run_loop_internal(&mut self) -> Result<(), WorkflowEngineError> {
        // P0-2: Guard - do not continue execution in safe-failed state
        if self.recovery_failed {
            log::error!(
                "[Workflow][session={}][phase=run_loop] Cannot run loop - session is in safe-failed recovery state",
                self.session_id
            );
            return Err(WorkflowEngineError::General(
                "Cannot run workflow: session is in safe-failed recovery state".to_string(),
            ));
        }

        let mut signal_rx = self
            .signal_rx
            .take()
            .ok_or_else(|| WorkflowEngineError::General("Signal receiver already taken".into()))?;

        while self.state != WorkflowState::Completed
            && self.state != WorkflowState::Error
            && self.state != WorkflowState::Cancelled
        {
            // IMPORTANT: Do NOT pre-drain user_message while already in waiting states.
            // Waiting states must resume only through the unified wait branch below.
            let is_waiting_state = self.state == WorkflowState::Paused
                || self.state == WorkflowState::AwaitingUser
                || self.state == WorkflowState::AwaitingApproval
                || self.state == WorkflowState::AwaitingSubAgent;
            if !is_waiting_state {
                // Check stop/config/user-message queue signals at loop start (non-waiting only)
                if self.check_stop_signal(&mut signal_rx).await? {
                    break;
                }
            }

            // Handle waiting states - wait for appropriate signal
            if self.state == WorkflowState::Paused
                || self.state == WorkflowState::AwaitingUser
                || self.state == WorkflowState::AwaitingApproval
                || self.state == WorkflowState::AwaitingSubAgent
            {
                let wait_reason_enum = match &self.state {
                    WorkflowState::Paused => Some(WaitReason::Confirmation),
                    WorkflowState::AwaitingUser => Some(WaitReason::UserInput),
                    WorkflowState::AwaitingApproval => Some(WaitReason::Approval),
                    WorkflowState::AwaitingSubAgent => Some(WaitReason::SubAgent),
                    _ => None,
                };

                log::info!(
                    "[Workflow][session={}][phase=wait][event=enter] Entering wait state, reason={:?}",
                    self.session_id,
                    wait_reason_enum
                );

                let signal_str = signal_rx
                    .recv()
                    .await
                    .ok_or_else(|| WorkflowEngineError::General("Signal channel closed".into()))?;

                let workflow_signal = WorkflowSignal::parse(&signal_str);

                if let Some(signal) = &workflow_signal {
                    if !signal.is_valid_for(wait_reason_enum.as_ref()) {
                        log::warn!(
                            "[Workflow][session={}][phase=wait][event=signal_rejected] Signal '{}' is not valid for wait_reason {:?}",
                            self.session_id,
                            signal.type_name(),
                            wait_reason_enum
                        );
                        // Continue waiting for a valid signal
                        continue;
                    }

                    log::info!(
                        "[Workflow][session={}][phase=wait][event=signal_received] Signal '{}' accepted for wait_reason {:?}",
                        self.session_id,
                        signal.type_name(),
                        wait_reason_enum
                    );
                }

                // Fall back to JSON parsing for legacy handling
                let signal_json: Value = serde_json::from_str(&signal_str)
                    .unwrap_or(serde_json::json!({ "type": "message", "content": signal_str }));

                let signal_type = signal_json["type"].as_str().unwrap_or("unknown");
                let signal_type_enum = SignalType::from_str(signal_type);
                log::info!(
                    "[Workflow][session={}][phase=wait] Signal received, type={}, wait_reason={:?}",
                    self.session_id,
                    signal_type,
                    wait_reason_enum
                );

                // DEBUG: Log all received signals to help diagnose empty message issue
                #[cfg(debug_assertions)]
                log::debug!(
                    "WorkflowExecutor {}: Received signal while {}: type={}, has_content={}, content={}",
                    self.session_id,
                    self.state,
                    signal_json["type"].as_str().unwrap_or("unknown"),
                    signal_json.get("content").is_some(),
                    signal_json.get("content").and_then(|c| c.as_str()).unwrap_or("<none>")
                );

                if self
                    .handle_runtime_config_signal(&signal_json, signal_type_enum)
                    .await?
                {
                    continue;
                }

                // Handle remove_auto_approved_tool signal
                if signal_type_enum == Some(SignalType::RemoveAutoApprovedTool) {
                    if let Some(tool_name) = signal_json["tool_name"].as_str() {
                        self.remove_auto_approved_tool(tool_name);

                        // Send update event to frontend
                        let tools = self.get_auto_approved_tools();
                        if let Err(e) = self
                            .dispatch_ui_payload(GatewayPayload::AutoApprovedToolsUpdated { tools })
                            .await
                        {
                            log::error!(
                                "WorkflowExecutor {}: Failed to send auto-approved tools update after removal: {}",
                                self.session_id,
                                e
                            );
                        }
                    }
                    continue;
                }

                // Handle remove_shell_policy_item signal
                if signal_type_enum == Some(SignalType::RemoveShellPolicyItem) {
                    if let Some(pattern) = signal_json["pattern"].as_str() {
                        if let Some(updated_policy) = self.remove_shell_policy_item(pattern).await {
                            // Send update event to frontend
                            if let Err(e) = self
                                .dispatch_ui_payload(GatewayPayload::ShellPolicyUpdated {
                                    policy: updated_policy,
                                })
                                .await
                            {
                                log::error!(
                                    "WorkflowExecutor {}: Failed to send shell policy update after removal: {}",
                                    self.session_id,
                                    e
                                );
                            }
                        }
                    }
                    continue;
                }

                // Accept runtime config updates in waiting states to avoid noisy unknown-signal warnings.
                if signal_type_enum == Some(SignalType::UpdateFinalAudit) {
                    let audit = signal_json
                        .get("finalAudit")
                        .and_then(|v| v.as_bool())
                        .or_else(|| signal_json.get("audit").and_then(|v| v.as_bool()));
                    if let Some(audit) = audit {
                        log::info!(
                            "WorkflowExecutor {}: Updating final audit to {} while waiting",
                            self.session_id,
                            audit
                        );
                        self.agent_config.final_audit = Some(audit);
                    }
                    continue;
                }

                if signal_type_enum == Some(SignalType::UpdateApprovalLevel) {
                    self.apply_runtime_approval_level_update(&signal_json, true)
                        .await?;
                    continue;
                }

                if signal_type_enum == Some(SignalType::Stop) {
                    log::info!(
                        "[Workflow][session={}][phase=wait][event=stop] Stop signal received in waiting state",
                        self.session_id
                    );
                    self.update_state(WorkflowState::Cancelled).await?;
                    self.signal_rx = Some(signal_rx);
                    return Ok(());
                }

                // Handle structured WorkflowSignal
                if let Some(signal) = workflow_signal {
                    match signal {
                        WorkflowSignal::RebroadcastPending => {
                            log::info!(
                                "WorkflowExecutor {}: Received RebroadcastPending signal",
                                self.session_id
                            );
                            if self.state == WorkflowState::AwaitingApproval {
                                let items: Vec<_> = self
                                    .pending_approvals
                                    .iter()
                                    .map(|r| (r.key().clone(), r.value().clone()))
                                    .collect();
                                for (id, info) in items {
                                    let details_value = info
                                        .get("details")
                                        .and_then(|v| {
                                            if let Some(s) = v.as_str() {
                                                serde_json::from_str::<serde_json::Value>(s).ok()
                                            } else {
                                                Some(v.clone())
                                            }
                                        })
                                        .unwrap_or(serde_json::Value::Null);
                                    let _ = self
                                        .dispatch_ui_payload(GatewayPayload::Confirm {
                                            id,
                                            action: info["name"]
                                                .as_str()
                                                .unwrap_or("unknown")
                                                .to_string(),
                                            details: details_value,
                                            display_type: info
                                                .get("display_type")
                                                .and_then(|v| v.as_str())
                                                .map(|s| s.to_string()),
                                        })
                                        .await;
                                }
                            }
                            continue;
                        }
                        WorkflowSignal::ApprovalDecision {
                            tool_call_id,
                            approved,
                            approve_all,
                            rejection_message,
                        } => {
                            let event = WorkflowEvent::approval_resolved(
                                self.session_id.clone(),
                                tool_call_id.clone(),
                                approved,
                                approve_all,
                            );
                            if let Err(e) = self.append_event(&event) {
                                log::error!(
                                    "[Workflow][session={}] workflow.event.append_failed - error={}",
                                    self.session_id,
                                    e
                                );
                            }

                            if approved {
                                // 1. Retrieve the stashed tool details from the server-side map
                                let (tool_name, tool_args) = if let Some(stashed) =
                                    self.pending_approvals.get(&tool_call_id)
                                {
                                    let name =
                                        stashed["name"].as_str().unwrap_or("unknown").to_string();
                                    let args = stashed["arguments"].clone();
                                    (name, args)
                                } else {
                                    log::warn!(
                                        "WorkflowExecutor {}: Approval received for unknown ID: {}",
                                        self.session_id,
                                        tool_call_id
                                    );
                                    ("unknown".to_string(), serde_json::json!({}))
                                };

                                log::info!(
                                    "WorkflowExecutor {}: User APPROVED tool '{}'{} (ID: {})",
                                    self.session_id,
                                    tool_name,
                                    if approve_all { " (Approve All)" } else { "" },
                                    tool_call_id
                                );

                                if approve_all {
                                    if tool_name == "bash" {
                                        if let Some(cmd) =
                                            tool_args.get("command").and_then(|v| v.as_str())
                                        {
                                            let wildcard_patterns =
                                                Self::generate_shell_approval_patterns(cmd);
                                            log::info!(
                                                "WorkflowExecutor {}: Generated shell approval patterns {:?} for command '{}'",
                                                self.session_id, wildcard_patterns, cmd
                                            );

                                            if wildcard_patterns.is_empty() {
                                                log::info!(
                                                    "WorkflowExecutor {}: No safe reusable shell approval pattern extracted from '{}'",
                                                    self.session_id,
                                                    cmd
                                                );
                                            }

                                            let mut shell_policy_payload: Option<
                                                Vec<crate::tools::ShellPolicyRule>,
                                            > = None;
                                            if !wildcard_patterns.is_empty() {
                                                if let Ok(store) = self.context.main_store.write() {
                                                    if let Ok(snapshot) = store
                                                        .get_workflow_snapshot(&self.session_id)
                                                    {
                                                        let mut agent_config: serde_json::Value =
                                                            snapshot
                                                                .workflow
                                                                .agent_config
                                                                .and_then(|s| {
                                                                    serde_json::from_str(&s).ok()
                                                                })
                                                                .unwrap_or(serde_json::json!({}));

                                                        let existing_policy = agent_config
                                                            .get("shell_policy")
                                                            .and_then(|v| {
                                                                serde_json::from_value::<
                                                            Vec<crate::tools::ShellPolicyRule>,
                                                        >(v.clone()).ok()
                                                            })
                                                            .unwrap_or_default();

                                                        let updated_policy =
                                                            Self::build_shell_policy_with_patterns(
                                                                &existing_policy,
                                                                &wildcard_patterns,
                                                            );

                                                        agent_config["shell_policy"] =
                                                            serde_json::to_value(&updated_policy)
                                                                .unwrap_or(serde_json::json!([]));

                                                        if let Ok(config_str) =
                                                            serde_json::to_string(&agent_config)
                                                        {
                                                            let _ = store
                                                                .update_workflow_agent_config(
                                                                    &self.session_id,
                                                                    &config_str,
                                                                );
                                                        }
                                                        shell_policy_payload = Some(updated_policy);
                                                    }
                                                }
                                            }
                                            if let Some(policy) = shell_policy_payload {
                                                self.agent_config.shell_policy =
                                                    serde_json::to_string(&policy).ok();
                                                if let Err(e) = self
                                                    .dispatch_ui_payload(
                                                        GatewayPayload::ShellPolicyUpdated {
                                                            policy,
                                                        },
                                                    )
                                                    .await
                                                {
                                                    log::error!(
                                                        "WorkflowExecutor {}: Failed to send shell policy update: {}",
                                                        self.session_id,
                                                        e
                                                    );
                                                }
                                            }
                                        }
                                    } else {
                                        self.auto_approve.insert(tool_name.to_string());

                                        let tools = self.get_auto_approved_tools();
                                        if let Err(e) = self
                                            .dispatch_ui_payload(
                                                GatewayPayload::AutoApprovedToolsUpdated {
                                                    tools: tools.clone(),
                                                },
                                            )
                                            .await
                                        {
                                            log::error!(
                                                "WorkflowExecutor {}: Failed to send auto-approved tools update: {}",
                                                self.session_id,
                                                e
                                            );
                                        }

                                        if let Ok(store) = self.context.main_store.write() {
                                            if let Ok(snapshot) =
                                                store.get_workflow_snapshot(&self.session_id)
                                            {
                                                let mut agent_config: serde_json::Value = snapshot
                                                    .workflow
                                                    .agent_config
                                                    .and_then(|s| serde_json::from_str(&s).ok())
                                                    .unwrap_or(serde_json::json!({}));

                                                agent_config["auto_approve"] =
                                                    serde_json::to_value(&tools)
                                                        .unwrap_or(serde_json::json!([]));

                                                if let Ok(config_str) =
                                                    serde_json::to_string(&agent_config)
                                                {
                                                    let _ = store.update_workflow_agent_config(
                                                        &self.session_id,
                                                        &config_str,
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }

                                // 2. Execution: MCP tools use global, native tools use local
                                // Inject internal tool_call_id for streaming tools
                                // Ensure tool_args is an object (parse if it's a JSON string from fallback)
                                let tool_args_obj = if tool_args.is_string() {
                                    serde_json::from_str::<serde_json::Value>(
                                        tool_args.as_str().unwrap_or("{}"),
                                    )
                                    .unwrap_or_else(|_| tool_args.clone())
                                } else {
                                    tool_args.clone()
                                };

                                let mut enriched_args = tool_args_obj.clone();
                                enriched_args[crate::constants::INTERNAL_PARAM_TOOL_CALL_ID] =
                                    serde_json::json!(tool_call_id);

                                let result =
                                    if tool_name.contains(crate::tools::MCP_TOOL_NAME_SPLIT) {
                                        self.global_tool_manager
                                            .tool_call(&tool_name, enriched_args)
                                            .await
                                    } else {
                                        self.tool_manager.tool_call(&tool_name, enriched_args).await
                                    };

                                let tool_call_obj = serde_json::json!({
                                    "id": tool_call_id,
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

                                // Mark as approved since this went through user approval
                                let mut metadata = serde_json::json!({
                                    "tool_call_id": tool_call_id,
                                    "tool_name": tool_name,
                                    "title": reinforced.title,
                                    "summary": reinforced.summary,
                                    "execution_status": if reinforced.is_error { "failed" } else { "completed" },
                                    "is_error": reinforced.is_error,
                                    "error_type": reinforced.error_type,
                                    "display_type": reinforced.display_type,
                                    "approval_status": "approved"
                                });
                                Self::enrich_tool_observation_metadata(
                                    &tool_name,
                                    &mut metadata,
                                    &reinforced,
                                );
                                self.add_message_and_notify_internal(
                                    "tool".to_string(),
                                    reinforced.content,
                                    None,
                                    None,
                                    Some(StepType::Observe),
                                    reinforced.is_error,
                                    reinforced.error_type.clone(),
                                    Some(metadata),
                                )
                                .await?;

                                self.pending_approvals.remove(&tool_call_id);
                                self.update_state(WorkflowState::Thinking).await?;
                            } else {
                                let tool_name = if let Some(stashed) =
                                    self.pending_approvals.get(&tool_call_id)
                                {
                                    stashed["name"].as_str().unwrap_or("unknown").to_string()
                                } else {
                                    "unknown".to_string()
                                };

                                log::info!(
                                    "WorkflowExecutor {}: User REJECTED tool '{}' (ID: {})",
                                    self.session_id,
                                    tool_name,
                                    tool_call_id
                                );

                                let pretty_title = ObservationReinforcer::generate_title(
                                    &tool_name,
                                    &serde_json::json!({}),
                                    None,
                                    None,
                                );
                                let observation = Self::build_rejection_observation(
                                    &tool_name,
                                    rejection_message.as_deref(),
                                );

                                self.add_message_and_notify_internal(
                                    "user".to_string(),
                                    observation,
                                    None,
                                    None,
                                    Some(StepType::Observe),
                                    true,
                                    Some("UserRejected".to_string()),
                                    Some(serde_json::json!({
                                        "tool_call_id": tool_call_id,
                                        "tool_name": tool_name,
                                        "title": pretty_title,
                                        "summary": "User rejected",
                                        "execution_status": "rejected",
                                        "is_error": true,
                                        "error_type": "UserRejected",
                                        "approval_status": "rejected"
                                    })),
                                )
                                .await?;

                                self.pending_approvals.remove(&tool_call_id);
                                self.update_state(WorkflowState::Thinking).await?;
                            }
                            continue;
                        }
                        WorkflowSignal::Continue => {
                            log::info!(
                                "[Workflow][session={}][phase=wait][event=signal_received] Continue signal accepted for wait_reason {:?}",
                                self.session_id,
                                wait_reason_enum
                            );
                            self.current_step = 0;
                            self.max_steps += DEFAULT_MAX_STEPS;
                            self.update_state(WorkflowState::Thinking).await?;
                            continue;
                        }
                        WorkflowSignal::UserMessage { content, .. } => {
                            let user_content = content.clone();
                            self.add_message_and_notify_internal(
                                "user".to_string(),
                                content,
                                None,
                                None,
                                None,
                                false,
                                None,
                                None,
                            )
                            .await?;
                            let event = WorkflowEvent::user_input_received(
                                self.session_id.clone(),
                                user_content,
                            );
                            if let Err(e) = self.append_event(&event) {
                                log::error!(
                                    "[Workflow][session={}] workflow.event.append_failed - error={}",
                                    self.session_id,
                                    e
                                );
                            }
                            self.update_state(WorkflowState::Thinking).await?;
                            continue;
                        }
                        WorkflowSignal::SubAgentComplete {
                            sub_agent_id,
                            result,
                        } => {
                            log::info!(
                                "[Workflow][session={}][phase=wait][event=sub_agent_complete] Sub-agent {} completed with result {:?}",
                                self.session_id,
                                sub_agent_id,
                                result
                            );

                            if !self
                                .apply_sub_agent_completion(sub_agent_id.clone(), result)
                                .await?
                            {
                                log::warn!(
                                    "[Workflow][session={}][phase=wait][event=sub_agent_mismatch] Received completion for unexpected sub-agent {}. Expected: {:?}",
                                    self.session_id,
                                    sub_agent_id,
                                    self.sub_agent_id
                                );
                            }
                            continue;
                        }
                        WorkflowSignal::CompressionReady {
                            compressed_until_message_id,
                            summary,
                        } => {
                            self.apply_background_compression_ready(
                                compressed_until_message_id,
                                summary,
                            )
                            .await?;
                            continue;
                        }
                        WorkflowSignal::CompressionFailed {
                            compressed_until_message_id,
                            error,
                        } => {
                            self.clear_background_compression_if_matches(
                                compressed_until_message_id,
                            );
                            log::warn!(
                                "[Workflow][session={}][phase=compression] Background rollup compression failed for boundary {}: {}",
                                self.session_id,
                                compressed_until_message_id,
                                error
                            );
                            continue;
                        }
                        _ => {}
                    }
                }

                // Legacy fallback for request_confirm_broadcast
                // Legacy fallback for request_confirm_broadcast (old signal name)
                if signal_type_enum == Some(SignalType::LegacyRequestConfirmBroadcast) {
                    log::info!(
                        "WorkflowExecutor {}: Received request to re-broadcast pending confirmations",
                        self.session_id
                    );
                    if self.state == WorkflowState::AwaitingApproval {
                        let items: Vec<_> = self
                            .pending_approvals
                            .iter()
                            .map(|r| (r.key().clone(), r.value().clone()))
                            .collect();
                        for (id, info) in items {
                            let details_value = info
                                .get("details")
                                .and_then(|v| {
                                    if let Some(s) = v.as_str() {
                                        serde_json::from_str::<serde_json::Value>(s).ok()
                                    } else {
                                        Some(v.clone())
                                    }
                                })
                                .unwrap_or(serde_json::Value::Null);
                            let _ = self
                                .dispatch_ui_payload(GatewayPayload::Confirm {
                                    id,
                                    action: info["name"].as_str().unwrap_or("unknown").to_string(),
                                    details: details_value,
                                    display_type: info
                                        .get("display_type")
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string()),
                                })
                                .await;
                        }
                    }
                    continue;
                }

                // Legacy fallback for approval (old JSON format)
                if signal_type_enum == Some(SignalType::Approval) {
                    let approved = signal_json["approved"].as_bool().unwrap_or(false);
                    let approve_all = signal_json["approve_all"].as_bool().unwrap_or(false);
                    let rejection_message = signal_json["rejection_message"]
                        .as_str()
                        .map(|s| s.to_string());
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
                            if tool_name == "bash" {
                                if let Some(cmd) = tool_args.get("command").and_then(|v| v.as_str())
                                {
                                    let wildcard_patterns =
                                        Self::generate_shell_approval_patterns(cmd);
                                    log::info!(
                                        "WorkflowExecutor {}: Generated shell approval patterns {:?} for command '{}'",
                                        self.session_id, wildcard_patterns, cmd
                                    );

                                    if wildcard_patterns.is_empty() {
                                        log::info!(
                                            "WorkflowExecutor {}: No safe reusable shell approval pattern extracted from '{}'",
                                            self.session_id,
                                            cmd
                                        );
                                    }

                                    let mut shell_policy_payload: Option<
                                        Vec<crate::tools::ShellPolicyRule>,
                                    > = None;
                                    if !wildcard_patterns.is_empty() {
                                        if let Ok(store) = self.context.main_store.write() {
                                            if let Ok(snapshot) =
                                                store.get_workflow_snapshot(&self.session_id)
                                            {
                                                let mut agent_config: serde_json::Value = snapshot
                                                    .workflow
                                                    .agent_config
                                                    .and_then(|s| serde_json::from_str(&s).ok())
                                                    .unwrap_or(serde_json::json!({}));

                                                let existing_policy = agent_config
                                                    .get("shell_policy")
                                                    .and_then(|v| {
                                                        serde_json::from_value::<
                                                            Vec<crate::tools::ShellPolicyRule>,
                                                        >(
                                                            v.clone()
                                                        )
                                                        .ok()
                                                    })
                                                    .unwrap_or_default();

                                                let updated_policy =
                                                    Self::build_shell_policy_with_patterns(
                                                        &existing_policy,
                                                        &wildcard_patterns,
                                                    );

                                                agent_config["shell_policy"] =
                                                    serde_json::to_value(&updated_policy)
                                                        .unwrap_or(serde_json::json!([]));

                                                if let Ok(config_str) =
                                                    serde_json::to_string(&agent_config)
                                                {
                                                    let _ = store.update_workflow_agent_config(
                                                        &self.session_id,
                                                        &config_str,
                                                    );
                                                }
                                                shell_policy_payload = Some(updated_policy);
                                            }
                                        }
                                    }
                                    if let Some(policy) = shell_policy_payload {
                                        self.agent_config.shell_policy =
                                            serde_json::to_string(&policy).ok();
                                        if let Err(e) = self
                                            .dispatch_ui_payload(
                                                GatewayPayload::ShellPolicyUpdated { policy },
                                            )
                                            .await
                                        {
                                            log::error!(
                                                "WorkflowExecutor {}: Failed to send shell policy update: {}",
                                                self.session_id,
                                                e
                                            );
                                        }
                                    }
                                }
                            } else {
                                self.auto_approve.insert(tool_name.to_string());

                                let tools = self.get_auto_approved_tools();
                                if let Err(e) = self
                                    .dispatch_ui_payload(GatewayPayload::AutoApprovedToolsUpdated {
                                        tools: tools.clone(),
                                    })
                                    .await
                                {
                                    log::error!(
                                        "WorkflowExecutor {}: Failed to send auto-approved tools update: {}",
                                        self.session_id,
                                        e
                                    );
                                }

                                if let Ok(store) = self.context.main_store.write() {
                                    if let Ok(snapshot) =
                                        store.get_workflow_snapshot(&self.session_id)
                                    {
                                        let mut agent_config: serde_json::Value = snapshot
                                            .workflow
                                            .agent_config
                                            .and_then(|s| serde_json::from_str(&s).ok())
                                            .unwrap_or(serde_json::json!({}));

                                        agent_config["auto_approve"] = serde_json::to_value(&tools)
                                            .unwrap_or(serde_json::json!([]));

                                        if let Ok(config_str) = serde_json::to_string(&agent_config)
                                        {
                                            let _ = store.update_workflow_agent_config(
                                                &self.session_id,
                                                &config_str,
                                            );
                                        }
                                    }
                                }
                            }
                        }

                        // 2. Execution: MCP tools use global, native tools use local
                        // Inject internal tool_call_id for streaming tools
                        // Ensure tool_args is an object (parse if it's a JSON string from fallback)
                        let tool_args_obj = if tool_args.is_string() {
                            serde_json::from_str::<serde_json::Value>(
                                tool_args.as_str().unwrap_or("{}"),
                            )
                            .unwrap_or_else(|_| tool_args.clone())
                        } else {
                            tool_args.clone()
                        };

                        let mut enriched_args = tool_args_obj.clone();
                        enriched_args[crate::constants::INTERNAL_PARAM_TOOL_CALL_ID] =
                            serde_json::json!(signal_id);

                        let result = if tool_name.contains(crate::tools::MCP_TOOL_NAME_SPLIT) {
                            self.global_tool_manager
                                .tool_call(&tool_name, enriched_args)
                                .await
                        } else {
                            self.tool_manager.tool_call(&tool_name, enriched_args).await
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

                        // Mark as approved since this went through user approval
                        let mut metadata = serde_json::json!({
                            "tool_call_id": signal_id,
                            "tool_name": tool_name,
                            "title": reinforced.title,
                            "summary": reinforced.summary,
                            "execution_status": if reinforced.is_error { "failed" } else { "completed" },
                            "is_error": reinforced.is_error,
                            "error_type": reinforced.error_type,
                            "display_type": reinforced.display_type,
                            "approval_status": "approved"
                        });
                        Self::enrich_tool_observation_metadata(
                            &tool_name,
                            &mut metadata,
                            &reinforced,
                        );
                        self.add_message_and_notify_internal(
                            "tool".to_string(),
                            reinforced.content,
                            None,
                            None,
                            Some(StepType::Observe),
                            reinforced.is_error,
                            reinforced.error_type.clone(),
                            Some(metadata),
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
                        let observation = Self::build_rejection_observation(
                            &tool_name,
                            rejection_message.as_deref(),
                        );

                        self.add_message_and_notify_internal(
                            "user".to_string(),
                            observation,
                            None,
                            None,
                            Some(StepType::Observe),
                            true,
                            Some("UserRejected".to_string()),
                            Some(serde_json::json!({
                                "tool_call_id": signal_id,
                                "tool_name": tool_name,
                                "title": pretty_title,
                                "summary": "User rejected",
                                "execution_status": "rejected",
                                "is_error": true,
                                "error_type": "UserRejected",
                                "approval_status": "rejected"
                            })),
                        )
                        .await?;
                    }

                    // 4. Clean up the stashed approval entry
                    self.pending_approvals.remove(signal_id);

                    self.update_state(WorkflowState::Thinking).await?;
                    continue;
                }

                // Handle structured WorkflowSignal
                if let Some(signal) = WorkflowSignal::parse(&signal_str) {
                    if signal.is_valid_for(wait_reason_enum.as_ref()) {
                        match signal {
                            WorkflowSignal::Continue => {
                                log::info!(
                                    "[Workflow][session={}][phase=wait][event=signal_received] Continue signal accepted for wait_reason {:?}",
                                    self.session_id,
                                    wait_reason_enum
                                );
                                self.current_step = 0;
                                self.max_steps += DEFAULT_MAX_STEPS;
                                self.update_state(WorkflowState::Thinking).await?;
                            }
                            WorkflowSignal::CompressionReady {
                                compressed_until_message_id,
                                summary,
                            } => {
                                self.apply_background_compression_ready(
                                    compressed_until_message_id,
                                    summary,
                                )
                                .await?;
                            }
                            WorkflowSignal::CompressionFailed {
                                compressed_until_message_id,
                                error,
                            } => {
                                self.clear_background_compression_if_matches(
                                    compressed_until_message_id,
                                );
                                log::warn!(
                                    "[Workflow][session={}][phase=compression] Background rollup compression failed for boundary {}: {}",
                                    self.session_id,
                                    compressed_until_message_id,
                                    error
                                );
                            }
                            WorkflowSignal::UserMessage { content, .. } => {
                                let user_content = content.clone();
                                self.add_message_and_notify_internal(
                                    "user".to_string(),
                                    content,
                                    None,
                                    None,
                                    None,
                                    false,
                                    None,
                                    None,
                                )
                                .await?;
                                let event = WorkflowEvent::user_input_received(
                                    self.session_id.clone(),
                                    user_content,
                                );
                                if let Err(e) = self.append_event(&event) {
                                    log::error!(
                                        "[Workflow][session={}] workflow.event.append_failed - error={}",
                                        self.session_id,
                                        e
                                    );
                                }
                                self.update_state(WorkflowState::Thinking).await?;
                            }
                            _ => {}
                        }
                    } else {
                        log::warn!(
                            "[Workflow][session={}][phase=wait][event=signal_rejected] Signal {:?} rejected for wait_reason {:?}",
                            self.session_id,
                            signal,
                            wait_reason_enum
                        );
                    }
                } else {
                    log::warn!(
                        "[Workflow][session={}][phase=wait] Unknown signal type: {}, ignoring",
                        self.session_id,
                        signal_type
                    );
                }
                continue;
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
                    self.sync_runtime_limits();

                    // Persist phase change to database
                    if let Ok(store) = self.context.main_store.write() {
                        if let Ok(snapshot) = store.get_workflow_snapshot(&self.session_id) {
                            let mut agent_config: serde_json::Value = snapshot
                                .workflow
                                .agent_config
                                .and_then(|s| serde_json::from_str(&s).ok())
                                .unwrap_or(serde_json::json!({}));
                            agent_config["phase"] =
                                serde_json::json!(self.policy.phase.to_string());
                            if let Ok(config_str) = serde_json::to_string(&agent_config) {
                                let _ = store
                                    .update_workflow_agent_config(&self.session_id, &config_str);
                            }
                        }
                    }

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
            log::info!(
                "[Workflow][session={}][step] Step {}/{}, approval_level={:?}",
                self.session_id,
                self.current_step,
                self.max_steps,
                self.policy.approval_level
            );

            // --- Max-step budget guard ---
            if self.current_step > self.max_steps {
                log::warn!(
                    "WorkflowExecutor {}: Reached max steps ({}). Asking user for continuation.",
                    self.session_id,
                    self.max_steps
                );

                // In Full approval mode, auto-extend the step budget
                if self.policy.approval_level == crate::workflow::react::policy::ApprovalLevel::Full
                {
                    log::info!(
                        "WorkflowExecutor {}: Auto-extending step budget in Full approval mode",
                        self.session_id
                    );
                    self.max_steps += DEFAULT_MAX_STEPS;
                    self.context
                        .add_message(
                            "user".to_string(),
                            format!(
                                "<SYSTEM_REMINDER>STEP BUDGET AUTO-EXTENDED: Full approval mode detected. \
                                Step budget has been increased by {}. Current step: {}/{}.</SYSTEM_REMINDER>",
                                DEFAULT_MAX_STEPS, self.current_step, self.max_steps
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
                } else {
                    // In other approval modes, enter confirmation waiting
                    // The unified waiting loop will handle Continue/Stop signals
                    log::info!(
                        "WorkflowExecutor {}: Step budget exhausted ({}/{}), entering confirmation waiting",
                        self.session_id,
                        self.current_step,
                        self.max_steps
                    );
                    self.update_state(WorkflowState::Paused).await?;
                    continue;
                }
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

            let agent_config = &self.agent_config;

            // Get model configs directly from AgentModels structure
            let act_model = agent_config.models.as_ref().and_then(|m| m.act.as_ref());
            let plan_model = agent_config.models.as_ref().and_then(|m| m.plan.as_ref());

            // Select the appropriate model based on phase and subagent type
            let selected_model = if self.policy.phase == ExecutionPhase::Planning {
                plan_model.or(act_model)
            } else {
                act_model
            };

            let _model_name = selected_model.map(|m| m.model.clone()).unwrap_or_default();
            let provider_id = selected_model.map(|m| m.id).unwrap_or(0);

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

            let available_tools = self
                .tool_manager
                .get_tool_calling_spec(None, None)
                .await
                .unwrap_or_default();

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
                    true, // require_tool_call: ReAct main loop requires tool calls
                )
                .await?;

            let invalid_tool_call_error = usage
                .as_ref()
                .and_then(|meta| meta.get("invalid_tool_call_error").and_then(|v| v.as_str()))
                .map(|s| s.to_string());

            let mut needs_compression = false;
            let mut assistant_metadata = Self::sanitize_assistant_metadata_for_storage(usage);
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

            // Stop has higher priority than persisting a new assistant tool-call turn.
            // This closes the race window where stop arrives right after LLM returns.
            if self.check_stop_signal(&mut signal_rx).await? {
                log::info!(
                    "[Workflow][session={}][phase=run_loop] Stop detected after LLM response; skipping assistant message commit",
                    self.session_id
                );
                break;
            }

            if !(full_response.trim().is_empty()
                && tool_calls_json.is_empty()
                && invalid_tool_call_error.is_some())
            {
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
            }

            self.update_state(WorkflowState::Executing).await?;
            let results_opt = match self
                .execute_tools(full_response.clone(), tool_calls_json, &mut signal_rx)
                .await
            {
                Ok(results) => Some(results),
                Err(WorkflowEngineError::Cancelled(msg)) => {
                    log::info!(
                        "WorkflowExecutor {}: User cancelled operation: {}",
                        self.session_id,
                        msg
                    );
                    self.update_state(WorkflowState::Cancelled).await?;
                    None
                }
                Err(e) => return Err(e),
            };

            let Some((results, has_todo)) = results_opt else {
                // Cancellation was handled, exit the loop
                break;
            };

            for (id, reinforced, original_call) in &results {
                if Self::is_postponed_turn_block_result(reinforced) {
                    log::info!(
                        "[Workflow][session={}][phase=observe] Suppressing postponed tool observation '{}'",
                        self.session_id,
                        reinforced.title
                    );
                    continue;
                }

                // Extract tool name from the original call for the metadata
                let tool_name = original_call
                    .get("name")
                    .or_else(|| original_call.get("function").and_then(|f| f.get("name")))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");

                // Build metadata with approval_status if present
                let mut execution_status = if reinforced.is_error {
                    "failed".to_string()
                } else if reinforced.approval_status.as_deref() == Some("pending") {
                    "pending_approval".to_string()
                } else if reinforced.approval_status.as_deref() == Some("rejected") {
                    "rejected".to_string()
                } else {
                    "completed".to_string()
                };

                let mut metadata = serde_json::json!({
                    "tool_call_id": id,
                    "tool_name": tool_name, // CRITICAL: Added for LlmProcessor's recovery logic
                    "title": reinforced.title,
                    "summary": reinforced.summary,
                    "execution_status": execution_status,
                    "is_error": reinforced.is_error,
                    "error_type": reinforced.error_type,
                    "display_type": reinforced.display_type
                });

                // Add approval_status if it exists in the reinforced result
                if let Some(approval_status) = &reinforced.approval_status {
                    metadata["approval_status"] = serde_json::json!(approval_status);
                }
                if let Some(observation_kind) = &reinforced.observation_kind {
                    metadata["observation_kind"] =
                        serde_json::to_value(observation_kind).unwrap_or(serde_json::Value::Null);
                }
                if let Some(observation_type) =
                    Self::runtime_observation_type_for_reinforced(reinforced)
                {
                    enrich_runtime_observation_metadata(
                        &mut metadata,
                        observation_type,
                        serde_json::json!({
                            "tool_call_id": id,
                            "tool_name": tool_name,
                            "title": reinforced.title,
                            "summary": reinforced.summary,
                            "is_error": reinforced.is_error,
                            "error_type": reinforced.error_type,
                            "llm_content": reinforced.content,
                        }),
                    );
                }
                Self::enrich_tool_observation_metadata(tool_name, &mut metadata, reinforced);
                if tool_name == crate::tools::TOOL_SUB_AGENT_RUN {
                    if let Ok(task_result) =
                        serde_json::from_str::<serde_json::Value>(&reinforced.content)
                    {
                        if let Some(task_id) = task_result.get("task_id").and_then(|v| v.as_str()) {
                            metadata["sub_agent_id"] = serde_json::json!(task_id);
                        }
                        if let Some(agent_name) =
                            task_result.get("agent_name").and_then(|v| v.as_str())
                        {
                            metadata["sub_agent_name"] = serde_json::json!(agent_name);
                        }
                        if let Some(task) = task_result.get("task").and_then(|v| v.as_str()) {
                            metadata["sub_agent_task"] = serde_json::json!(task);
                        }
                        if let Some(status) = task_result.get("status").and_then(|v| v.as_str()) {
                            metadata["sub_agent_status"] = serde_json::json!(status);
                            if status == "waiting" {
                                execution_status = "waiting".to_string();
                            }
                        }
                        if let Some(mode) = task_result.get("mode").and_then(|v| v.as_str()) {
                            metadata["sub_agent_mode"] = serde_json::json!(mode);
                            if mode == "call" && execution_status == "completed" {
                                execution_status = "waiting".to_string();
                            }
                        }
                    }
                    metadata["execution_status"] = serde_json::json!(execution_status);
                }

                let _ = self
                    .add_message_and_notify_internal(
                        "tool".to_string(),
                        reinforced.content.clone(),
                        None,
                        None,
                        Some(StepType::Observe),
                        reinforced.is_error,
                        reinforced.error_type.clone(),
                        Some(metadata),
                    )
                    .await?;
            }

            // Flush user messages queued during active execution after Observe stage completes.
            let queued_applied = self.flush_queued_user_messages().await?;
            if queued_applied {
                self.current_step = 0;
                self.consecutive_no_tool_calls = 0;
                self.loop_detector.reset_no_tool_response_history();
            }

            if self.state == WorkflowState::AwaitingSubAgent {
                log::info!(
                    "[Workflow][session={}][phase=run_loop] Child call is pending; entering wait branch",
                    self.session_id
                );
                sleep(Duration::from_millis(50)).await;
                continue;
            }

            if results.is_empty() {
                let repeated_no_tool_warning = self
                    .loop_detector
                    .record_no_tool_response_and_check(&full_response);
                self.consecutive_no_tool_calls += 1;
                log::warn!(
                    "WorkflowExecutor {}: No tool calls in response (consecutive: {})",
                    self.session_id,
                    self.consecutive_no_tool_calls
                );
                let mut runtime_observation_type = RuntimeObservationType::NoToolCall;
                let error_msg = if let Some(warning) = repeated_no_tool_warning {
                    runtime_observation_type = RuntimeObservationType::LoopDetected;
                    warning
                } else if let Some(invalid_tool_call_error) = invalid_tool_call_error {
                    runtime_observation_type = RuntimeObservationType::InvalidToolCall;
                    format!(
                        "<SYSTEM_REMINDER>Error: {}. You must call exactly one of the available tools exposed in this session. Re-read the tool list and try again with a valid tool name and valid arguments.</SYSTEM_REMINDER>",
                        invalid_tool_call_error
                    )
                } else if self.consecutive_no_tool_calls >= 3 {
                    let remaining = self.max_steps.saturating_sub(self.current_step);
                    let finish_tool = if self.is_child_agent_workflow() {
                        "submit_result"
                    } else {
                        "complete_workflow_with_summary"
                    };
                    format!(
                        "<SYSTEM_REMINDER>NO TOOL CALLS DETECTED: You have not called any tools for {} consecutive responses. \
                        This is wasting your limited step budget ({}/{} steps used, {} remaining). \
                        If you are truly finished, provide a summary and call '{}'.</SYSTEM_REMINDER>",
                        self.consecutive_no_tool_calls, self.current_step, self.max_steps, remaining, finish_tool
                    )
                } else {
                    if self.is_child_agent_workflow() {
                        "<SYSTEM_REMINDER>Error: No tool call detected in your last response. You MUST call a tool to proceed. If you have finished your delegated task, call 'submit_result' with both result and summary.</SYSTEM_REMINDER>".to_string()
                    } else {
                        "<SYSTEM_REMINDER>Error: No tool call detected in your last response. You MUST call a tool to proceed. If you have finished your task, call 'complete_workflow_with_summary' with a valid `summary` argument or after providing a visible summary in plain text.</SYSTEM_REMINDER>".to_string()
                    }
                };
                let _ = self
                    .add_message_and_notify_internal(
                        "user".to_string(),
                        error_msg.clone(),
                        None,
                        None,
                        Some(StepType::Observe),
                        false,
                        None,
                        Some(runtime_observation_metadata(
                            runtime_observation_type,
                            serde_json::json!({
                                "consecutive_no_tool_calls": self.consecutive_no_tool_calls,
                                "current_step": self.current_step,
                                "max_steps": self.max_steps,
                                "llm_content": error_msg,
                            }),
                        )),
                    )
                    .await?;
                if self.flush_queued_user_messages().await? {
                    self.current_step = 0;
                    self.consecutive_no_tool_calls = 0;
                    self.loop_detector.reset_no_tool_response_history();
                }
                if self.consecutive_no_tool_calls >= 3 {
                    self.consecutive_no_tool_calls = 0;
                }
                // Skip further processing since there are no tool results
                sleep(Duration::from_millis(100)).await;
                continue;
            } else {
                self.consecutive_no_tool_calls = 0;
                self.loop_detector.reset_no_tool_response_history();
            }

            let has_successful_finish_task =
                results.iter().any(|(_, reinforced, original_call)| {
                    if reinforced.is_error {
                        return false;
                    }
                    let tool_name = original_call
                        .get("name")
                        .or_else(|| original_call.get("function").and_then(|f| f.get("name")))
                        .and_then(|v| v.as_str())
                        .unwrap_or_default();
                    let is_terminal_tool = if self.is_child_agent_workflow() {
                        tool_name == TOOL_SUBMIT_RESULT
                    } else {
                        tool_name == TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY
                    };
                    is_terminal_tool
                        && reinforced.approval_status.as_deref() != Some("pending")
                        && reinforced.approval_status.as_deref() != Some("rejected")
                });

            if has_successful_finish_task {
                self.update_state(WorkflowState::Completed).await?;
                break;
            }

            if has_todo {
                self.sync_todo_list().await?;
            }

            // --- TRIGGER CONTEXT COMPRESSION ---
            // Cooldown: at least 3 steps since the last persisted compression to prevent churn.
            if self.current_step > self.last_compression_step + 3 {
                if needs_compression {
                    if let Some((compression_candidate, compressed_until_message_id)) =
                        self.context.build_blocking_compression_candidate()
                    {
                        if self.last_compression_boundary_id == Some(compressed_until_message_id) {
                            log::info!(
                                "[Workflow][session={}][phase=compression] Skip blocking compression because boundary {} was already compressed",
                                self.session_id,
                                compressed_until_message_id
                            );
                            continue;
                        }

                        self.dispatch_ui_payload(GatewayPayload::CompressionStatus {
                            is_compressing: true,
                            message: t!("workflow.compression_in_progress").to_string(),
                        })
                        .await?;

                        self.dispatch_ui_payload(GatewayPayload::Notification {
                            message: t!("workflow.compression_in_progress").to_string(),
                            category: Some("info".to_string()),
                        })
                        .await?;

                        let compression_result =
                            self.compressor.compress(&compression_candidate).await;

                        self.dispatch_ui_payload(GatewayPayload::CompressionStatus {
                            is_compressing: false,
                            message: String::new(),
                        })
                        .await?;

                        self.dispatch_ui_payload(GatewayPayload::Notification {
                            message: String::new(),
                            category: Some("info".to_string()),
                        })
                        .await?;

                        match compression_result {
                            Ok(summary) => {
                                if let Err(err) = self
                                    .persist_compression_summary(
                                        summary,
                                        compressed_until_message_id,
                                    )
                                    .await
                                {
                                    log::warn!(
                                        "[Workflow][session={}][phase=compression] Failed to persist compressed context: {}",
                                        self.session_id,
                                        err
                                    );
                                }
                            }
                            Err(err) => {
                                log::warn!(
                                    "[Workflow][session={}][phase=compression] Compression request failed: {}",
                                    self.session_id,
                                    err
                                );
                            }
                        }
                    } else {
                        log::info!(
                            "[Workflow][session={}][phase=compression] Skip blocking compression because no completed complete_workflow_with_summary segment is available",
                            self.session_id
                        );
                    }
                } else if self.background_compression_boundary_id.is_none() {
                    if let Some((compression_candidate, compressed_until_message_id)) =
                        self.context.build_rollup_compression_candidate()
                    {
                        if self.last_compression_boundary_id == Some(compressed_until_message_id) {
                            log::info!(
                                "[Workflow][session={}][phase=compression] Skip background rollup because boundary {} was already compressed",
                                self.session_id,
                                compressed_until_message_id
                            );
                        } else if !self
                            .should_retry_background_compression(compressed_until_message_id)
                        {
                            if let Some((failures, next_retry_step)) = self
                                .background_compression_retry_state
                                .get(&compressed_until_message_id)
                                .copied()
                            {
                                log::info!(
                                    "[Workflow][session={}][phase=compression] Backing off background rollup for boundary {} after {} failures; retry at step {} (current step {})",
                                    self.session_id,
                                    compressed_until_message_id,
                                    failures,
                                    next_retry_step,
                                    self.current_step
                                );
                            }
                        } else {
                            log::info!(
                                "[Workflow][session={}][phase=compression] Starting background rollup compression through boundary {}",
                                self.session_id,
                                compressed_until_message_id
                            );
                            self.spawn_background_compression(
                                compression_candidate,
                                compressed_until_message_id,
                            );
                        }
                    }
                }
            }

            sleep(Duration::from_millis(50)).await;
        }

        // === Memory Analysis After Workflow Completion ===
        // Run only for successful completion to avoid extra model calls after stop/cancel.
        if self.state == WorkflowState::Completed {
            log::info!(
                "[Workflow][session={}][phase=memory] Starting post-completion memory analysis",
                self.session_id
            );
            if let Err(e) = self.analyze_and_update_memories().await {
                log::warn!(
                    "[Workflow][session={}][phase=memory] Memory analysis failed: {}",
                    self.session_id,
                    e
                );
                // Don't fail the workflow if memory analysis fails
            } else {
                log::info!(
                    "[Workflow][session={}][phase=memory] Post-completion memory analysis finished",
                    self.session_id
                );
            }
        } else {
            log::info!(
                "[Workflow][session={}][phase=memory] Skip memory analysis for terminal state={}",
                self.session_id,
                self.state
            );
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
            log::info!(
                "[Workflow][session={}][phase=memory] Skip memory analysis because there are no user inputs",
                self.session_id
            );
            return Ok(());
        }

        log::info!(
            "[Workflow][session={}][phase=memory] Analyzing {} user input(s) for memory updates",
            self.session_id,
            user_inputs.len()
        );

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
        let mut updated_global = false;
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
                updated_global = true;
                log::info!(
                    "[Workflow][session={}][phase=memory] Global memory updated",
                    self.session_id
                );
            }
        }

        let mut updated_project = false;
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
                    updated_project = true;
                    log::info!(
                        "[Workflow][session={}][phase=memory] Project memory updated",
                        self.session_id
                    );
                }
            }
        }

        if !updated_global && !updated_project {
            log::info!(
                "[Workflow][session={}][phase=memory] Memory analysis completed with no memory changes",
                self.session_id
            );
        }

        Ok(())
    }

    fn normalize_tool_arguments_value(value: serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::String(raw) => {
                let cleaned = crate::libs::util::format_json_str(&raw);
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&cleaned) {
                    return parsed;
                }

                let start = cleaned
                    .char_indices()
                    .find(|(_, ch)| *ch == '{' || *ch == '[')
                    .map(|(idx, _)| idx);

                if let Some(start_idx) = start {
                    let candidate = &cleaned[start_idx..];
                    for (idx, ch) in candidate.char_indices().rev() {
                        if ch != '}' && ch != ']' {
                            continue;
                        }

                        let slice = &candidate[..=idx];
                        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(slice) {
                            return parsed;
                        }
                    }
                }

                serde_json::Value::String(raw)
            }
            other => other,
        }
    }

    fn enrich_tool_arguments_with_call_id(
        args: &serde_json::Value,
        tool_call_id: &str,
    ) -> serde_json::Value {
        let mut enriched_args = match args {
            serde_json::Value::Object(map) => serde_json::Value::Object(map.clone()),
            other => serde_json::json!({
                "__raw_arguments": other.clone()
            }),
        };
        enriched_args[crate::constants::INTERNAL_PARAM_TOOL_CALL_ID] =
            serde_json::json!(tool_call_id);
        enriched_args
    }

    fn find_completed_sub_agent_result_for_prompt(
        &self,
        prompt: &str,
    ) -> Option<(String, String, String)> {
        let prompt = prompt.trim();
        if prompt.is_empty() {
            return None;
        }

        let matching_task_ids: HashSet<String> = self
            .context
            .messages
            .iter()
            .filter_map(|message| {
                let metadata = message.metadata.as_ref()?;
                if metadata.get("tool_name").and_then(|value| value.as_str())
                    != Some(crate::tools::TOOL_SUB_AGENT_RUN)
                {
                    return None;
                }
                if metadata
                    .get("sub_agent_task")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    != Some(prompt)
                {
                    return None;
                }
                metadata
                    .get("sub_agent_id")
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
            })
            .collect();

        if matching_task_ids.is_empty() {
            return None;
        }

        self.context.messages.iter().rev().find_map(|message| {
            let metadata = message.metadata.as_ref()?;
            if metadata
                .get("observation_type")
                .and_then(|value| value.as_str())
                != Some("sub_agent_completion")
            {
                return None;
            }
            let sub_agent_id = metadata
                .get("sub_agent_id")
                .or_else(|| {
                    metadata
                        .get("data")
                        .and_then(|data| data.get("sub_agent_id"))
                })
                .and_then(|value| value.as_str())?;
            if !matching_task_ids.contains(sub_agent_id) {
                return None;
            }
            let result = metadata
                .get("result")
                .or_else(|| metadata.get("data").and_then(|data| data.get("result")))?;
            let status = result
                .get("status")
                .and_then(|value| value.as_str())
                .or_else(|| {
                    metadata
                        .get("execution_status")
                        .or_else(|| {
                            metadata
                                .get("data")
                                .and_then(|data| data.get("execution_status"))
                        })
                        .and_then(|value| value.as_str())
                })
                .unwrap_or("completed");
            let content = result
                .get("result")
                .and_then(|value| value.as_str())
                .or_else(|| result.get("summary").and_then(|value| value.as_str()))
                .or_else(|| result.get("error").and_then(|value| value.as_str()))
                .or_else(|| metadata.get("summary").and_then(|value| value.as_str()))?;
            Some((
                sub_agent_id.to_string(),
                status.to_string(),
                content.to_string(),
            ))
        })
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
        // P0-2: Guard against tool execution in safe-failed state
        if self.recovery_failed {
            log::error!(
                "[Workflow][session={}][phase=execute_tools] Blocked tool execution - session is in safe-failed recovery state",
                self.session_id
            );
            return Err(WorkflowEngineError::General(
                "Cannot execute tools: session is in safe-failed recovery state".to_string(),
            ));
        }

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
        let mut predicted_turn_block = false;

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
            let args = Self::normalize_tool_arguments_value(args_raw);

            call_order.push(id.clone());

            // --- CAUSAL BLOCKING CHECK ---
            // If a PREVIOUS tool in this TURN has already transitioned the engine to a blocking state
            // (like AwaitingApproval, AwaitingUser or Paused), we MUST NOT run or even audit subsequent tools.
            // They are simply postponed until the previous turn's block is resolved.
            if predicted_turn_block
                || self.state == WorkflowState::AwaitingApproval
                || self.state == WorkflowState::AwaitingUser
                || self.state == WorkflowState::Paused
                || self.state == WorkflowState::AwaitingSubAgent
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
                    approval_status: Some("rejected".to_string()),
                    observation_kind: Some(ObservationKind::TurnBlockedPostponed),
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
                    let blocks_following_tools =
                        matches!(name.as_str(), crate::tools::TOOL_SUB_AGENT_RUN)
                            && args
                                .get("execution_mode")
                                .and_then(|value| value.as_str())
                                .unwrap_or("call")
                                == "call"
                            || matches!(name.as_str(), crate::tools::TOOL_SUB_AGENT_OUTPUT)
                                && args
                                    .get("wait_until_complete")
                                    .and_then(|value| value.as_bool())
                                    .unwrap_or(false);

                    // Safe to proceed to physical execution!
                    if name.starts_with("todo_")
                        || matches!(
                            name.as_str(),
                            crate::tools::TOOL_SUB_AGENT_RUN
                                | crate::tools::TOOL_SUB_AGENT_OUTPUT
                                | crate::tools::TOOL_SUB_AGENT_STOP
                        )
                    {
                        has_todo_call = true;
                        sequential_execution_queue.push((id, name, args, call));
                    } else {
                        parallel_execution_queue.push((id, name, args, call));
                    }

                    if blocks_following_tools {
                        predicted_turn_block = true;
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

                // Inject internal tool_call_id for streaming tools
                let enriched_args = Self::enrich_tool_arguments_with_call_id(&args, &id);

                tool_futures.push_back(async move {
                    let _permit = semaphore_clone.acquire().await.ok();

                    let final_res = if name.contains(crate::tools::MCP_TOOL_NAME_SPLIT) {
                        // MCP tools are managed globally
                        gtm_clone.tool_call(&name, enriched_args).await
                    } else {
                        // Native tools are managed session-locally. No fallback.
                        tm_clone.tool_call(&name, enriched_args).await
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
            // Inject internal tool_call_id for streaming tools
            let enriched_args = Self::enrich_tool_arguments_with_call_id(&args, &id);

            let final_res = if name.contains(crate::tools::MCP_TOOL_NAME_SPLIT) {
                self.global_tool_manager
                    .tool_call(&name, enriched_args)
                    .await
            } else {
                self.tool_manager.tool_call(&name, enriched_args).await
            };

            let reinforced = self
                .post_process_tool_result(&name, &args, &call, final_res)
                .await?;
            result_map.insert(id, (reinforced, call));

            if self.state == WorkflowState::AwaitingSubAgent
                || self.state == WorkflowState::AwaitingApproval
                || self.state == WorkflowState::AwaitingUser
                || self.state == WorkflowState::Paused
            {
                break;
            }
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
        if name == crate::tools::TOOL_SUB_AGENT_RUN {
            if let Ok(val) = &result {
                if let Some(sub_agent_id) = Self::extract_call_mode_sub_agent_task_id(args, val) {
                    self.sub_agent_id = Some(sub_agent_id.clone());
                    if !self.sub_agent_sessions.iter().any(|id| id == &sub_agent_id) {
                        self.sub_agent_sessions.push(sub_agent_id.clone());
                    }
                    self.update_state(WorkflowState::AwaitingSubAgent).await?;
                }
            }
        }

        // 1. Complete Workflow Early Return
        if name == TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY {
            let default_summary = t!("workflow.task_finished").to_string();
            let summary = args
                .get("summary")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
                .unwrap_or(default_summary);
            return Ok(ReinforcedResult {
                content: "Finished".into(),
                title: "Complete Workflow with Summary".to_string(),
                summary,
                is_error: false,
                error_type: None,
                display_type: "text".to_string(),
                approval_status: None,
                observation_kind: None,
            });
        }

        if name == TOOL_SUBMIT_RESULT {
            return Ok(ReinforcedResult {
                content: args
                    .get("result")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string(),
                title: "Submit Result".to_string(),
                summary: args
                    .get("summary")
                    .and_then(|value| value.as_str())
                    .unwrap_or("Result submitted")
                    .to_string(),
                is_error: false,
                error_type: None,
                display_type: "text".to_string(),
                approval_status: None,
                observation_kind: None,
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
                                    approval_status: None,
                                    observation_kind: None,
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
            TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY => {
                return self.handle_finish_task_intercept(text_part, args).await
            }
            TOOL_SUBMIT_RESULT => return self.handle_submit_result_intercept(args).await,
            TOOL_ASK_USER => return self.handle_ask_user_intercept(args).await,
            _ => {}
        }

        // --- 2. Security & Runtime Checks (Bash, FS, Loops) ---
        // CRITICAL: These must happen BEFORE approval checks to ensure hard security boundaries
        // are never bypassed by user approval (especially in sensitive phases like Planning).
        if name == crate::tools::TOOL_SUB_AGENT_RUN
            && args
                .get("execution_mode")
                .and_then(|value| value.as_str())
                .unwrap_or("call")
                == "call"
        {
            if let Some(prompt) = args.get("prompt").and_then(|value| value.as_str()) {
                if let Some((sub_agent_id, status, content)) =
                    self.find_completed_sub_agent_result_for_prompt(prompt)
                {
                    return Ok(Some(ReinforcedResult {
                        content: format!(
                            "<tool_result tool=\"sub_agent_run\" id=\"{}\" mode=\"call\" status=\"{}\" reused=\"true\">\nTask:\n{}\n\nResult:\n{}\n</tool_result>\n<SYSTEM_REMINDER>An identical call-mode sub-agent task already completed in this workflow, so the previous result was reused instead of spawning another sub-agent. Use this result as context and continue the original user request.</SYSTEM_REMINDER>",
                            sub_agent_id, status, prompt, content
                        ),
                        title: "Sub Agent Run".to_string(),
                        summary: "Reused completed sub-agent result".to_string(),
                        is_error: false,
                        error_type: None,
                        display_type: "text".to_string(),
                        approval_status: None,
                        observation_kind: None,
                    }));
                }
            }
        }

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
                approval_status: None,
                observation_kind: None,
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

        // 2.4 Smart approval AI review
        if self.policy.approval_level == ApprovalLevel::Smart
            && !crate::tools::is_auto_execute_workflow_tool(name)
            && matches!(
                Self::smart_mode_approval_decision(name, args),
                crate::workflow::react::interceptors::SmartApprovalDecision::ReviewWithAi
            )
        {
            if let Some(review) = self
                .review_tool_call_for_smart_mode(name, args, text_part)
                .await?
            {
                if review.approved {
                    self.smart_approved_tool_call_ids.insert(id.to_string());
                    log::info!(
                        "WorkflowExecutor {}: AI approved tool '{}' in Smart mode (risk: {}, reason: {})",
                        self.session_id,
                        name,
                        review.risk_level,
                        review.reason
                    );
                    return Ok(None);
                }

                log::info!(
                    "WorkflowExecutor {}: AI did not auto-approve tool '{}' in Smart mode (risk: {}, reason: {})",
                    self.session_id,
                    name,
                    review.risk_level,
                    review.reason
                );
            }
        }

        // --- 3. Approval Policy Enforcement ---
        // Only if security checks pass do we consider asking the user for permission.
        if self.smart_approved_tool_call_ids.remove(id) {
            log::info!(
                "WorkflowExecutor {}: Skipping approval for Smart-AI-approved tool call '{}' ({})",
                self.session_id,
                id,
                name
            );
            return Ok(None);
        }

        if self.should_intercept_for_approval(name, args) {
            return self
                .handle_approval_interception(id, name, args, None)
                .await;
        }

        // Log auto-approval for better visibility
        if self.policy.approval_level == crate::workflow::react::policy::ApprovalLevel::Full
            || self.auto_approve.contains(name)
        {
            log::info!(
                "WorkflowExecutor {}: Auto-approved tool '{}' in {} mode",
                self.session_id,
                name,
                if self.policy.approval_level == crate::workflow::react::policy::ApprovalLevel::Full
                {
                    "Full"
                } else {
                    "Default (auto_approve list)"
                }
            );
        }

        Ok(None)
    }

    fn is_postponed_turn_block_result(reinforced: &ReinforcedResult) -> bool {
        reinforced.observation_kind == Some(ObservationKind::TurnBlockedPostponed)
    }

    fn runtime_observation_type_for_reinforced(
        reinforced: &ReinforcedResult,
    ) -> Option<RuntimeObservationType> {
        if reinforced.observation_kind == Some(ObservationKind::TurnBlockedPostponed) {
            return Some(RuntimeObservationType::TurnBlockedPostponed);
        }

        match reinforced.error_type.as_deref() {
            Some("InvalidFinishSummary") => Some(RuntimeObservationType::CompletionRejected),
            Some("PendingTodos") => Some(RuntimeObservationType::ActiveTodosBlocked),
            Some("AuditRejected") => Some(RuntimeObservationType::AuditRejected),
            Some("LoopDetected") => Some(RuntimeObservationType::LoopDetected),
            Some("InvalidToolCall") => Some(RuntimeObservationType::InvalidToolCall),
            Some("NoSummary") | Some("InvalidAskUserPayload") | Some("InvalidSubmitResult") => {
                Some(RuntimeObservationType::GenericReminder)
            }
            _ => None,
        }
    }

    async fn apply_sub_agent_completion(
        &mut self,
        sub_agent_id: String,
        result: serde_json::Value,
    ) -> Result<bool, WorkflowEngineError> {
        if let Some(resolution) = crate::workflow::react::child_tasks::resolve_sub_agent_completion(
            &mut self.sub_agent_id,
            &mut self.sub_agent_sessions,
            &sub_agent_id,
            &result,
        ) {
            for completion in &mut self.pending_sub_agent_completions {
                if completion.sub_agent_id == sub_agent_id {
                    completion.consumed = true;
                }
            }

            let summary_section = resolution
                .summary
                .as_deref()
                .filter(|summary| !summary.trim().is_empty() && *summary != resolution.content)
                .map(|summary| format!("\n\nSummary:\n{}", summary))
                .unwrap_or_default();
            let task_section = self
                .context
                .messages
                .iter()
                .rev()
                .filter_map(|message| message.metadata.as_ref())
                .find(|metadata| {
                    metadata
                        .get("sub_agent_id")
                        .and_then(|value| value.as_str())
                        == Some(resolution.sub_agent_id.as_str())
                })
                .and_then(|metadata| metadata.get("sub_agent_task"))
                .and_then(|value| value.as_str())
                .filter(|task| !task.trim().is_empty())
                .map(|task| format!("Task:\n{}\n\n", task))
                .unwrap_or_default();
            self.add_message_and_notify_internal(
                "user".to_string(),
                format!(
                    "<tool_result tool=\"sub_agent_run\" id=\"{}\" mode=\"call\" status=\"{}\">\n{}Result:\n{}\n{}\n</tool_result>\n<SYSTEM_REMINDER>The call-mode sub-agent result above has already been delivered. Treat it as context for the user's original request, then choose the appropriate next action: continue implementation, inspect more context, verify, ask a blocking question, answer the user, or finish only if the original request is fully addressed. Do not copy the sub-agent result verbatim as the final answer.</SYSTEM_REMINDER>",
                    resolution.sub_agent_id,
                    resolution.status,
                    task_section,
                    resolution.content,
                    summary_section
                ),
                None,
                None,
                Some(StepType::Observe),
                resolution.is_error,
                resolution.is_error.then(|| "SubAgentFailed".to_string()),
                Some({
                    let mut metadata = runtime_observation_metadata(
                        RuntimeObservationType::SubAgentCompletion,
                        json!({
                            "sub_agent_id": sub_agent_id.clone(),
                            "result": result.clone(),
                            "title": format!("Sub-agent {}", resolution.sub_agent_id),
                            "summary": resolution.content.clone(),
                            "execution_status": resolution.status.clone(),
                            "is_error": resolution.is_error,
                            "error_type": if resolution.is_error { "SubAgentFailed" } else { "" }
                        }),
                    );
                    metadata["sub_agent_id"] = json!(sub_agent_id.clone());
                    metadata["result"] = json!(result.clone());
                    metadata["title"] = json!(format!("Sub-agent {}", resolution.sub_agent_id));
                    metadata["summary"] = json!(resolution.content.clone());
                    metadata["execution_status"] = json!(resolution.status.clone());
                    metadata["is_error"] = json!(resolution.is_error);
                    metadata["error_type"] =
                        json!(if resolution.is_error { "SubAgentFailed" } else { "" });
                    metadata
                }),
            )
            .await?;

            self.update_state(WorkflowState::Thinking).await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub(crate) async fn update_state(
        &mut self,
        new_state: WorkflowState,
    ) -> Result<(), WorkflowEngineError> {
        let old_state = self.state.clone();

        // Log resume from waiting state
        let was_waiting = matches!(
            old_state,
            WorkflowState::Paused | WorkflowState::AwaitingUser | WorkflowState::AwaitingApproval
        );
        let now_running = matches!(
            new_state,
            WorkflowState::Thinking | WorkflowState::Executing
        );
        if was_waiting && now_running {
            log::info!(
                "[Workflow][session={}][phase=wait][event=resume] Resuming from {} to {}",
                self.session_id,
                old_state,
                new_state
            );
        }

        log::info!(
            "[Workflow][session={}][phase=state] State transition: {} -> {}",
            self.session_id,
            old_state,
            new_state
        );

        // Write StateChanged event if state actually changed
        if old_state != new_state {
            let event = WorkflowEvent::state_changed(
                self.session_id.clone(),
                old_state.to_string(),
                new_state.to_string(),
            );
            if let Err(e) = self.append_event(&event) {
                log::error!(
                    "[Workflow][session={}] workflow.event.append_failed - error={}",
                    self.session_id,
                    e
                );
            }
        }

        self.state = new_state.clone();

        // Cleanup pending approvals when transitioning away from approval-waiting states
        if matches!(
            new_state,
            WorkflowState::Thinking | WorkflowState::Executing | WorkflowState::Completed
        ) {
            let pending_count = self.pending_approvals.len();
            if pending_count > 0 {
                log::info!(
                    "[Workflow][session={}][phase=state] Clearing {} pending approvals on transition to {}",
                    self.session_id,
                    pending_count,
                    new_state
                );
            }
            self.pending_approvals.clear();
        }

        // Calculate wait_reason atomically with state
        let wait_reason = match &new_state {
            WorkflowState::Paused => Some(WaitReason::Confirmation),
            WorkflowState::AwaitingUser => Some(WaitReason::UserInput),
            WorkflowState::AwaitingApproval | WorkflowState::AwaitingAutoApproval => {
                Some(WaitReason::Approval)
            }
            WorkflowState::AwaitingSubAgent => Some(WaitReason::SubAgent),
            _ => None,
        };

        if old_state != new_state {
            // Write WaitEntered event only when entering waiting state.
            if wait_reason.is_some() {
                let pending_tools: Vec<serde_json::Value> = self
                    .pending_approvals
                    .iter()
                    .map(|entry| {
                        let info = entry.value();
                        serde_json::json!({
                            "tool_call_id": entry.key(),
                            "tool_name": info["name"].as_str().unwrap_or("unknown"),
                            "arguments": info.get("arguments").cloned().unwrap_or(serde_json::json!({})),
                            "details": info.get("details").and_then(|v| v.as_str()).unwrap_or_default(),
                            "display_type": info.get("display_type").and_then(|v| v.as_str()),
                        })
                    })
                    .collect();

                let event = WorkflowEvent::wait_entered(
                    self.session_id.clone(),
                    wait_reason.as_ref().unwrap().to_string(),
                    pending_tools,
                );
                if let Err(e) = self.append_event(&event) {
                    log::error!(
                        "[Workflow][session={}] workflow.event.append_failed - error={}",
                        self.session_id,
                        e
                    );
                }
            }

            // Write terminal events only on actual state transition.
            match &new_state {
                WorkflowState::Completed => {
                    let event = WorkflowEvent::workflow_completed(self.session_id.clone(), None);
                    self.dispatch_terminal_with_fallback("completed", &event)
                        .await;
                }
                WorkflowState::Cancelled => {
                    let event = WorkflowEvent::workflow_cancelled(self.session_id.clone());
                    self.dispatch_terminal_with_fallback("cancelled", &event)
                        .await;
                }
                WorkflowState::Error => {
                    let event = WorkflowEvent::workflow_failed(
                        self.session_id.clone(),
                        "Workflow encountered an error".to_string(),
                    );
                    self.dispatch_terminal_with_fallback("error", &event).await;
                }
                _ => {}
            }
        }

        self.dispatch_ui_payload(GatewayPayload::State {
            state: new_state.clone(),
            wait_reason,
        })
        .await?;
        self.dispatch_sub_agent_progress().await;

        {
            let store_res = self
                .context
                .main_store
                .write()
                .map_err(|e| WorkflowEngineError::General(e.to_string()));
            if let Ok(store) = store_res {
                let _ = store.update_workflow_status(&self.session_id, &new_state.to_string());
            }
        }

        // Save snapshot on waiting/terminal states
        if matches!(
            new_state,
            WorkflowState::Paused
                | WorkflowState::AwaitingUser
                | WorkflowState::AwaitingApproval
                | WorkflowState::AwaitingAutoApproval
                | WorkflowState::AwaitingSubAgent
                | WorkflowState::Completed
                | WorkflowState::Error
                | WorkflowState::Cancelled
        ) {
            if let Err(e) = self.save_snapshot().await {
                log::error!(
                    "[Workflow][session={}][phase=snapshot] Failed to save: {}",
                    self.session_id,
                    e
                );
            }
        }

        Ok(())
    }

    pub(crate) fn append_event(&self, event: &WorkflowEvent) -> Result<(), WorkflowEngineError> {
        if let Some(ref dispatcher) = self.dispatcher {
            if let Err(e) =
                dispatcher.dispatch_now(crate::workflow::react::dispatcher::DispatchEvent::Audit {
                    event: event.clone(),
                })
            {
                log::warn!(
                    "[Workflow][session={}][phase=dispatcher] audit dispatch failed: {}, falling back to direct DB write",
                    self.session_id,
                    e
                );
            } else {
                return Ok(());
            }
        }

        let store = self
            .context
            .main_store
            .read()
            .map_err(|e| WorkflowEngineError::General(e.to_string()))?;
        store
            .append_workflow_event(event)
            .map_err(|e| WorkflowEngineError::General(e.to_string()))?;
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
        // Cancellation is terminal for runtime output. Drop late assistant/tool writes
        // to avoid phantom last-turn messages after user clicks Stop.
        if self.state == WorkflowState::Cancelled && (role == "assistant" || role == "tool") {
            log::info!(
                "[Workflow][session={}][phase=message] Dropping late '{}' message because session is cancelled",
                self.session_id,
                role
            );
            return Ok(false);
        }

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

        // --- 2. Slash Command Auto-Activation (For User Messages) ---
        // We do this BEFORE adding the user message to ensure the skill instructions
        // appear immediately before the query that triggered them.
        if role == "user" {
            let _ = self.check_and_auto_activate_skills(&content).await;

            // [Bug Fix] If user sends a message while the session is Paused, AwaitingUser or AwaitingApproval,
            // automatically transition back to Thinking state so the loop can resume.
            // This is especially important for resumed sessions where the engine was restarted
            // in a paused state but given a new initial prompt.
            if self.state == WorkflowState::Paused
                || self.state == WorkflowState::AwaitingUser
                || self.state == WorkflowState::AwaitingApproval
            {
                log::info!("WorkflowExecutor {}: User message received while {:?}, transitioning to Thinking", self.session_id, self.state);
                self.update_state(WorkflowState::Thinking).await?;
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
                error_type.clone(),
                metadata.clone(),
            )
            .await?;

        self.dispatch_ui_payload(GatewayPayload::Message {
            role: role.clone(),
            content: content.clone(),
            reasoning,
            step_type,
            step_index: self.current_step as i32,
            is_error,
            error_type: error_type.clone(),
            metadata,
        })
        .await?;

        self.dispatch_context_usage().await?;
        self.dispatch_sub_agent_progress().await;

        // Summary messages should not trigger compression - they are the result of compression
        let is_summary = msg
            .metadata
            .as_ref()
            .map_or(false, |m| m["type"] == "summary");

        Ok(needs_compression && !is_summary)
    }

    /// Automatically detects and activates skills triggered by slash commands in user input.
    pub(crate) async fn check_and_auto_activate_skills(
        &mut self,
        content: &str,
    ) -> Result<(), WorkflowEngineError> {
        // Only check at the start of the message or after a newline
        if !content.starts_with('/') && !content.contains("\n/") {
            return Ok(());
        }

        // Regex to match slash commands: starts with / followed by alphanumeric chars/underscores/hyphens
        // We use a capture group for the skill name.
        let re = regex::Regex::new(r"(?m)^/([a-zA-Z0-9_-]+)").map_err(|e| {
            WorkflowEngineError::General(format!("Failed to compile slash command regex: {}", e))
        })?;

        for cap in re.captures_iter(content) {
            let skill_name = &cap[1];
            if let Some(skill) = self.available_skills.get(skill_name) {
                log::info!(
                    "WorkflowExecutor {}: Auto-activating skill '{}' from slash command",
                    self.session_id,
                    skill_name
                );

                let activated_content = format!(
                    "<activated_skill name=\"{}\" skill_dir=\"{}\">\n<instructions>\n{}\n</instructions>\n</activated_skill>\n<SYSTEM_REMINDER>\nSkill {} activated. You MUST strictly follow the expert guidance and workflows defined in the <instructions> above to fulfill the following user request. This context is current and complete; proceed immediately.\n</SYSTEM_REMINDER>",
                    &skill.name,
                    skill.skill_dir.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
                    skill.instructions,
                    &skill.name
                );

                // Add as a system message to the context so the LLM sees it immediately.
                // It is added AFTER the user message in history, but since we are about to call the LLM,
                // it will effectively act as the most recent instruction.
                self.context
                    .add_message(
                        "user".to_string(),
                        activated_content,
                        None,
                        None,
                        Some(StepType::Think),
                        self.current_step as i32,
                        false,
                        None,
                        None,
                    )
                    .await?;
            }
        }

        Ok(())
    }

    fn parse_allowed_paths_from_signal(&self, sig_json: &Value) -> Vec<PathBuf> {
        let mut unique_paths = HashSet::new();
        sig_json
            .get("paths")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str())
            .map(PathBuf::from)
            .filter_map(|p| {
                let abs_p = if p.is_absolute() {
                    p
                } else {
                    std::env::current_dir().unwrap_or_default().join(p)
                };
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
            .collect()
    }

    fn persist_workflow_agent_config_value(&self, key: &str, value: Value) {
        if let Ok(store) = self.context.main_store.write() {
            if let Ok(snapshot) = store.get_workflow_snapshot(&self.session_id) {
                let mut agent_config: serde_json::Value = snapshot
                    .workflow
                    .agent_config
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or(serde_json::json!({}));
                agent_config[key] = value;
                if let Ok(config_str) = serde_json::to_string(&agent_config) {
                    let _ = store.update_workflow_agent_config(&self.session_id, &config_str);
                }
            }
        }
    }

    async fn apply_runtime_approval_level_update(
        &mut self,
        sig_json: &Value,
        waiting: bool,
    ) -> Result<bool, WorkflowEngineError> {
        let Some((level_str, level)) = Self::parse_approval_level_from_signal(sig_json) else {
            return Ok(false);
        };

        if waiting {
            log::info!(
                "WorkflowExecutor {}: Updating approval level to {} while waiting",
                self.session_id,
                level_str
            );
        } else {
            log::info!(
                "WorkflowExecutor {}: Updating approval level to {}",
                self.session_id,
                level_str
            );
        }

        self.policy.approval_level = level.clone();
        self.persist_workflow_agent_config_value("approvalLevel", serde_json::json!(level_str));

        if waiting
            && self.state == WorkflowState::AwaitingApproval
            && level == ApprovalLevel::Full
            && !self.pending_approvals.is_empty()
        {
            log::info!(
                "WorkflowExecutor {}: Approval level switched to full while awaiting approval; resuming execution",
                self.session_id
            );
            self.update_state(WorkflowState::Thinking).await?;
        }

        Ok(true)
    }

    async fn handle_runtime_config_signal(
        &mut self,
        sig_json: &Value,
        sig_type_enum: Option<SignalType>,
    ) -> Result<bool, WorkflowEngineError> {
        if sig_type_enum == Some(SignalType::UpdateAllowedPaths) {
            let paths = self.parse_allowed_paths_from_signal(sig_json);
            log::info!(
                "WorkflowExecutor {}: Updating allowed paths to {:?}",
                self.session_id,
                paths
            );
            if let Ok(mut guard) = self.path_guard.write() {
                guard.update_allowed_roots(paths);
            }
            return Ok(true);
        }

        if sig_type_enum == Some(SignalType::UpdateModelConfig) {
            if let Some(configs) = sig_json.get("configs") {
                log::info!(
                    "WorkflowExecutor {}: Updating model configuration",
                    self.session_id
                );
                if let Ok(store) = self.context.main_store.write() {
                    if let Ok(snapshot) = store.get_workflow_snapshot(&self.session_id) {
                        let mut agent_config: serde_json::Value = snapshot
                            .workflow
                            .agent_config
                            .and_then(|s| serde_json::from_str(&s).ok())
                            .unwrap_or(serde_json::json!({}));
                        agent_config["models"] = configs.clone();
                        if let Ok(config_str) = serde_json::to_string(&agent_config) {
                            let _ =
                                store.update_workflow_agent_config(&self.session_id, &config_str);
                        }
                    }
                }
                if let Ok(models) =
                    serde_json::from_value::<crate::db::agent::AgentModels>(configs.clone())
                {
                    self.agent_config.models = Some(models);
                }
            }
            return Ok(true);
        }

        if sig_type_enum == Some(SignalType::UpdateApprovalLevel) {
            return self
                .apply_runtime_approval_level_update(sig_json, false)
                .await;
        }

        if sig_type_enum == Some(SignalType::UpdateFinalAudit) {
            let audit = sig_json
                .get("finalAudit")
                .and_then(|v| v.as_bool())
                .or_else(|| sig_json.get("audit").and_then(|v| v.as_bool()))
                .or_else(|| sig_json.get("final_audit").and_then(|v| v.as_bool()));
            if let Some(audit) = audit {
                log::info!(
                    "WorkflowExecutor {}: Updating final audit to {}",
                    self.session_id,
                    audit
                );
                self.agent_config.final_audit = Some(audit);
                if let Ok(store) = self.context.main_store.write() {
                    if let Ok(snapshot) = store.get_workflow_snapshot(&self.session_id) {
                        let mut agent_config: serde_json::Value = snapshot
                            .workflow
                            .agent_config
                            .and_then(|s| serde_json::from_str(&s).ok())
                            .unwrap_or(serde_json::json!({}));
                        agent_config["final_audit"] = serde_json::json!(audit);
                        if let Ok(config_str) = serde_json::to_string(&agent_config) {
                            let _ =
                                store.update_workflow_agent_config(&self.session_id, &config_str);
                        }
                    }
                }
            }
            return Ok(true);
        }

        Ok(false)
    }

    /// Checks if a stop signal is pending in the channel
    async fn check_stop_signal(
        &mut self,
        rx: &mut tokio::sync::mpsc::Receiver<String>,
    ) -> Result<bool, WorkflowEngineError> {
        while let Ok(s) = rx.try_recv() {
            let sig_json: Value = serde_json::from_str(&s).unwrap_or_default();
            let sig_type_str = sig_json["type"].as_str().unwrap_or_default();
            let sig_type_enum = SignalType::from_str(sig_type_str);
            match parse_runtime_signal(&s) {
                RuntimeSignal::Stop => {
                    log::info!(
                        "WorkflowExecutor {}: Stop signal detected, cancelling workflow",
                        self.session_id
                    );
                    self.update_state(WorkflowState::Cancelled).await?;
                    return Ok(true);
                }
                RuntimeSignal::UserMessage {
                    content,
                    queued_user_message_id,
                } => {
                    log::info!(
                        "[Workflow][session={}][phase=signal] Queueing user message during active execution",
                        self.session_id
                    );
                    self.enqueue_user_message(content, queued_user_message_id)
                        .await?;
                }
                RuntimeSignal::Other => {}
            }

            if self
                .handle_runtime_config_signal(&sig_json, sig_type_enum)
                .await?
            {
                continue;
            }

            if let Some(WorkflowSignal::CompressionReady {
                compressed_until_message_id,
                summary,
            }) = WorkflowSignal::parse(&s)
            {
                self.apply_background_compression_ready(compressed_until_message_id, summary)
                    .await?;
                continue;
            }

            if let Some(WorkflowSignal::CompressionFailed {
                compressed_until_message_id,
                error,
            }) = WorkflowSignal::parse(&s)
            {
                self.clear_background_compression_if_matches(compressed_until_message_id);
                self.record_background_compression_failure(compressed_until_message_id);
                let retry_state = self
                    .background_compression_retry_state
                    .get(&compressed_until_message_id)
                    .copied();
                log::warn!(
                    "[Workflow][session={}][phase=compression] Background rollup compression failed for boundary {}: {}. retry_state={:?}",
                    self.session_id,
                    compressed_until_message_id,
                    error,
                    retry_state
                );
                continue;
            }
        }

        // Also drain user messages stashed by temporary signal consumers (e.g. LLM retry backoff).
        for (queued_id, content) in take_stashed_user_messages(&self.session_id) {
            self.queued_user_messages.push_back((queued_id, content));
        }

        Ok(false)
    }

    async fn enqueue_user_message(
        &mut self,
        content: String,
        queued_user_message_id: Option<String>,
    ) -> Result<(), WorkflowEngineError> {
        let queued_id = queued_user_message_id.unwrap_or_else(|| {
            self.tsid_generator
                .generate()
                .unwrap_or_else(|_| format!("queued_{}", crate::ccproxy::get_tool_id()))
        });

        self.queued_user_messages
            .push_back((queued_id.clone(), content.clone()));

        // Immediate frontend ack: show user message instantly with queued status.
        self.dispatch_ui_payload(GatewayPayload::Message {
            role: "user".to_string(),
            content,
            reasoning: None,
            step_type: None,
            step_index: self.current_step as i32,
            is_error: false,
            error_type: None,
            metadata: Some(serde_json::json!({
                "queued_user_message_id": queued_id,
                "queue_status": "queued"
            })),
        })
        .await?;

        Ok(())
    }

    async fn flush_queued_user_messages(&mut self) -> Result<bool, WorkflowEngineError> {
        if self.queued_user_messages.is_empty() {
            return Ok(false);
        }

        if matches!(
            self.state,
            WorkflowState::Paused
                | WorkflowState::AwaitingUser
                | WorkflowState::AwaitingApproval
                | WorkflowState::AwaitingAutoApproval
                | WorkflowState::AwaitingSubAgent
        ) {
            log::debug!(
                "[Workflow][session={}][phase=queue] Skip flushing queued user messages while state={:?}",
                self.session_id,
                self.state
            );
            return Ok(false);
        }

        let mut applied = false;
        while let Some((queued_id, user_content)) = self.queued_user_messages.pop_front() {
            self.add_message_and_notify_internal(
                "user".to_string(),
                user_content.clone(),
                None,
                None,
                None,
                false,
                None,
                Some(serde_json::json!({
                    "queued_user_message_id": queued_id,
                    "queue_status": "applied"
                })),
            )
            .await?;
            let event = WorkflowEvent::user_input_received(self.session_id.clone(), user_content);
            if let Err(e) = self.append_event(&event) {
                log::error!(
                    "[Workflow][session={}] workflow.event.append_failed - error={}",
                    self.session_id,
                    e
                );
            }
            applied = true;
        }

        Ok(applied)
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
        self.dispatch_ui_payload(GatewayPayload::SyncTodo {
            todo_list: json!(todos),
        })
        .await?;
        Ok(())
    }

    /// Get list of auto-approved tools
    pub fn get_auto_approved_tools(&self) -> Vec<String> {
        self.auto_approve.iter().cloned().collect()
    }

    /// Remove a tool from auto-approve list
    pub fn remove_auto_approved_tool(&mut self, tool_name: &str) {
        self.auto_approve.remove(tool_name);
        log::info!(
            "WorkflowExecutor {}: Removed '{}' from auto-approve list",
            self.session_id,
            tool_name
        );
    }

    /// Remove an item from shell_policy and return the updated policy
    pub async fn remove_shell_policy_item(
        &mut self,
        pattern: &str,
    ) -> Option<Vec<crate::tools::ShellPolicyRule>> {
        // Update runtime agent_config.shell_policy
        if let Some(policy_str) = &self.agent_config.shell_policy {
            if let Ok(mut policy) =
                serde_json::from_str::<Vec<crate::tools::ShellPolicyRule>>(policy_str)
            {
                policy.retain(|item| item.pattern != pattern);
                self.agent_config.shell_policy =
                    Some(serde_json::to_string(&policy).unwrap_or_default());
                log::info!(
                    "WorkflowExecutor {}: Removed shell policy item with pattern '{}'",
                    self.session_id,
                    pattern
                );
                return Some(policy);
            }
        }
        None
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

    pub fn export_execution_context(&self) -> ExecutionContext {
        let state = RuntimeState::from(&self.state);

        let wait_reason = match &self.state {
            WorkflowState::Paused => Some(WaitReason::Confirmation),
            WorkflowState::AwaitingUser => Some(WaitReason::UserInput),
            WorkflowState::AwaitingApproval | WorkflowState::AwaitingAutoApproval => {
                Some(WaitReason::Approval)
            }
            WorkflowState::AwaitingSubAgent => Some(WaitReason::SubAgent),
            _ => None,
        };

        let pending_tools: Vec<PendingTool> = self
            .pending_approvals
            .iter()
            .map(|entry| {
                let value = entry.value();
                PendingTool {
                    tool_call_id: entry.key().clone(),
                    tool_name: value
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string(),
                    arguments: value.get("arguments").cloned().unwrap_or(json!({})),
                    details: value
                        .get("details")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    display_type: value
                        .get("display_type")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                }
            })
            .collect();

        ExecutionContext {
            session_id: self.session_id.clone(),
            state,
            wait_reason,
            current_step: self.current_step,
            max_steps: self.max_steps,
            pending_tools,
            last_action_summary: self
                .context
                .messages
                .last()
                .and_then(|m| m.metadata.as_ref())
                .and_then(|meta| meta.get("summary").and_then(|v| v.as_str()))
                .map(|s| s.to_string()),
            current_context_tokens: Some(self.context.current_token_estimate()),
            max_context_tokens: Some(self.context.max_tokens),
            last_event_id: None,
            version: ExecutionContext::CURRENT_VERSION.to_string(),
            waiting_on_sub_agent_id: self.sub_agent_id.clone(),
            sub_agent_sessions: self.sub_agent_sessions.clone(),
            pending_sub_agent_completions: self.pending_sub_agent_completions.clone(),
        }
    }

    pub async fn save_snapshot(&self) -> Result<(), WorkflowEngineError> {
        let mut ctx = self.export_execution_context();
        self.fill_snapshot_last_event_id(&mut ctx)?;

        if let Some(ref dispatcher) = self.dispatcher {
            if let Err(e) = dispatcher.dispatch_snapshot(ctx.clone()).await {
                log::warn!(
                    "[Workflow][session={}][phase=dispatcher] snapshot dispatch failed: {}, falling back to direct DB write",
                    self.session_id,
                    e
                );
                let store = self
                    .context
                    .main_store
                    .read()
                    .map_err(|e| WorkflowEngineError::General(e.to_string()))?;
                store
                    .upsert_execution_context(&ctx)
                    .map_err(|e| WorkflowEngineError::General(e.to_string()))?;
            }
        } else {
            let store = self
                .context
                .main_store
                .read()
                .map_err(|e| WorkflowEngineError::General(e.to_string()))?;
            store
                .upsert_execution_context(&ctx)
                .map_err(|e| WorkflowEngineError::General(e.to_string()))?;
        }

        log::info!(
            "[Workflow][session={}][phase=snapshot] Saved: state={:?}, wait_reason={:?}, pending_tools={}, last_event_id={:?}",
            self.session_id,
            ctx.state,
            ctx.wait_reason,
            ctx.pending_tools.len(),
            ctx.last_event_id
        );

        Ok(())
    }

    fn fill_snapshot_last_event_id(
        &self,
        ctx: &mut ExecutionContext,
    ) -> Result<(), WorkflowEngineError> {
        let store = self
            .context
            .main_store
            .read()
            .map_err(|e| WorkflowEngineError::General(e.to_string()))?;
        ctx.last_event_id = store
            .get_last_event_id(&self.session_id)
            .map_err(|e| WorkflowEngineError::General(e.to_string()))?;
        Ok(())
    }
}

impl Drop for WorkflowExecutor {
    fn drop(&mut self) {
        Dispatcher::unregister_session_dispatcher(&self.session_id);
    }
}

// ==================== Integration Tests for Recovery Flow ====================

#[cfg(test)]
mod recovery_tests {
    use super::*;
    use crate::db::MainStore;
    use crate::workflow::react::replay::{RecoveryError, RecoveryResult};
    use std::sync::Arc;
    use tempfile::tempdir;

    fn create_test_store() -> Arc<std::sync::RwLock<MainStore>> {
        let dir = tempdir().expect("failed to create temp dir");
        let db_path = dir.path().join("engine_recovery_test.db");
        let store = MainStore::new(db_path).expect("failed to create MainStore");
        Arc::new(std::sync::RwLock::new(store))
    }

    #[test]
    fn test_recovery_result_safe_failed_is_not_success() {
        let safe_failed = RecoveryResult::SafeFailed {
            session_id: "test".to_string(),
            error: RecoveryError::ReplayFailed {
                reason: "test failure".to_string(),
            },
        };
        assert!(!safe_failed.is_success());
    }

    #[test]
    fn test_recovery_result_snapshot_hit_is_success() {
        let ctx = ExecutionContext::new("test".to_string());
        let snapshot_hit = RecoveryResult::SnapshotHit { context: ctx };
        assert!(snapshot_hit.is_success());
    }

    #[test]
    fn test_recovery_result_replay_fallback_is_success() {
        let ctx = ExecutionContext::new("test".to_string());
        let replay_fallback = RecoveryResult::ReplayFallback { context: ctx };
        assert!(replay_fallback.is_success());
    }

    #[test]
    fn test_safe_failed_result_has_no_context() {
        let safe_failed = RecoveryResult::SafeFailed {
            session_id: "test".to_string(),
            error: RecoveryError::ReplayFailed {
                reason: "test failure".to_string(),
            },
        };
        assert!(safe_failed.context().is_none());
        assert!(safe_failed.into_context().is_none());
    }

    #[test]
    fn test_replay_failed_produces_safe_failed_result() {
        let store = create_test_store();
        let session_id = "replay-failed-test";

        {
            let s = store.read().unwrap();
            let conn = s.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO workflow_events (session_id, event_type, event_version, event_data, created_at)
                 VALUES (?1, ?2, ?3, ?4, datetime('now'))",
                rusqlite::params![
                    session_id,
                    "state_changed",
                    "1.0.0",
                    r#"{"from_state": "pending"}"#
                ],
            )
            .unwrap();
        }

        let result =
            crate::workflow::react::replay::restore_execution_context(store.clone(), session_id);

        match result {
            RecoveryResult::SafeFailed {
                session_id: sid,
                error,
            } => {
                assert_eq!(sid, session_id);
                match error {
                    RecoveryError::MissingEventData { .. } => {}
                    _ => panic!("Expected MissingEventData error"),
                }
            }
            _ => panic!("Expected SafeFailed, got {:?}", result),
        }
    }

    #[test]
    fn test_workflow_state_error_transitions_correctly() {
        let state = WorkflowState::Error;
        let runtime_state = RuntimeState::from(&state);
        assert_eq!(runtime_state, RuntimeState::Failed);
    }

    #[test]
    fn test_generate_shell_approval_patterns_ignores_cd_prefix() {
        let patterns = WorkflowExecutor::generate_shell_approval_patterns(
            "cd /a/b/c && cargo check --workspace",
        );
        assert_eq!(patterns, vec!["^cargo check($| .*)".to_string()]);
    }

    #[test]
    fn test_generate_shell_approval_patterns_ignores_redirection() {
        let patterns =
            WorkflowExecutor::generate_shell_approval_patterns("cat /dev/null>xxx && cargo check");
        assert_eq!(patterns, vec!["^cargo check($| .*)".to_string()]);
    }

    #[test]
    fn test_generate_shell_approval_patterns_ignores_inline_python() {
        let patterns = WorkflowExecutor::generate_shell_approval_patterns(
            "python -c \"print('hello')\" && cargo check",
        );
        assert_eq!(patterns, vec!["^cargo check($| .*)".to_string()]);
    }

    #[test]
    fn test_generate_shell_approval_patterns_extracts_multiple_safe_subcommands() {
        let patterns = WorkflowExecutor::generate_shell_approval_patterns(
            "cd /repo && cargo check --workspace && git status | head -20 && python -c \"print('x')\"",
        );
        assert_eq!(
            patterns,
            vec![
                "^cargo check($| .*)".to_string(),
                "^git status($| .*)".to_string()
            ]
        );
    }

    #[test]
    fn test_extract_call_mode_sub_agent_task_id_from_structured_content() {
        let args = serde_json::json!({ "execution_mode": "call" });
        let result = serde_json::json!({
            "content": "{\"status\":\"waiting\",\"task_id\":\"subagent_structured\"}",
            "structured_content": {
                "status": "waiting",
                "task_id": "subagent_structured"
            },
            "is_error": false
        });

        let task_id = WorkflowExecutor::extract_call_mode_sub_agent_task_id(&args, &result);
        assert_eq!(task_id.as_deref(), Some("subagent_structured"));
    }

    #[test]
    fn test_extract_call_mode_sub_agent_task_id_falls_back_to_content_payload() {
        let args = serde_json::json!({ "execution_mode": "call" });
        let result = serde_json::json!({
            "content": "{\"status\":\"waiting\",\"task_id\":\"subagent_content_only\"}",
            "is_error": false
        });

        let task_id = WorkflowExecutor::extract_call_mode_sub_agent_task_id(&args, &result);
        assert_eq!(task_id.as_deref(), Some("subagent_content_only"));
    }
}
