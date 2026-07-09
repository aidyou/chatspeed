use crate::ai::interaction::chat_completion::ChatState;
use crate::db::{Agent, MainStore};
use crate::libs::tsid::TsidGenerator;
use crate::tools::ToolManager;
use crate::workflow::react::engine::WorkflowExecutor;
use crate::workflow::react::error::WorkflowEngineError;
use crate::workflow::react::gateway::Gateway;
use crate::workflow::react::orchestrator::SubAgentFactory;
use std::path::PathBuf;
use std::sync::Arc;

use crate::workflow::react::engine::ReActExecutor;
use crate::workflow::react::policy::ExecutionPolicy;
use crate::workflow::react::types::StepType;
use async_trait::async_trait;

/// ExecutionExecutor focuses on implementation and task completion.
/// It has full tool access to perform requested changes.
pub struct ExecutionExecutor {
    executor: WorkflowExecutor,
}

#[async_trait]
impl ReActExecutor for ExecutionExecutor {
    async fn init(&mut self) -> Result<(), WorkflowEngineError> {
        self.executor.init_internal().await
    }

    async fn run_loop(&mut self) -> Result<(), WorkflowEngineError> {
        self.executor.run_loop_internal().await
    }

    async fn begin_new_context_segment(&mut self) -> Result<(), WorkflowEngineError> {
        self.executor.begin_new_context_segment().await
    }

    async fn begin_manual_clear_context_segment(&mut self) -> Result<(), WorkflowEngineError> {
        self.executor.begin_manual_clear_context_segment().await
    }

    async fn prepare_completed_resume(&mut self) -> Result<(), WorkflowEngineError> {
        self.executor.prepare_completed_resume().await
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
        self.executor
            .add_message_and_notify_internal(
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
        self.executor.session_id.clone()
    }

    fn state(&self) -> crate::workflow::react::types::WorkflowState {
        self.executor.state.clone()
    }

    fn set_state(&mut self, state: crate::workflow::react::types::WorkflowState) {
        self.executor.state = state;
    }

    fn attach_signal_rx(&mut self, signal_rx: tokio::sync::mpsc::Receiver<String>) {
        self.executor.signal_rx = Some(signal_rx);
    }

    fn messages(&self) -> Vec<crate::db::WorkflowMessage> {
        self.executor.context.messages.clone()
    }
}

impl ExecutionExecutor {
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
        tsid_generator: Arc<TsidGenerator>,
        global_tool_manager: Arc<ToolManager>,
        auto_compress_enabled: bool,
        policy: ExecutionPolicy,
    ) -> Self {
        let executor = WorkflowExecutor::new(
            session_id,
            main_store,
            chat_state,
            gateway,
            sub_agent_factory,
            agent_config,
            allowed_paths,
            app_data_dir,
            subagent_type,
            signal_rx,
            tsid_generator,
            global_tool_manager,
            auto_compress_enabled,
            policy,
        );
        Self { executor }
    }
}
