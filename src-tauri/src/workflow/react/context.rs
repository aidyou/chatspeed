use crate::db::{MainStore, WorkflowMessage};
use crate::libs::tsid::TsidGenerator;
use crate::workflow::react::error::WorkflowEngineError;
use crate::workflow::react::types::StepType;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Semaphore;

/// ContextManager is responsible for managing and persisting the conversation context for a ReAct session.
pub struct ContextManager {
    pub session_id: String,
    pub main_store: Arc<std::sync::RwLock<MainStore>>,
    pub max_tokens: usize,
    pub messages: Vec<WorkflowMessage>,
    pub tsid_generator: Arc<TsidGenerator>,
    /// Semaphore to control concurrent operations if needed
    pub semaphore: Arc<Semaphore>,
}

impl ContextManager {
    pub fn new(
        session_id: String,
        main_store: Arc<std::sync::RwLock<MainStore>>,
        max_tokens: usize,
        tsid_generator: Arc<TsidGenerator>,
    ) -> Self {
        Self {
            session_id,
            main_store,
            max_tokens,
            messages: Vec::new(),
            tsid_generator,
            semaphore: Arc::new(Semaphore::new(3)), // Restore to 3 concurrent calls
        }
    }

    /// Loads history from database, starting from the last summary if exists.
    pub async fn load_history(&mut self) -> Result<(), WorkflowEngineError> {
        let snapshot = {
            let store = self.main_store.read().map_err(|e| {
                WorkflowEngineError::Db(crate::db::error::StoreError::LockError(e.to_string()))
            })?;
            store.get_workflow_snapshot(&self.session_id)?
        };

        let last_summary_idx = snapshot
            .messages
            .iter()
            .enumerate()
            .rev()
            .find(|(_, m)| {
                m.metadata
                    .as_ref()
                    .map(|v| v["type"] == "summary")
                    .unwrap_or(false)
            })
            .map(|(i, _)| i);

        if let Some(idx) = last_summary_idx {
            log::info!(
                "ContextManager {}: Resuming from last summary at index {}",
                self.session_id,
                idx
            );

            // Start with the initial user query if it exists and is before the summary
            let initial_query = snapshot
                .messages
                .iter()
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
        attached_context: Option<String>,
        reasoning: Option<String>,
        step_type: Option<StepType>,
        step_index: i32,
        is_error: bool,
        error_type: Option<String>,
        metadata: Option<serde_json::Value>,
    ) -> Result<(WorkflowMessage, bool), WorkflowEngineError> {
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|e| WorkflowEngineError::General(e.to_string()))?;

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
            attached_context,
            step_type: step_type.map(|s| s.to_string()),
            step_index,
            is_error,
            error_type,
            created_at: None,
        };

        let persisted = {
            let store = self.main_store.read().map_err(|e| {
                WorkflowEngineError::Db(crate::db::error::StoreError::LockError(e.to_string()))
            })?;
            store.add_workflow_message(&msg)?
        };

        self.messages.push(persisted.clone());

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
            // Include attached_context in estimation
            let content_to_estimate = if let Some(att) = &m.attached_context {
                format!("{}\n\n{}", m.message, att)
            } else {
                m.message.clone()
            };
            total_tokens +=
                crate::ccproxy::utils::token_estimator::estimate_tokens(&content_to_estimate);
        }

        Ok((persisted, total_tokens > (self.max_tokens as f64 * 0.8)))
    }

    pub async fn compress(
        &mut self,
        summary: String,
        step_index: i32,
    ) -> Result<(), WorkflowEngineError> {
        // Keep the most recent N messages for continuity after compression.
        // Fixed count is more predictable than a percentage for long conversations.
        const KEEP_RECENT_MESSAGES: usize = 10;
        let total_msgs = self.messages.len();
        let keep_count = KEEP_RECENT_MESSAGES.min(total_msgs).max(5);
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
            attached_context: None,
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
        let mut llm_messages = self.messages.clone();

        for msg in llm_messages.iter_mut() {
            if let Some(att) = &msg.attached_context {
                if !att.is_empty() {
                    msg.message = format!("{}\n\n{}", msg.message, att);
                }
            }
            // 1. Wrap the initial user query
            if msg.role == "user" && !msg.message.starts_with("<user_query>") {
                msg.message = format!("<user_query>\n{}\n</user_query>", msg.message);
            }

            // 2. Wrap Approved Plan components if metadata exists
            if let Some(meta) = &msg.metadata {
                if meta["subtype"] == "approved_plan" {
                    let plan = meta["plan_content"].as_str().unwrap_or("");
                    let todos = meta["todo_content"].as_str().unwrap_or("[]");

                    msg.message = format!(
                        "# APPROVED EXECUTION PLAN\n<approved_plan>\n{}\n</approved_plan>\n<current_todo_list>\n{}\n</current_todo_list>",
                        plan,
                        todos
                    );
                }
            }
        }

        llm_messages
    }

    /// Gets the initial user query for the session.
    pub fn get_initial_query(&self) -> String {
        self.messages
            .iter()
            .find(|m| m.role == "user")
            .map(|m| m.message.clone())
            .unwrap_or_default()
    }

    /// Prunes the context for transitioning from Planning to Execution.
    /// It keeps the initial user query and adds the final approved plan as the new anchor.
    pub async fn prune_for_execution(
        &mut self,
        approved_plan: String,
    ) -> Result<(), WorkflowEngineError> {
        log::info!(
            "ContextManager {}: Pruning context for execution phase",
            self.session_id
        );

        let initial_query = {
            let store = self.main_store.read().map_err(|e| {
                WorkflowEngineError::Db(crate::db::error::StoreError::LockError(e.to_string()))
            })?;
            let snapshot = store.get_workflow_snapshot(&self.session_id)?;
            snapshot.messages.iter().find(|m| m.role == "user").cloned()
        };

        // 1. Physical Database Pruning: Delete all intermediate steps but keep initial query ID
        {
            let store = self.main_store.write().map_err(|e| {
                WorkflowEngineError::Db(crate::db::error::StoreError::LockError(e.to_string()))
            })?;
            let keep_ids = initial_query
                .as_ref()
                .and_then(|m| m.id)
                .map(|id| vec![id])
                .unwrap_or_default();
            store.delete_workflow_messages(&self.session_id, keep_ids)?;
        }

        // 2. Clear current in-memory messages
        self.messages.clear();

        // 3. Re-add initial query to memory if found
        if let Some(query) = initial_query {
            self.messages.push(query);
        }

        // 3. Fetch current todo items to inject into context
        let todo_json = {
            let store = self.main_store.read().map_err(|e| {
                WorkflowEngineError::Db(crate::db::error::StoreError::LockError(e.to_string()))
            })?;
            let todos = store.get_todo_list_for_workflow(&self.session_id)?;
            serde_json::to_string_pretty(&todos).unwrap_or_else(|_| "[]".to_string())
        };

        // 4. Add the approved plan as a specialized summary message
        let msg_id = self
            .tsid_generator
            .generate_u64()
            .map_err(|e| WorkflowEngineError::General(e))?;
        let plan_msg = WorkflowMessage {
            id: Some(msg_id as i64),
            session_id: self.session_id.clone(),
            role: "system".to_string(),
            message: format!(
                "# APPROVED EXECUTION PLAN\n\n## PLAN\n{}\n\n## TODO LIST\n{}",
                approved_plan, todo_json
            ),
            reasoning: None,
            metadata: Some(json!({
                "type": "summary",
                "subtype": "approved_plan",
                "plan_content": approved_plan,
                "todo_content": todo_json
            })),
            attached_context: None,
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
