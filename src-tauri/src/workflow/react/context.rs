use crate::db::MainStore;
use crate::db::WorkflowMessage;
use crate::workflow::react::error::WorkflowEngineError;
use crate::workflow::react::types::StepType;
use serde_json::json;
use std::sync::Arc;

pub struct ContextManager {
    pub session_id: String,
    pub messages: Vec<WorkflowMessage>,
    pub main_store: Arc<std::sync::RwLock<MainStore>>,
    pub max_tokens: usize,
    pub tsid_generator: Arc<crate::libs::tsid::TsidGenerator>,
    pub semaphore: Arc<tokio::sync::Semaphore>,
}

impl ContextManager {
    pub fn new(
        session_id: String,
        main_store: Arc<std::sync::RwLock<MainStore>>,
        max_tokens: usize,
        tsid_generator: Arc<crate::libs::tsid::TsidGenerator>,
    ) -> Self {
        Self {
            session_id,
            messages: Vec::new(),
            main_store,
            max_tokens,
            tsid_generator,
            semaphore: Arc::new(tokio::sync::Semaphore::new(3)),
        }
    }

    /// Loads history from database, starting from the last summary if exists.
    pub async fn load_history(&mut self) -> Result<(), WorkflowEngineError> {
        let store = self.main_store.read().map_err(|e| {
            WorkflowEngineError::Db(crate::db::error::StoreError::LockError(e.to_string()))
        })?;
        let snapshot = store.get_workflow_snapshot(&self.session_id)?;

        // Find the index of the last summary message
        let last_summary_idx = snapshot.messages.iter().rposition(|m| {
            m.role == "system"
                && m.metadata
                    .as_ref()
                    .map_or(false, |meta| meta["type"] == "summary")
        });

        if let Some(idx) = last_summary_idx {
            log::info!(
                "ContextManager {}: Resuming from last summary at index {}",
                self.session_id,
                idx
            );
            
            // Start with the initial user query if it exists and is before the summary
            let initial_query = snapshot.messages.iter()
                .take(idx)
                .find(|m| m.role == "user")
                .cloned();

            let mut msgs = Vec::new();
            if let Some(q) = initial_query {
                msgs.push(q);
            }
            msgs.extend(snapshot.messages[idx..].to_vec());
            self.messages = msgs;
        } else {
            self.messages = snapshot.messages;
        }

        Ok(())
    }

    /// Adds a new message and persists it. Returns true if compression is needed.
    pub async fn add_message(
        &mut self,
        role: String,
        content: String,
        reasoning: Option<String>,
        step_type: Option<StepType>,
        step_index: i32,
        is_error: bool,
        error_type: Option<String>,
        metadata: Option<serde_json::Value>,
    ) -> Result<bool, WorkflowEngineError> {
        let msg_id = self
            .tsid_generator
            .generate_u64()
            .map_err(|e| WorkflowEngineError::General(e))?;
        let msg = WorkflowMessage {
            id: Some(msg_id as i64),
            session_id: self.session_id.clone(),
            role: role.clone(),
            message: content,
            reasoning,
            metadata,
            step_type: step_type.map(|t| t.to_string()),
            step_index,
            is_error,
            error_type,
            created_at: None,
        };

        #[cfg(debug_assertions)]
        if role == "assistant" {
            dbg!(&msg);
        }

        let persisted_msg = {
            let store = self.main_store.read().map_err(|e| {
                WorkflowEngineError::Db(crate::db::error::StoreError::LockError(e.to_string()))
            })?;
            store.add_workflow_message(&msg)?
        };

        self.messages.push(persisted_msg);

        // Check if compression is needed (80% threshold)
        // Use actual usage if available in metadata, else estimate
        let mut total_tokens = 0.0;
        for m in &self.messages {
            if let Some(meta) = &m.metadata {
                if let Some(usage) = meta.get("usage") {
                    if let Some(tokens) = usage.get("total_tokens").and_then(|v| v.as_f64()) {
                        total_tokens += tokens;
                        continue;
                    }
                }
            }
            total_tokens += crate::ccproxy::utils::token_estimator::estimate_tokens(&m.message);
        }

        Ok(total_tokens > (self.max_tokens as f64 * 0.8))
    }

    /// Adds a summary message to mark a compression point
    pub async fn add_summary(
        &mut self,
        summary: String,
        step_index: i32,
    ) -> Result<(), WorkflowEngineError> {
        // Keep the most recent N messages for continuity after compression.
        // Fixed count is more predictable than a percentage for long conversations.
        const KEEP_RECENT_MESSAGES: usize = 10;
        let total_msgs = self.messages.len();
        let keep_count = KEEP_RECENT_MESSAGES.min(total_msgs).max(5); // Always keep at least 5
        let split_idx = total_msgs.saturating_sub(keep_count);

        // We only keep messages AFTER split_idx
        self.messages = self.messages[split_idx..].to_vec();

        // Prepend the new summary
        let msg_id = self
            .tsid_generator
            .generate_u64()
            .map_err(|e| WorkflowEngineError::General(e))?;
        let summary_msg = WorkflowMessage {
            id: Some(msg_id as i64),
            session_id: self.session_id.clone(),
            role: "system".to_string(),
            message: format!("## Previous Context Snapshot\n{}", summary),
            reasoning: None,
            metadata: Some(json!({ "type": "summary" })),
            step_type: None,
            step_index,
            is_error: false,
            error_type: None,
            created_at: None,
        };

        // Persist summary
        let persisted_summary = {
            let store = self.main_store.read().map_err(|e| {
                WorkflowEngineError::Db(crate::db::error::StoreError::LockError(e.to_string()))
            })?;
            store.add_workflow_message(&summary_msg)?
        };

        self.messages.insert(0, persisted_summary);

        Ok(())
    }

    pub fn get_messages_for_llm(&self) -> Vec<WorkflowMessage> {
        self.messages.clone()
    }

    /// Prunes the context for transitioning from Planning to Execution.
    /// It keeps the initial user query and adds the final approved plan as the new anchor.
    pub async fn prune_for_execution(&mut self, approved_plan: String) -> Result<(), WorkflowEngineError> {
        log::info!("ContextManager {}: Pruning context for execution phase", self.session_id);

        let initial_query = {
            let store = self.main_store.read().map_err(|e| {
                WorkflowEngineError::Db(crate::db::error::StoreError::LockError(e.to_string()))
            })?;
            let snapshot = store.get_workflow_snapshot(&self.session_id)?;
            snapshot.messages.iter()
                .find(|m| m.role == "user")
                .cloned()
        };

        // 1. Clear current messages
        self.messages.clear();

        // 2. Re-add initial query if found
        if let Some(query) = initial_query {
            self.messages.push(query);
        }

        // 3. Add the approved plan as a specialized summary message
        let msg_id = self.tsid_generator.generate_u64().map_err(|e| WorkflowEngineError::General(e))?;
        let plan_msg = WorkflowMessage {
            id: Some(msg_id as i64),
            session_id: self.session_id.clone(),
            role: "system".to_string(),
            message: format!("# APPROVED EXECUTION PLAN\n\n{}", approved_plan),
            reasoning: None,
            metadata: Some(json!({ "type": "summary", "subtype": "approved_plan" })),
            step_type: None,
            step_index: 0,
            is_error: false,
            error_type: None,
            created_at: None,
        };

        let persisted_plan = {
            let store = self.main_store.read().map_err(|e| {
                WorkflowEngineError::Db(crate::db::error::StoreError::LockError(e.to_string()))
            })?;
            store.add_workflow_message(&plan_msg)?
        };

        self.messages.push(persisted_plan);

        Ok(())
    }
}
