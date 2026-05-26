use crate::db::{MainStore, WorkflowAiContextMessage, WorkflowMessage};
use crate::libs::tsid::TsidGenerator;
use crate::tools::TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY;
use crate::workflow::react::error::WorkflowEngineError;
use crate::workflow::react::runtime_observation::{
    is_runtime_observation, RuntimeObservationLlmVisibility,
};
use crate::workflow::react::types::StepType;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::Semaphore;

const CONTEXT_PRESSURE_COMPRESSION_THRESHOLD: f64 = 0.75;

/// ContextManager manages durable transcript history plus the AI-only context projection cache.
pub struct ContextManager {
    pub session_id: String,
    pub main_store: Arc<std::sync::RwLock<MainStore>>,
    pub max_tokens: usize,
    pub messages: Vec<WorkflowMessage>,
    pub ai_context_messages: Vec<WorkflowAiContextMessage>,
    pub current_segment_id: i32,
    pub tsid_generator: Arc<TsidGenerator>,
    /// Semaphore to control concurrent operations if needed
    pub semaphore: Arc<Semaphore>,
}

impl ContextManager {
    pub(crate) fn sanitize_reasoning_content(reasoning: Option<String>) -> Option<String> {
        let reasoning = reasoning?;
        let trimmed = reasoning.trim();
        if trimmed.is_empty() {
            return None;
        }

        let sanitized = trimmed
            .trim_start_matches("<think>")
            .trim_start_matches("<THINK>")
            .trim_start_matches("<thought>")
            .trim_start_matches("<THOUGHT>")
            .trim_end_matches("</think>")
            .trim_end_matches("</THINK>")
            .trim_end_matches("</thought>")
            .trim_end_matches("</THOUGHT>")
            .trim()
            .to_string();

        if sanitized.is_empty() {
            None
        } else {
            Some(sanitized)
        }
    }

    fn is_summary_message(message: &WorkflowMessage) -> bool {
        message.message_kind == "summary"
    }

    pub(crate) fn is_compression_summary_message(message: &WorkflowMessage) -> bool {
        let Some(meta) = message.metadata.as_ref() else {
            return false;
        };

        message.message_kind == "summary"
            && message.message_subtype.as_deref() == Some("compression")
            && meta.get("compressed_until_message_id").is_some()
    }

    fn is_successful_completion_message(message: &WorkflowMessage) -> bool {
        if message.role != "tool" || message.is_error {
            return false;
        }

        let Some(meta) = message.metadata.as_ref() else {
            return false;
        };

        let is_completion_tool = meta
            .get("tool_name")
            .and_then(|v| v.as_str())
            .map(|tool_name| tool_name == TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY)
            .unwrap_or(false);
        if !is_completion_tool {
            return false;
        }

        let execution_status = meta
            .get("execution_status")
            .and_then(|v| v.as_str())
            .unwrap_or("completed");
        let approval_status = meta
            .get("approval_status")
            .and_then(|v| v.as_str())
            .unwrap_or("approved");

        execution_status == "completed"
            && approval_status != "pending"
            && approval_status != "rejected"
    }

    fn latest_summary_message(messages: &[WorkflowMessage]) -> Option<&WorkflowMessage> {
        messages
            .iter()
            .rev()
            .find(|m| Self::is_compression_summary_message(m))
    }

    fn latest_summary_boundary_id(messages: &[WorkflowMessage]) -> Option<i64> {
        Self::latest_summary_message(messages).and_then(|message| {
            message
                .metadata
                .as_ref()
                .and_then(|meta| meta.get("compressed_until_message_id"))
                .and_then(|v| v.as_i64())
        })
    }

    fn latest_successful_completion_index(messages: &[WorkflowMessage]) -> Option<usize> {
        messages
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, message)| {
                if Self::is_successful_completion_message(message) {
                    Some(index)
                } else {
                    None
                }
            })
    }

    fn latest_summary_index(messages: &[WorkflowMessage]) -> Option<usize> {
        messages
            .iter()
            .rposition(Self::is_compression_summary_message)
    }

    fn estimate_context_message_tokens(message: &WorkflowAiContextMessage) -> f64 {
        let mut total = crate::ccproxy::utils::token_estimator::estimate_tokens(&message.message);

        if let Some(reasoning) = message.reasoning.as_deref() {
            total += crate::ccproxy::utils::token_estimator::estimate_tokens(reasoning);
        }

        total
    }

    fn rebuild_compacted_messages(
        messages: &[WorkflowMessage],
        max_tokens: usize,
    ) -> Vec<WorkflowMessage> {
        let latest_summary = match Self::latest_summary_message(messages).cloned() {
            Some(summary) => summary,
            None => {
                let _ = max_tokens;
                return messages.to_vec();
            }
        };

        let Some(compressed_until_message_id) = Self::latest_summary_boundary_id(messages) else {
            return messages.to_vec();
        };

        let mut compacted = vec![latest_summary];
        compacted.extend(
            messages
                .iter()
                .filter(|message| !Self::is_summary_message(message))
                .filter(|message| {
                    message
                        .id
                        .is_some_and(|id| id > compressed_until_message_id)
                })
                .cloned(),
        );
        compacted
    }

    pub fn current_token_estimate(&self) -> usize {
        self.ai_context_messages
            .iter()
            .map(Self::estimate_context_message_tokens)
            .sum::<f64>()
            .round() as usize
    }

    fn build_compression_candidate_preserving_latest_completion(
        &self,
        minimum_completions_after_summary: usize,
    ) -> Option<(Vec<WorkflowMessage>, i64)> {
        let compressed_until_message_id =
            Self::latest_summary_boundary_id(&self.messages).unwrap_or(0);

        let completion_indices: Vec<usize> = self
            .messages
            .iter()
            .enumerate()
            .filter(|(_, message)| {
                Self::is_successful_completion_message(message)
                    && message
                        .id
                        .is_some_and(|id| id > compressed_until_message_id)
            })
            .map(|(idx, _)| idx)
            .collect();

        if completion_indices.len() < minimum_completions_after_summary {
            return None;
        }

        let target_completion_idx =
            *completion_indices.get(completion_indices.len().saturating_sub(2))?;
        let latest_completion_idx = *completion_indices.last()?;

        if latest_completion_idx <= target_completion_idx {
            return None;
        }

        if self
            .messages
            .len()
            .saturating_sub(latest_completion_idx + 1)
            == 0
        {
            return None;
        }

        let compressed_until_id = self.messages[target_completion_idx].id?;

        let start_idx = Self::latest_summary_index(&self.messages).unwrap_or(0);

        if start_idx > target_completion_idx {
            return None;
        }

        Some((
            self.messages[start_idx..=target_completion_idx].to_vec(),
            compressed_until_id,
        ))
    }

    pub fn build_pressure_compression_candidate(&self) -> Option<(Vec<WorkflowMessage>, i64)> {
        self.build_compression_candidate_preserving_latest_completion(2)
    }

    pub fn build_task_boundary_compression_candidate(&self) -> Option<(Vec<WorkflowMessage>, i64)> {
        self.build_compression_candidate_preserving_latest_completion(3)
    }

    #[allow(dead_code)]
    pub fn build_blocking_compression_candidate(&self) -> Option<(Vec<WorkflowMessage>, i64)> {
        self.build_pressure_compression_candidate()
    }

    #[allow(dead_code)]
    pub fn build_rollup_compression_candidate(&self) -> Option<(Vec<WorkflowMessage>, i64)> {
        self.build_task_boundary_compression_candidate()
    }

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
            ai_context_messages: Vec::new(),
            current_segment_id: 1,
            tsid_generator,
            semaphore: Arc::new(Semaphore::new(3)), // Restore to 3 concurrent calls
        }
    }

    /// Loads history from database, starting from the last summary if exists.
    pub async fn load_history(&mut self) -> Result<(), WorkflowEngineError> {
        let (snapshot, execution_context) = {
            let store = self.main_store.read().map_err(|e| {
                WorkflowEngineError::Db(crate::db::error::StoreError::LockError(e.to_string()))
            })?;
            (
                store.get_workflow_snapshot(&self.session_id)?,
                store.get_execution_context(&self.session_id)?,
            )
        };
        self.messages = Self::rebuild_compacted_messages(&snapshot.messages, self.max_tokens);
        let transcript_segment_id = self
            .messages
            .iter()
            .map(|message| message.segment_id)
            .max()
            .unwrap_or(1);
        let recovery_segment_id = execution_context
            .as_ref()
            .map(|ctx| ctx.current_segment_id)
            .unwrap_or(1);
        self.current_segment_id = transcript_segment_id.max(recovery_segment_id).max(1);
        self.rebuild_context_projection_from_runtime_messages(false, true)
            .await?;

        Ok(())
    }

    fn merge_attached_context(content: &str, attached_context: Option<&str>) -> String {
        match attached_context {
            Some(attached) if !attached.is_empty() => format!("{}\n\n{}", content, attached),
            _ => content.to_string(),
        }
    }

    fn content_for_context_projection(message: &WorkflowMessage) -> String {
        if message.role == "tool" {
            if let Some(llm_content) = message
                .metadata
                .as_ref()
                .and_then(|meta| meta.get("llm_content"))
                .and_then(|value| value.as_str())
            {
                return llm_content.to_string();
            }
        }

        Self::merge_attached_context(&message.message, message.attached_context.as_deref())
    }

    fn llm_visibility(
        metadata: Option<&serde_json::Value>,
    ) -> Option<RuntimeObservationLlmVisibility> {
        metadata
            .and_then(|meta| meta.get("llm_visibility"))
            .and_then(|value| serde_json::from_value(value.clone()).ok())
    }

    fn latest_tool_message_index(source_messages: &[WorkflowMessage]) -> HashMap<String, usize> {
        source_messages
            .iter()
            .enumerate()
            .filter_map(|(idx, message)| {
                let tool_call_id = message
                    .metadata
                    .as_ref()
                    .and_then(|meta| meta.get("tool_call_id").and_then(|value| value.as_str()))?;
                (message.role == "tool").then_some((tool_call_id.to_string(), idx))
            })
            .collect()
    }

    fn should_project_message_for_llm(
        message: &WorkflowMessage,
        idx: usize,
        latest_tool_message_index: &HashMap<String, usize>,
    ) -> bool {
        if message.role == "system" {
            return true;
        }

        if is_runtime_observation(message.metadata.as_ref())
            && Self::llm_visibility(message.metadata.as_ref())
                == Some(RuntimeObservationLlmVisibility::Hide)
        {
            return false;
        }

        if message.role != "tool" {
            return true;
        }

        let Some(tool_call_id) = message
            .metadata
            .as_ref()
            .and_then(|meta| meta.get("tool_call_id").and_then(|value| value.as_str()))
        else {
            return true;
        };

        latest_tool_message_index
            .get(tool_call_id)
            .is_none_or(|latest_idx| *latest_idx == idx)
    }

    fn tool_call_ids_from_message(message: &WorkflowAiContextMessage) -> HashSet<String> {
        message
            .metadata
            .as_ref()
            .and_then(|meta| meta.get("tool_calls"))
            .and_then(|tool_calls| tool_calls.as_array())
            .map(|tool_calls| {
                tool_calls
                    .iter()
                    .filter_map(|tool_call| {
                        tool_call
                            .get("id")
                            .and_then(|value| value.as_str())
                            .or_else(|| {
                                tool_call
                                    .get("tool_call_id")
                                    .and_then(|value| value.as_str())
                            })
                            .map(str::to_string)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn protocol_safe_context_projection(
        projected: Vec<WorkflowAiContextMessage>,
    ) -> Vec<WorkflowAiContextMessage> {
        let mut protocol_safe = Vec::with_capacity(projected.len());
        let mut pending_tool_call_ids: Option<HashSet<String>> = None;
        let mut pending_assistant_index: Option<usize> = None;

        fn strip_unresolved_tool_calls(
            projected: &mut Vec<WorkflowAiContextMessage>,
            assistant_index: usize,
        ) {
            let Some(message) = projected.get_mut(assistant_index) else {
                return;
            };

            if let Some(meta) = message
                .metadata
                .as_mut()
                .and_then(|value| value.as_object_mut())
            {
                meta.remove("tool_calls");
            }

            let has_tool_calls = message
                .metadata
                .as_ref()
                .and_then(|meta| meta.get("tool_calls"))
                .and_then(|value| value.as_array())
                .is_some_and(|calls| !calls.is_empty());
            let has_content = !message.message.trim().is_empty();
            let has_reasoning = message
                .reasoning
                .as_ref()
                .is_some_and(|reasoning| !reasoning.trim().is_empty());

            if !has_tool_calls && !has_content && !has_reasoning {
                projected.remove(assistant_index);
            }
        }

        for message in projected {
            match message.role.as_str() {
                "assistant" => {
                    if pending_tool_call_ids.take().is_some() {
                        if let Some(assistant_index) = pending_assistant_index.take() {
                            strip_unresolved_tool_calls(&mut protocol_safe, assistant_index);
                        }
                    }

                    let tool_call_ids = Self::tool_call_ids_from_message(&message);
                    pending_assistant_index = None;
                    pending_tool_call_ids = (!tool_call_ids.is_empty()).then(|| {
                        pending_assistant_index = Some(protocol_safe.len());
                        tool_call_ids
                    });
                    protocol_safe.push(message);
                }
                "tool" => {
                    let Some(tool_call_id) = message
                        .metadata
                        .as_ref()
                        .and_then(|meta| meta.get("tool_call_id").and_then(|value| value.as_str()))
                        .map(str::to_string)
                    else {
                        continue;
                    };

                    let Some(open_tool_calls) = pending_tool_call_ids.as_mut() else {
                        continue;
                    };

                    if !open_tool_calls.remove(&tool_call_id) {
                        continue;
                    }

                    protocol_safe.push(message);

                    if open_tool_calls.is_empty() {
                        pending_tool_call_ids = None;
                        pending_assistant_index = None;
                    }
                }
                _ => {
                    if pending_tool_call_ids.take().is_some() {
                        if let Some(assistant_index) = pending_assistant_index.take() {
                            strip_unresolved_tool_calls(&mut protocol_safe, assistant_index);
                        }
                    }

                    pending_tool_call_ids = None;
                    protocol_safe.push(message);
                }
            }
        }

        if pending_tool_call_ids.take().is_some() {
            if let Some(assistant_index) = pending_assistant_index.take() {
                strip_unresolved_tool_calls(&mut protocol_safe, assistant_index);
            }
        }

        protocol_safe
    }

    fn projection_source_messages(&self) -> Vec<WorkflowMessage> {
        let Some(summary_idx) = Self::latest_summary_index(&self.messages) else {
            return self.messages.clone();
        };

        let mut projection = vec![self.messages[summary_idx].clone()];
        let tail_start = summary_idx + 1;
        if tail_start >= self.messages.len() {
            return projection;
        }

        let completion_indices_after_summary: Vec<usize> = self
            .messages
            .iter()
            .enumerate()
            .skip(tail_start)
            .filter_map(|(idx, message)| {
                Self::is_successful_completion_message(message).then_some(idx)
            })
            .collect();

        let retained_start = completion_indices_after_summary
            .iter()
            .rev()
            .nth(1)
            .map(|index| index + 1)
            .unwrap_or(tail_start);

        projection.extend(self.messages[retained_start..].iter().cloned());
        projection
    }

    fn build_context_projection(
        &self,
        source_messages: &[WorkflowMessage],
        segment_id: i32,
    ) -> Vec<WorkflowAiContextMessage> {
        let mut projected = Vec::new();
        let latest_tool_message_index = Self::latest_tool_message_index(source_messages);

        for (idx, message) in source_messages.iter().enumerate() {
            if !Self::should_project_message_for_llm(message, idx, &latest_tool_message_index) {
                continue;
            }

            let step_type = message
                .step_type
                .as_deref()
                .and_then(|value| value.parse().ok());

            let merged_content = Self::content_for_context_projection(message);
            let final_content = if message.message_subtype.as_deref() == Some("approved_plan") {
                let plan = message
                    .metadata
                    .as_ref()
                    .and_then(|meta| meta.get("plan_content"))
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                let todos = message
                    .metadata
                    .as_ref()
                    .and_then(|meta| meta.get("todo_content"))
                    .and_then(|value| value.as_str())
                    .unwrap_or("[]");
                format!(
                    "# APPROVED EXECUTION PLAN\n<approved_plan>\n{}\n</approved_plan>\n<current_todo_list>\n{}\n</current_todo_list>",
                    plan, todos
                )
            } else if message.role == "user"
                && step_type.as_ref() != Some(&StepType::Observe)
                && !projected
                    .iter()
                    .any(|existing: &WorkflowAiContextMessage| existing.role == "user")
            {
                format!("<user_query>\n{}\n</user_query>", merged_content)
            } else {
                merged_content
            };

            projected.push(WorkflowAiContextMessage {
                id: None,
                session_id: self.session_id.clone(),
                segment_id,
                role: message.role.clone(),
                message: final_content,
                reasoning: message.reasoning.clone(),
                message_kind: message.message_kind.clone(),
                message_subtype: message.message_subtype.clone(),
                metadata: message.metadata.clone(),
                source_message_id: message.id,
                created_at: None,
            });
        }

        Self::protocol_safe_context_projection(projected)
    }

    async fn rebuild_context_projection_from_runtime_messages(
        &mut self,
        increment_segment: bool,
        persist: bool,
    ) -> Result<(), WorkflowEngineError> {
        if increment_segment {
            self.current_segment_id += 1;
        }

        let segment_id = self.current_segment_id;
        let projection =
            self.build_context_projection(&self.projection_source_messages(), segment_id);
        self.ai_context_messages.clear();

        if persist {
            let store = self.main_store.read().map_err(|e| {
                WorkflowEngineError::Db(crate::db::error::StoreError::LockError(e.to_string()))
            })?;
            store.delete_workflow_ai_context_segment(&self.session_id, segment_id)?;
            for message in projection {
                let persisted = store.add_workflow_ai_context_message(&message)?;
                self.ai_context_messages.push(persisted);
            }
        } else {
            self.ai_context_messages = projection;
        }

        Ok(())
    }

    pub async fn begin_new_task_segment_from_runtime_projection(
        &mut self,
    ) -> Result<(), WorkflowEngineError> {
        self.rebuild_context_projection_from_runtime_messages(true, true)
            .await
    }

    fn to_workflow_messages(messages: &[WorkflowAiContextMessage]) -> Vec<WorkflowMessage> {
        messages
            .iter()
            .map(|message| WorkflowMessage {
                id: message.id,
                session_id: message.session_id.clone(),
                role: message.role.clone(),
                message: message.message.clone(),
                reasoning: message.reasoning.clone(),
                message_kind: message.message_kind.clone(),
                message_subtype: message.message_subtype.clone(),
                segment_id: message.segment_id,
                source_event_type: None,
                metadata: message.metadata.clone(),
                attached_context: None,
                step_type: None,
                step_index: 0,
                is_error: false,
                error_type: None,
                created_at: message.created_at.clone(),
            })
            .collect()
    }

    async fn persist_context_seed(
        &self,
        seed: WorkflowAiContextMessage,
    ) -> Result<WorkflowAiContextMessage, WorkflowEngineError> {
        let store = self.main_store.read().map_err(|e| {
            WorkflowEngineError::Db(crate::db::error::StoreError::LockError(e.to_string()))
        })?;
        store
            .add_workflow_ai_context_message(&seed)
            .map_err(WorkflowEngineError::from)
    }

    pub async fn begin_new_segment_with_seed(
        &mut self,
        seed_messages: Vec<WorkflowAiContextMessage>,
    ) -> Result<(), WorkflowEngineError> {
        self.current_segment_id += 1;
        self.ai_context_messages.clear();

        for mut seed in seed_messages {
            seed.segment_id = self.current_segment_id;
            seed.session_id = self.session_id.clone();
            let persisted = self.persist_context_seed(seed).await?;
            self.ai_context_messages.push(persisted);
        }

        Ok(())
    }

    pub async fn begin_execution_segment_from_approved_plan(
        &mut self,
    ) -> Result<(), WorkflowEngineError> {
        let initial_user = self
            .ai_context_messages
            .iter()
            .find(|message| message.role == "user")
            .cloned()
            .ok_or_else(|| {
                WorkflowEngineError::General(
                    "Cannot start execution segment without a current user query".to_string(),
                )
            })?;

        let approved_plan = self
            .ai_context_messages
            .iter()
            .rev()
            .find(|message| message.message_subtype.as_deref() == Some("approved_plan"))
            .cloned()
            .ok_or_else(|| {
                WorkflowEngineError::General(
                    "Cannot start execution segment without an approved plan anchor".to_string(),
                )
            })?;

        self.begin_new_segment_with_seed(vec![initial_user, approved_plan])
            .await
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
        let should_start_new_segment = role == "user"
            && step_type != Some(StepType::Observe)
            && self
                .messages_since_last_completion()
                .into_iter()
                .all(|message| {
                    !(message.role == "user"
                        && message.step_type.as_deref() != Some("observe")
                        && !message.message.trim().is_empty())
                });
        if should_start_new_segment {
            self.rebuild_context_projection_from_runtime_messages(true, true)
                .await?;
        }

        let persisted = {
            let _permit = self
                .semaphore
                .acquire()
                .await
                .map_err(|e| WorkflowEngineError::General(e.to_string()))?;

            let msg_id = self
                .tsid_generator
                .generate_u64()
                .map_err(|e| WorkflowEngineError::General(e))?;
            let reasoning = Self::sanitize_reasoning_content(reasoning);
            let msg = WorkflowMessage {
                id: Some(msg_id as i64),
                session_id: self.session_id.clone(),
                role: role.clone(),
                message: content,
                reasoning,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: self.current_segment_id,
                source_event_type: None,
                metadata,
                attached_context,
                step_type: step_type.clone().map(|s| s.to_string()),
                step_index,
                is_error,
                error_type,
                created_at: None,
            };

            let store = self.main_store.read().map_err(|e| {
                WorkflowEngineError::Db(crate::db::error::StoreError::LockError(e.to_string()))
            })?;
            store.add_workflow_message(&msg)?
        };

        self.messages.push(persisted.clone());

        self.rebuild_context_projection_from_runtime_messages(false, true)
            .await?;

        // Check if compression is needed.
        // This estimate is based on projected runtime context only and does not fully include
        // the later injected system prompt layers, so keep a conservative safety margin.
        // Estimate from the currently retained context only.
        // Do not sum per-request usage.total_tokens across messages because those values
        // already include the full prompt history for each request and would double-count.
        let total_tokens = self.current_token_estimate() as f64;

        Ok((
            persisted,
            total_tokens > (self.max_tokens as f64 * CONTEXT_PRESSURE_COMPRESSION_THRESHOLD),
        ))
    }

    pub async fn compress(
        &mut self,
        summary: String,
        step_index: i32,
        compressed_until_message_id: i64,
    ) -> Result<(), WorkflowEngineError> {
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
            message_kind: "summary".to_string(),
            message_subtype: Some("compression".to_string()),
            segment_id: self.current_segment_id,
            source_event_type: None,
            metadata: Some(json!({
                "type": "summary",
                "compressed_until_message_id": compressed_until_message_id
            })),
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

        let mut all_messages = self.messages.clone();
        all_messages.push(persisted_summary.clone());
        self.messages = Self::rebuild_compacted_messages(&all_messages, self.max_tokens);

        self.rebuild_context_projection_from_runtime_messages(true, true)
            .await?;

        Ok(())
    }

    pub fn get_messages_for_llm(&self) -> Vec<WorkflowMessage> {
        Self::to_workflow_messages(&self.ai_context_messages)
    }

    /// Gets the initial user query for the session.
    pub fn get_initial_query(&self) -> String {
        self.messages
            .iter()
            .find(|m| m.role == "user")
            .map(|m| m.message.clone())
            .unwrap_or_default()
    }

    /// Returns all messages that belong to the current work segment after the last successful complete_workflow_with_summary.
    pub fn messages_since_last_completion(&self) -> Vec<WorkflowMessage> {
        let start_index = Self::latest_successful_completion_index(&self.messages)
            .map(|index| index + 1)
            .unwrap_or(0);
        self.messages.iter().skip(start_index).cloned().collect()
    }

    /// Returns the first user request after the last successful complete_workflow_with_summary.
    /// Falls back to the initial query if the current segment has no user message yet.
    pub fn current_user_request_since_last_completion(&self) -> String {
        self.messages_since_last_completion()
            .into_iter()
            .find(|message| {
                message.role == "user" && message.step_type.as_deref() != Some("observe")
            })
            .map(|message| message.message)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| self.get_initial_query())
    }
}

#[cfg(test)]
mod tests {
    use super::ContextManager;
    use crate::ccproxy::utils::token_estimator::estimate_tokens;
    use crate::db::WorkflowAiContextMessage;
    use crate::db::{Agent, MainStore, WorkflowMessage};
    use crate::libs::tsid::TsidGenerator;
    use crate::tools::TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY;
    use crate::workflow::react::constants::TASK_FINISHED;
    use crate::workflow::react::types::{ExecutionContext, StepType};
    use serde_json::json;
    use std::sync::{Arc, RwLock};

    fn setup_store() -> (tempfile::TempDir, Arc<RwLock<MainStore>>) {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let db_path = dir.path().join("context_compression_test.db");
        let store = MainStore::new(&db_path).expect("failed to create MainStore");
        (dir, Arc::new(RwLock::new(store)))
    }

    fn insert_workflow(store: &Arc<RwLock<MainStore>>, session_id: &str) {
        let agent = Agent::new(
            "test-agent".to_string(),
            "Compression Test Agent".to_string(),
            None,
            Some("primary".to_string()),
            None,
            "You are a test agent.".to_string(),
            None,        // planning_prompt
            None,        // image_recognition_prompt
            None,        // available_tools
            None,        // auto_approve
            None,        // models
            None,        // shell_policy
            None,        // allowed_paths
            Some(false), // final_audit
            None,        // approval_level
            Some(false), // skill_enabled
            None,        // selected_skills
            None,        // phase
            Some(false), // is_system
            Some(false), // disabled
            Some(4096),  // max_contexts
        );

        let store_guard = store.write().expect("lock poisoned");
        store_guard
            .add_agent(&agent)
            .expect("failed to create agent");
        store_guard
            .create_workflow(session_id, "Initial query", "test-agent", None, None)
            .expect("failed to create workflow");
    }

    #[test]
    fn current_token_estimate_includes_reasoning_content() {
        let message = WorkflowAiContextMessage {
            id: None,
            session_id: "session".to_string(),
            segment_id: 0,
            role: "assistant".to_string(),
            message: "Visible answer".to_string(),
            reasoning: Some("Hidden reasoning".to_string()),
            message_kind: "message".to_string(),
            message_subtype: None,
            metadata: None,
            source_message_id: None,
            created_at: None,
        };

        let expected = estimate_tokens("Visible answer") + estimate_tokens("Hidden reasoning");
        let actual = ContextManager::estimate_context_message_tokens(&message);

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn compress_persists_summary_and_rebuilds_context_without_pruning_db_history() {
        let (_dir, store) = setup_store();
        let session_id = "session-compress-test";
        insert_workflow(&store, session_id);

        let tsid_generator = Arc::new(TsidGenerator::new(1).expect("failed to create tsid"));
        let mut context =
            ContextManager::new(session_id.to_string(), store.clone(), 4096, tsid_generator);

        let _ = context
            .add_message(
                "user".to_string(),
                "Initial query".to_string(),
                None,
                None,
                None,
                0,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add user message");

        for index in 0..16 {
            let _ = context
                .add_message(
                    if index % 2 == 0 {
                        "assistant".to_string()
                    } else {
                        "tool".to_string()
                    },
                    format!("message-{index} {}", "x".repeat(3000)),
                    None,
                    None,
                    None,
                    index + 1,
                    false,
                    None,
                    Some(json!({ "tool_call_id": format!("tool-{index}") })),
                )
                .await
                .expect("failed to add context message");
        }

        let completion_message = context
            .add_message(
                "tool".to_string(),
                TASK_FINISHED.to_owned(),
                None,
                None,
                None,
                17,
                false,
                None,
                Some(json!({ "tool_name": TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY })),
            )
            .await
            .expect("failed to add completion message")
            .0;
        let _ = context
            .add_message(
                "user".to_string(),
                "Follow-up task".to_string(),
                None,
                None,
                None,
                18,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add follow-up task");
        let _ = context
            .add_message(
                "tool".to_string(),
                TASK_FINISHED.to_string(),
                None,
                None,
                None,
                19,
                false,
                None,
                Some(json!({ "tool_name": TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY })),
            )
            .await
            .expect("failed to add follow-up completion");
        let _ = context
            .add_message(
                "assistant".to_string(),
                "Starting next task".to_string(),
                None,
                None,
                None,
                20,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add active task marker");

        let (candidate_messages, compressed_until_id) = context
            .build_blocking_compression_candidate()
            .expect("complete_workflow_with_summary should create a compressible segment");

        context
            .compress(
                "<state_snapshot>compressed</state_snapshot>".to_string(),
                99,
                compressed_until_id,
            )
            .await
            .expect("compression should persist");

        let snapshot = store
            .read()
            .expect("lock poisoned")
            .get_workflow_snapshot(session_id)
            .expect("failed to load snapshot");

        let summary_messages = snapshot
            .messages
            .iter()
            .filter(|m| m.message_kind == "summary")
            .count();

        assert_eq!(
            summary_messages, 1,
            "summary should be persisted exactly once"
        );
        assert!(
            snapshot.messages.len() >= 18,
            "compression should preserve the full database history"
        );
        assert!(
            snapshot
                .messages
                .iter()
                .any(|m| m.role == "user" && m.message == "Initial query"),
            "initial user query should remain in the stored history"
        );
        assert!(
            context.messages.iter().any(|m| m.message_kind == "summary"),
            "the in-memory context should include the persisted summary"
        );
        assert!(
            context.messages.len() < snapshot.messages.len(),
            "the runtime context should be compacted even when the database keeps full history"
        );
        assert_eq!(
            candidate_messages.last().and_then(|m| m.id),
            completion_message.id,
            "compression boundary should stop at the earliest compressible completion and keep the latest completed task in memory"
        );
        assert_eq!(Some(compressed_until_id), completion_message.id);
    }

    #[tokio::test]
    async fn load_history_without_summary_keeps_full_history() {
        let (_dir, store) = setup_store();
        let session_id = "session-no-summary-test";
        insert_workflow(&store, session_id);

        let tsid_generator = Arc::new(TsidGenerator::new(1).expect("failed to create tsid"));
        let mut context =
            ContextManager::new(session_id.to_string(), store.clone(), 4096, tsid_generator);

        let _ = context
            .add_message(
                "user".to_string(),
                "Initial query".to_string(),
                None,
                None,
                None,
                0,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add initial user message");

        let _ = context
            .add_message(
                "assistant".to_string(),
                "Assistant reply".to_string(),
                None,
                None,
                None,
                1,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add assistant message");

        let _ = context
            .add_message(
                "user".to_string(),
                "Second user question".to_string(),
                None,
                None,
                None,
                2,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add second user message");

        let mut restored = ContextManager::new(
            session_id.to_string(),
            store.clone(),
            4096,
            Arc::new(TsidGenerator::new(2).expect("failed to create tsid")),
        );
        restored
            .load_history()
            .await
            .expect("load history should succeed");

        let snapshot = store
            .read()
            .expect("lock poisoned")
            .get_workflow_snapshot(session_id)
            .expect("failed to load snapshot");

        assert_eq!(
            restored.messages.len(),
            snapshot.messages.len(),
            "history without summary should remain uncompressed"
        );
    }

    #[tokio::test]
    async fn load_history_recovers_segment_id_from_snapshot_and_rebuilds_ai_cache() {
        let (_dir, store) = setup_store();
        let session_id = "session-segment-recovery-test";
        insert_workflow(&store, session_id);

        let tsid_generator = Arc::new(TsidGenerator::new(1).expect("failed to create tsid"));
        let mut context =
            ContextManager::new(session_id.to_string(), store.clone(), 4096, tsid_generator);

        let _ = context
            .add_message(
                "user".to_string(),
                "Initial query".to_string(),
                None,
                None,
                None,
                0,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add initial user message");

        let _ = context
            .add_message(
                "assistant".to_string(),
                "Assistant reply".to_string(),
                None,
                None,
                None,
                1,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add assistant message");

        {
            let store_guard = store.read().expect("lock poisoned");
            let mut execution_context = ExecutionContext::new(session_id.to_string());
            execution_context.state = crate::workflow::react::types::RuntimeState::Running;
            execution_context.current_segment_id = 3;
            store_guard
                .upsert_execution_context(&execution_context)
                .expect("failed to persist execution context");
        }

        {
            let store_guard = store.read().expect("lock poisoned");
            let conn = store_guard.conn.lock().expect("db lock poisoned");
            conn.execute(
                "DELETE FROM workflow_context_messages WHERE session_id = ?1",
                rusqlite::params![session_id],
            )
            .expect("failed to clear ai cache");
        }

        let mut restored = ContextManager::new(
            session_id.to_string(),
            store.clone(),
            4096,
            Arc::new(TsidGenerator::new(2).expect("failed to create tsid")),
        );
        restored
            .load_history()
            .await
            .expect("load history should succeed");

        assert_eq!(restored.current_segment_id, 3);
        assert!(
            restored
                .ai_context_messages
                .iter()
                .all(|message| message.segment_id == 3),
            "rebuilt AI cache should adopt the recovered current segment id"
        );

        let store_guard = store.read().expect("lock poisoned");
        let conn = store_guard.conn.lock().expect("db lock poisoned");
        let persisted_segment_id: i32 = conn
            .query_row(
                "SELECT MAX(segment_id) FROM workflow_context_messages WHERE session_id = ?1",
                rusqlite::params![session_id],
                |row| row.get(0),
            )
            .expect("failed to read persisted ai cache segment id");
        assert_eq!(persisted_segment_id, 3);
    }

    #[tokio::test]
    async fn build_rollup_compression_candidate_starts_after_previous_summary() {
        let (_dir, store) = setup_store();
        let session_id = "session-summary-boundary-test";
        insert_workflow(&store, session_id);

        let tsid_generator = Arc::new(TsidGenerator::new(1).expect("failed to create tsid"));
        let mut context =
            ContextManager::new(session_id.to_string(), store.clone(), 4096, tsid_generator);

        let _ = context
            .add_message(
                "user".to_string(),
                "task 1".to_string(),
                None,
                None,
                None,
                0,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add first task");
        let completion_message_1 = context
            .add_message(
                "tool".to_string(),
                TASK_FINISHED.to_string(),
                None,
                None,
                None,
                1,
                false,
                None,
                Some(json!({ "tool_name": TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY })),
            )
            .await
            .expect("failed to add first completion message")
            .0;

        context
            .compress(
                "<state_snapshot>task1</state_snapshot>".to_string(),
                2,
                completion_message_1
                    .id
                    .expect("complete_workflow_with_summary should have id"),
            )
            .await
            .expect("initial compression should succeed");

        let _ = context
            .add_message(
                "user".to_string(),
                "task 2".to_string(),
                None,
                None,
                None,
                3,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add second task");
        let _completion_message_2 = context
            .add_message(
                "tool".to_string(),
                TASK_FINISHED.to_string(),
                None,
                None,
                None,
                4,
                false,
                None,
                Some(json!({ "tool_name": TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY })),
            )
            .await
            .expect("failed to add second completion message")
            .0;
        let _ = context
            .add_message(
                "user".to_string(),
                "task 3".to_string(),
                None,
                None,
                None,
                5,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add third task");
        let completion_message_3 = context
            .add_message(
                "tool".to_string(),
                TASK_FINISHED.to_string(),
                None,
                None,
                None,
                6,
                false,
                None,
                Some(json!({ "tool_name": TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY })),
            )
            .await
            .expect("failed to add third completion message")
            .0;
        let _ = context
            .add_message(
                "user".to_string(),
                "task 4".to_string(),
                None,
                None,
                None,
                7,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add fourth task");
        let completion_message_4 = context
            .add_message(
                "tool".to_string(),
                TASK_FINISHED.to_string(),
                None,
                None,
                None,
                8,
                false,
                None,
                Some(json!({ "tool_name": TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY })),
            )
            .await
            .expect("failed to add fourth completion message")
            .0;
        let post_finish = context
            .add_message(
                "user".to_string(),
                "task 5".to_string(),
                None,
                None,
                None,
                9,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add active task")
            .0;

        let (candidate, compressed_until_id) = context
            .build_rollup_compression_candidate()
            .expect("third uncompressed completed task should trigger rollup");

        assert_eq!(
            candidate.first().map(|m| m.message_kind.as_str()),
            Some("summary"),
            "incremental compression should start from the latest summary"
        );
        assert_eq!(
            candidate.last().and_then(|m| m.id),
            completion_message_3.id,
            "incremental compression should stop at the second completion after the latest summary so the latest completed task remains available"
        );
        assert_eq!(
            compressed_until_id,
            completion_message_3
                .id
                .expect("completion message id missing")
        );
        assert!(
            context
                .messages
                .iter()
                .any(|m| m.id == completion_message_4.id),
            "the latest completed task should remain available in memory"
        );
        assert!(
            context.messages.iter().any(|m| m.id == post_finish.id),
            "messages after the compression boundary should stay in memory"
        );
    }

    #[tokio::test]
    async fn build_rollup_compression_candidate_requires_three_finished_tasks_for_initial_summary()
    {
        let (_dir, store) = setup_store();
        let session_id = "session-initial-rollup-test";
        insert_workflow(&store, session_id);

        let tsid_generator = Arc::new(TsidGenerator::new(1).expect("failed to create tsid"));
        let mut context =
            ContextManager::new(session_id.to_string(), store.clone(), 4096, tsid_generator);

        let _ = context
            .add_message(
                "user".to_string(),
                "task 1".to_string(),
                None,
                None,
                None,
                0,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add first task");
        let _ = context
            .add_message(
                "tool".to_string(),
                TASK_FINISHED.to_string(),
                None,
                None,
                None,
                1,
                false,
                None,
                Some(json!({ "tool_name": TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY })),
            )
            .await
            .expect("failed to add first finish task");

        let _ = context
            .add_message(
                "user".to_string(),
                "task 2".to_string(),
                None,
                None,
                None,
                2,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add second task");
        let completion_message_2 = context
            .add_message(
                "tool".to_string(),
                TASK_FINISHED.to_string(),
                None,
                None,
                None,
                3,
                false,
                None,
                Some(json!({ "tool_name": TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY })),
            )
            .await
            .expect("failed to add second completion message")
            .0;

        assert!(
            context.build_rollup_compression_candidate().is_none(),
            "initial rollup should wait until three finished segments exist"
        );

        let _ = context
            .add_message(
                "user".to_string(),
                "task 3".to_string(),
                None,
                None,
                None,
                4,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add third task");
        let _completion_message_3 = context
            .add_message(
                "tool".to_string(),
                TASK_FINISHED.to_string(),
                None,
                None,
                None,
                5,
                false,
                None,
                Some(json!({ "tool_name": TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY })),
            )
            .await
            .expect("failed to add third completion message")
            .0;

        assert!(
            context.build_rollup_compression_candidate().is_none(),
            "initial rollup should wait until a new active task starts after the retained completed task"
        );

        let _active_message = context
            .add_message(
                "assistant".to_string(),
                "Starting task 4".to_string(),
                None,
                None,
                None,
                6,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add active task marker");

        let (candidate, compressed_until_id) = context.build_rollup_compression_candidate().expect(
            "initial rollup should trigger after three finished segments once active work resumes",
        );

        assert!(
            candidate
                .first()
                .is_some_and(|m| !ContextManager::is_summary_message(m)),
            "initial rollup should start from raw history when no summary exists"
        );
        assert_eq!(
            candidate.last().and_then(|m| m.id),
            completion_message_2.id,
            "initial rollup should stop at the second successful completion"
        );
        assert_eq!(
            compressed_until_id,
            completion_message_2
                .id
                .expect("completion message id missing")
        );
    }

    #[tokio::test]
    async fn rollup_candidate_requires_three_finished_segments_after_summary() {
        let (_dir, store) = setup_store();
        let session_id = "session-rollup-threshold-test";
        insert_workflow(&store, session_id);

        let tsid_generator = Arc::new(TsidGenerator::new(1).expect("failed to create tsid"));
        let mut context =
            ContextManager::new(session_id.to_string(), store.clone(), 4096, tsid_generator);

        let _ = context
            .add_message(
                "user".to_string(),
                "task 1".to_string(),
                None,
                None,
                None,
                0,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add first task");
        let completion_message_1 = context
            .add_message(
                "tool".to_string(),
                TASK_FINISHED.to_string(),
                None,
                None,
                None,
                1,
                false,
                None,
                Some(json!({ "tool_name": TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY })),
            )
            .await
            .expect("failed to add first completion message")
            .0;

        context
            .compress(
                "<state_snapshot>task1</state_snapshot>".to_string(),
                2,
                completion_message_1
                    .id
                    .expect("completion message should have id"),
            )
            .await
            .expect("initial compression should succeed");

        let _ = context
            .add_message(
                "user".to_string(),
                "task 2".to_string(),
                None,
                None,
                None,
                3,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add second task");
        let _completion_message_2 = context
            .add_message(
                "tool".to_string(),
                TASK_FINISHED.to_string(),
                None,
                None,
                None,
                4,
                false,
                None,
                Some(json!({ "tool_name": TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY })),
            )
            .await
            .expect("failed to add second finish task")
            .0;

        assert!(
            context.build_rollup_compression_candidate().is_none(),
            "background rollup should not trigger until three finished segments exist after the latest summary"
        );

        let _ = context
            .add_message(
                "user".to_string(),
                "task 3".to_string(),
                None,
                None,
                None,
                5,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add third task");
        let _completion_message_3 = context
            .add_message(
                "tool".to_string(),
                TASK_FINISHED.to_string(),
                None,
                None,
                None,
                6,
                false,
                None,
                Some(json!({ "tool_name": TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY })),
            )
            .await
            .expect("failed to add third completion message")
            .0;

        assert!(
            context.build_rollup_compression_candidate().is_none(),
            "background rollup should wait until a new active task starts after the retained completed task"
        );

        let _ = context
            .add_message(
                "assistant".to_string(),
                "Starting task 4".to_string(),
                None,
                None,
                None,
                7,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add active task marker");
        let _ = context
            .add_message(
                "user".to_string(),
                "task 4".to_string(),
                None,
                None,
                None,
                7,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add fourth task");
        let _completion_message_4 = context
            .add_message(
                "tool".to_string(),
                TASK_FINISHED.to_string(),
                None,
                None,
                None,
                8,
                false,
                None,
                Some(json!({ "tool_name": TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY })),
            )
            .await
            .expect("failed to add fourth completion message")
            .0;
        let _ = context
            .add_message(
                "assistant".to_string(),
                "Starting task 5".to_string(),
                None,
                None,
                None,
                9,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add active task after fourth completion");

        let (_, compressed_until_id) = context
            .build_rollup_compression_candidate()
            .expect("background rollup should trigger after three finished segments exist after the latest summary");
        assert_eq!(
            compressed_until_id,
            _completion_message_3
                .id
                .expect("third completion message id missing")
        );
    }

    #[tokio::test]
    async fn blocking_candidate_keeps_latest_completed_task_once_work_resumes() {
        let (_dir, store) = setup_store();
        let session_id = "session-blocking-threshold-test";
        insert_workflow(&store, session_id);

        let tsid_generator = Arc::new(TsidGenerator::new(1).expect("failed to create tsid"));
        let mut context =
            ContextManager::new(session_id.to_string(), store.clone(), 4096, tsid_generator);

        let _ = context
            .add_message(
                "user".to_string(),
                "task 1".to_string(),
                None,
                None,
                None,
                0,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add first task");
        let completion_message_1 = context
            .add_message(
                "tool".to_string(),
                TASK_FINISHED.to_string(),
                None,
                None,
                None,
                1,
                false,
                None,
                Some(json!({ "tool_name": TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY })),
            )
            .await
            .expect("failed to add first completion message")
            .0;

        assert!(
            context.build_blocking_compression_candidate().is_none(),
            "blocking compression should not remove the only completed task"
        );

        let _ = context
            .add_message(
                "user".to_string(),
                "task 2".to_string(),
                None,
                None,
                None,
                2,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add second task");
        let _completion_message_2 = context
            .add_message(
                "tool".to_string(),
                TASK_FINISHED.to_string(),
                None,
                None,
                None,
                3,
                false,
                None,
                Some(json!({ "tool_name": TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY })),
            )
            .await
            .expect("failed to add second completion message")
            .0;

        assert!(
            context.build_blocking_compression_candidate().is_none(),
            "blocking compression should wait for a new active task after the retained completion"
        );

        let _ = context
            .add_message(
                "assistant".to_string(),
                "Starting task 3".to_string(),
                None,
                None,
                None,
                4,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add active task marker");

        let (_, compressed_until_id) = context
            .build_blocking_compression_candidate()
            .expect("blocking compression should keep the latest completed task");
        assert_eq!(
            compressed_until_id,
            completion_message_1
                .id
                .expect("completion message id missing")
        );
    }

    #[tokio::test]
    async fn current_user_request_uses_work_segment_after_last_completion() {
        let (_dir, store) = setup_store();
        let session_id = "session-current-task-test";
        insert_workflow(&store, session_id);

        let tsid_generator = Arc::new(TsidGenerator::new(1).expect("failed to create tsid"));
        let mut context =
            ContextManager::new(session_id.to_string(), store.clone(), 4096, tsid_generator);

        let _ = context
            .add_message(
                "user".to_string(),
                "Initial task".to_string(),
                None,
                None,
                None,
                0,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add initial task");
        let _ = context
            .add_message(
                "tool".to_string(),
                TASK_FINISHED.to_string(),
                None,
                None,
                None,
                1,
                false,
                None,
                Some(json!({ "tool_name": TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY })),
            )
            .await
            .expect("failed to add finish task");
        let _ = context
            .add_message(
                "assistant".to_string(),
                "Working on the next request".to_string(),
                None,
                None,
                None,
                2,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add assistant message");
        let _ = context
            .add_message(
                "user".to_string(),
                "Current task after finish".to_string(),
                None,
                None,
                None,
                3,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add current task");

        assert_eq!(
            context.current_user_request_since_last_completion(),
            "Current task after finish".to_string()
        );

        let segment = context.messages_since_last_completion();
        assert_eq!(segment.len(), 2);
        assert_eq!(
            segment.first().map(|message| message.role.as_str()),
            Some("assistant")
        );
        assert_eq!(
            segment.last().map(|message| message.role.as_str()),
            Some("user")
        );
    }

    #[tokio::test]
    async fn rejected_completion_does_not_create_compression_boundary() {
        let (_dir, store) = setup_store();
        let session_id = "session-rejected-finish-task-test";
        insert_workflow(&store, session_id);

        let tsid_generator = Arc::new(TsidGenerator::new(1).expect("failed to create tsid"));
        let mut context =
            ContextManager::new(session_id.to_string(), store.clone(), 4096, tsid_generator);

        let _ = context
            .add_message(
                "user".to_string(),
                "Task".to_string(),
                None,
                None,
                None,
                0,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add task");

        let _ = context
            .add_message(
                "tool".to_string(),
                TASK_FINISHED.to_string(),
                None,
                None,
                None,
                1,
                false,
                None,
                Some(json!({
                    "tool_name": TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY,
                    "execution_status": "rejected",
                    "approval_status": "rejected"
                })),
            )
            .await
            .expect("failed to add rejected completion message");

        assert!(
            context.build_blocking_compression_candidate().is_none(),
            "rejected completion must not be treated as a successful compression boundary"
        );
    }

    #[test]
    fn approved_plan_summary_is_not_treated_as_compression_summary() {
        let approved_plan = WorkflowMessage {
            id: Some(1),
            session_id: "s".to_string(),
            role: "system".to_string(),
            message: "# APPROVED EXECUTION PLAN".to_string(),
            reasoning: None,
            message_kind: "summary".to_string(),
            message_subtype: Some("approved_plan".to_string()),
            segment_id: 1,
            source_event_type: None,
            metadata: Some(json!({
                "type": "summary",
                "subtype": "approved_plan",
                "plan_content": "do the work",
                "todo_content": "[]"
            })),
            attached_context: None,
            step_type: None,
            step_index: 0,
            is_error: false,
            error_type: None,
            created_at: None,
        };

        assert!(ContextManager::is_summary_message(&approved_plan));
        assert!(!ContextManager::is_compression_summary_message(
            &approved_plan
        ));
    }

    #[tokio::test]
    async fn get_messages_for_llm_carries_forward_context_on_new_user_message() {
        let (_dir, store) = setup_store();
        let session_id = "session-llm-segment-test";
        insert_workflow(&store, session_id);

        let tsid_generator = Arc::new(TsidGenerator::new(1).expect("failed to create tsid"));
        let mut context =
            ContextManager::new(session_id.to_string(), store.clone(), 4096, tsid_generator);

        let _ = context
            .add_message(
                "user".to_string(),
                "First task".to_string(),
                None,
                None,
                None,
                0,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add first task");

        let _ = context
            .add_message(
                "assistant".to_string(),
                "First completion report".to_string(),
                None,
                None,
                Some(crate::workflow::react::types::StepType::Think),
                1,
                false,
                None,
                Some(json!({ "message_kind": "completion_report" })),
            )
            .await
            .expect("failed to add first completion report");

        let _ = context
            .add_message(
                "tool".to_string(),
                TASK_FINISHED.to_string(),
                None,
                None,
                None,
                2,
                false,
                None,
                Some(json!({ "tool_name": TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY })),
            )
            .await
            .expect("failed to add finish task");

        let _ = context
            .add_message(
                "user".to_string(),
                "Second task".to_string(),
                None,
                None,
                None,
                3,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add second task");

        let llm_messages = context.get_messages_for_llm();
        let llm_contents = llm_messages
            .iter()
            .map(|message| message.message.as_str())
            .collect::<Vec<_>>();

        assert!(llm_contents
            .iter()
            .any(|content| content.contains("Second task")));
        assert!(llm_contents
            .iter()
            .any(|content| content.contains("First completion report")),
            "the assistant's previous completion report should remain in the LLM context to preserve continuity"
        );
    }

    #[tokio::test]
    async fn get_messages_for_llm_with_summary_keeps_only_latest_completed_task_and_current_task() {
        let (_dir, store) = setup_store();
        let session_id = "session-llm-summary-segment-test";
        insert_workflow(&store, session_id);

        let tsid_generator = Arc::new(TsidGenerator::new(1).expect("failed to create tsid"));
        let mut context =
            ContextManager::new(session_id.to_string(), store.clone(), 4096, tsid_generator);

        let _ = context
            .add_message(
                "user".to_string(),
                "Task one".to_string(),
                None,
                None,
                None,
                0,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add task one");
        let _ = context
            .add_message(
                "assistant".to_string(),
                "Task one completion report".to_string(),
                None,
                None,
                Some(crate::workflow::react::types::StepType::Think),
                1,
                false,
                None,
                Some(json!({ "message_kind": "completion_report" })),
            )
            .await
            .expect("failed to add task one report");
        let completion_one = context
            .add_message(
                "tool".to_string(),
                TASK_FINISHED.to_string(),
                None,
                None,
                None,
                2,
                false,
                None,
                Some(json!({ "tool_name": TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY })),
            )
            .await
            .expect("failed to add task one completion")
            .0;

        let _ = context
            .add_message(
                "user".to_string(),
                "Task two".to_string(),
                None,
                None,
                None,
                3,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add task two");
        let _ = context
            .add_message(
                "assistant".to_string(),
                "Task two completion report".to_string(),
                None,
                None,
                Some(crate::workflow::react::types::StepType::Think),
                4,
                false,
                None,
                Some(json!({ "message_kind": "completion_report" })),
            )
            .await
            .expect("failed to add task two report");
        let completion_two = context
            .add_message(
                "tool".to_string(),
                TASK_FINISHED.to_string(),
                None,
                None,
                None,
                5,
                false,
                None,
                Some(json!({ "tool_name": TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY })),
            )
            .await
            .expect("failed to add task two completion")
            .0;

        let _ = context
            .add_message(
                "user".to_string(),
                "Task three".to_string(),
                None,
                None,
                None,
                6,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add task three");
        let _ = context
            .add_message(
                "assistant".to_string(),
                "Task three completion report".to_string(),
                None,
                None,
                Some(crate::workflow::react::types::StepType::Think),
                7,
                false,
                None,
                Some(json!({ "message_kind": "completion_report" })),
            )
            .await
            .expect("failed to add task three report");
        let _completion_three = context
            .add_message(
                "tool".to_string(),
                TASK_FINISHED.to_string(),
                None,
                None,
                None,
                8,
                false,
                None,
                Some(json!({ "tool_name": TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY })),
            )
            .await
            .expect("failed to add task three completion")
            .0;

        context
            .compress(
                "<state_snapshot>rolled up older work</state_snapshot>".to_string(),
                9,
                completion_two.id.expect("completion two id missing"),
            )
            .await
            .expect("compression should succeed");

        let _ = context
            .add_message(
                "user".to_string(),
                "Task four".to_string(),
                None,
                None,
                None,
                10,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add task four");

        let llm_messages = context.get_messages_for_llm();
        let llm_contents = llm_messages
            .iter()
            .map(|message| message.message.as_str())
            .collect::<Vec<_>>();

        assert!(llm_contents
            .iter()
            .any(|content| content.contains("Previous Context Snapshot")));
        assert!(llm_contents
            .iter()
            .any(|content| content.contains("Task three")));
        assert!(llm_contents
            .iter()
            .any(|content| content.contains("Task three completion report")));
        assert!(llm_contents
            .iter()
            .any(|content| content.contains("Task four")));
        assert!(!llm_contents
            .iter()
            .any(|content| content.contains("Task one")));
        assert!(!llm_contents
            .iter()
            .any(|content| content.contains("Task one completion report")));
        assert!(!llm_contents
            .iter()
            .any(|content| content.contains("Task two completion report")));
        assert!(
            completion_one.id.is_some(),
            "sanity check: first completion should have a persisted id"
        );
    }

    #[test]
    fn sanitize_reasoning_content_removes_incomplete_think_wrappers() {
        let sanitized = ContextManager::sanitize_reasoning_content(Some(
            "<think>  analyze this first\nthen continue".to_string(),
        ));
        assert_eq!(
            sanitized.as_deref(),
            Some("analyze this first\nthen continue")
        );

        let sanitized_closed = ContextManager::sanitize_reasoning_content(Some(
            "<think>\nwrapped\n</think>".to_string(),
        ));
        assert_eq!(sanitized_closed.as_deref(), Some("wrapped"));
    }

    #[tokio::test]
    async fn tool_projection_prefers_llm_content_over_full_tool_message() {
        let (_dir, store) = setup_store();
        insert_workflow(&store, "tool_llm_projection");
        let mut context = ContextManager::new(
            "tool_llm_projection".to_string(),
            store,
            20_000,
            Arc::new(TsidGenerator::new(1).expect("failed to create tsid generator")),
        );

        context
            .add_message(
                "user".to_string(),
                "Investigate".to_string(),
                None,
                None,
                None,
                1,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add user message");

        context
            .add_message(
                "tool".to_string(),
                "FULL SHELL OUTPUT".to_string(),
                None,
                None,
                Some(StepType::Observe),
                2,
                false,
                None,
                Some(json!({
                    "tool_name": "bash",
                    "llm_content": "REDUCED SHELL OUTPUT"
                })),
            )
            .await
            .expect("failed to add tool message");

        let llm_messages = context.get_messages_for_llm();
        let llm_contents = llm_messages
            .iter()
            .map(|message| message.message.as_str())
            .collect::<Vec<_>>();

        assert!(llm_contents
            .iter()
            .any(|content| content.contains("REDUCED SHELL OUTPUT")));
        assert!(!llm_contents
            .iter()
            .any(|content| content.contains("FULL SHELL OUTPUT")));
    }

    #[tokio::test]
    async fn begin_new_task_segment_from_runtime_projection_keeps_latest_completed_context() {
        let (_dir, store) = setup_store();
        insert_workflow(&store, "new_task_segment_projection");
        let mut context = ContextManager::new(
            "new_task_segment_projection".to_string(),
            store,
            20_000,
            Arc::new(TsidGenerator::new(1).expect("failed to create tsid generator")),
        );

        context
            .add_message(
                "user".to_string(),
                "Original task".to_string(),
                None,
                None,
                None,
                1,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add original task");

        context
            .add_message(
                "assistant".to_string(),
                "Original analysis".to_string(),
                None,
                None,
                Some(StepType::Think),
                2,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add original analysis");

        let completion = context
            .add_message(
                "tool".to_string(),
                TASK_FINISHED.to_string(),
                None,
                None,
                Some(StepType::Observe),
                3,
                false,
                None,
                Some(json!({ "tool_name": TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY })),
            )
            .await
            .expect("failed to add completion")
            .0;
        assert!(completion.id.is_some(), "completion id should exist");

        context
            .begin_new_task_segment_from_runtime_projection()
            .await
            .expect("failed to begin new task segment");

        context
            .add_message(
                "user".to_string(),
                "Follow-up clarification".to_string(),
                None,
                None,
                None,
                4,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add follow-up");

        let llm_messages = context.get_messages_for_llm();
        let llm_contents = llm_messages
            .iter()
            .map(|message| message.message.as_str())
            .collect::<Vec<_>>();

        assert!(llm_contents
            .iter()
            .any(|content| content.contains("Original task")));
        assert!(llm_contents
            .iter()
            .any(|content| content.contains("Original analysis")));
        assert!(llm_contents
            .iter()
            .any(|content| content.contains(TASK_FINISHED)));
        assert!(llm_contents
            .iter()
            .any(|content| content.contains("Follow-up clarification")));
    }

    #[tokio::test]
    async fn get_messages_for_llm_keeps_only_latest_tool_message_per_tool_call_id() {
        let (_dir, store) = setup_store();
        insert_workflow(&store, "dedupe_tool_projection");
        let mut context = ContextManager::new(
            "dedupe_tool_projection".to_string(),
            store,
            20_000,
            Arc::new(TsidGenerator::new(1).expect("failed to create tsid generator")),
        );

        context
            .add_message(
                "user".to_string(),
                "Write the file".to_string(),
                None,
                None,
                None,
                1,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add user message");

        context
            .add_message(
                "assistant".to_string(),
                "".to_string(),
                None,
                Some("Need to write the file".to_string()),
                Some(StepType::Think),
                2,
                false,
                None,
                Some(json!({
                    "tool_calls": [{
                        "id": "tool_write",
                        "type": "function",
                        "function": {
                            "name": "write_file",
                            "arguments": "{}"
                        }
                    }]
                })),
            )
            .await
            .expect("failed to add assistant tool call");

        context
            .add_message(
                "tool".to_string(),
                "Preview payload".to_string(),
                None,
                None,
                Some(StepType::Observe),
                3,
                false,
                None,
                Some(json!({
                    "tool_call_id": "tool_write",
                    "tool_name": "write_file",
                    "approval_status": "pending"
                })),
            )
            .await
            .expect("failed to add preview tool message");

        context
            .add_message(
                "tool".to_string(),
                "{\"bytes_written\":128}".to_string(),
                None,
                None,
                Some(StepType::Observe),
                4,
                false,
                None,
                Some(json!({
                    "tool_call_id": "tool_write",
                    "tool_name": "write_file",
                    "approval_status": "approved"
                })),
            )
            .await
            .expect("failed to add final tool message");

        let llm_messages = context.get_messages_for_llm();
        let tool_messages = llm_messages
            .iter()
            .filter(|message| message.role == "tool")
            .collect::<Vec<_>>();

        assert_eq!(tool_messages.len(), 1);
        assert_eq!(
            tool_messages[0]
                .metadata
                .as_ref()
                .and_then(|meta| meta.get("tool_call_id"))
                .and_then(|value| value.as_str()),
            Some("tool_write")
        );
        assert!(tool_messages[0].message.contains("bytes_written"));
    }

    #[tokio::test]
    async fn get_messages_for_llm_drops_orphan_tool_messages() {
        let (_dir, store) = setup_store();
        insert_workflow(&store, "orphan_tool_projection");
        let mut context = ContextManager::new(
            "orphan_tool_projection".to_string(),
            store,
            20_000,
            Arc::new(TsidGenerator::new(1).expect("failed to create tsid generator")),
        );

        context
            .add_message(
                "user".to_string(),
                "Continue".to_string(),
                None,
                None,
                None,
                1,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add user message");

        context
            .add_message(
                "assistant".to_string(),
                "I am thinking.".to_string(),
                None,
                None,
                Some(StepType::Think),
                2,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add assistant message");

        context
            .add_message(
                "tool".to_string(),
                "late tool result".to_string(),
                None,
                None,
                Some(StepType::Observe),
                3,
                false,
                None,
                Some(json!({
                    "tool_call_id": "tool_late",
                    "tool_name": "read_file"
                })),
            )
            .await
            .expect("failed to add orphan tool message");

        let llm_messages = context.get_messages_for_llm();
        assert!(llm_messages.iter().all(|message| message.role != "tool"));
    }

    #[tokio::test]
    async fn get_messages_for_llm_strips_unresolved_assistant_tool_calls() {
        let (_dir, store) = setup_store();
        insert_workflow(&store, "dangling_assistant_tool_call");
        let mut context = ContextManager::new(
            "dangling_assistant_tool_call".to_string(),
            store,
            20_000,
            Arc::new(TsidGenerator::new(1).expect("failed to create tsid generator")),
        );

        context
            .add_message(
                "user".to_string(),
                "Finish the task".to_string(),
                None,
                None,
                None,
                1,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add user message");

        context
            .add_message(
                "assistant".to_string(),
                "".to_string(),
                None,
                Some("Submitting completion".to_string()),
                Some(StepType::Think),
                2,
                false,
                None,
                Some(json!({
                    "tool_calls": [{
                        "id": "tool_finish",
                        "type": "function",
                        "function": {
                            "name": "complete_workflow_with_summary",
                            "arguments": "{}"
                        }
                    }]
                })),
            )
            .await
            .expect("failed to add dangling assistant tool call");

        context
            .add_message(
                "user".to_string(),
                "Start the next task".to_string(),
                None,
                None,
                None,
                3,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add next task message");

        let llm_messages = context.get_messages_for_llm();
        let assistant = llm_messages
            .iter()
            .find(|message| message.role == "assistant")
            .expect("assistant message should remain for reasoning continuity");
        assert!(
            assistant
                .metadata
                .as_ref()
                .and_then(|meta| meta.get("tool_calls"))
                .is_none(),
            "unresolved assistant tool_calls should be stripped from projection"
        );
    }

    #[tokio::test]
    async fn get_messages_for_llm_hides_runtime_observations_marked_hidden() {
        let (_dir, store) = setup_store();
        insert_workflow(&store, "hidden_runtime_observation");
        let mut context = ContextManager::new(
            "hidden_runtime_observation".to_string(),
            store,
            20_000,
            Arc::new(TsidGenerator::new(1).expect("failed to create tsid generator")),
        );

        context
            .add_message(
                "user".to_string(),
                "Investigate".to_string(),
                None,
                None,
                None,
                1,
                false,
                None,
                None,
            )
            .await
            .expect("failed to add user message");

        context
            .add_message(
                "tool".to_string(),
                "<SYSTEM_REMINDER>Internal note</SYSTEM_REMINDER>".to_string(),
                None,
                None,
                Some(StepType::Observe),
                2,
                false,
                None,
                Some(json!({
                    "message_kind": "runtime_observation",
                    "observation_type": "generic_reminder",
                    "llm_visibility": "hide",
                    "ui_visibility": "show"
                })),
            )
            .await
            .expect("failed to add hidden runtime observation");

        let llm_messages = context.get_messages_for_llm();
        assert_eq!(llm_messages.len(), 1);
        assert_eq!(llm_messages[0].role, "user");
    }

    #[tokio::test]
    async fn compression_preserves_latest_complete_workflow_summary_tool_response_in_ai_context() {
        let (_dir, store) = setup_store();
        let session_id = "compress-tool-response-test";
        insert_workflow(&store, session_id);

        let tsid_generator = Arc::new(TsidGenerator::new(1).expect("failed to create tsid"));
        let mut context =
            ContextManager::new(session_id.to_string(), store.clone(), 4096, tsid_generator);

        // --- Task 1: complete ---
        let _ = context
            .add_message(
                "user".to_string(),
                "task 1".to_string(),
                None,
                None,
                None,
                1,
                false,
                None,
                None,
            )
            .await
            .expect("add user");
        let _ = context
            .add_message(
                "tool".to_string(),
                TASK_FINISHED.to_string(),
                None,
                None,
                None,
                2,
                false,
                None,
                Some(json!({
                    "tool_call_id": "tool_complete_1",
                    "tool_name": TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY,
                    "execution_status": "completed",
                    "approval_status": "approved"
                })),
            )
            .await
            .expect("add task1 completion")
            .0;

        // --- Task 2: complete ---
        let _ = context
            .add_message(
                "user".to_string(),
                "task 2".to_string(),
                None,
                None,
                None,
                3,
                false,
                None,
                None,
            )
            .await
            .expect("add user");
        let completion_2 = context
            .add_message(
                "tool".to_string(),
                TASK_FINISHED.to_string(),
                None,
                None,
                None,
                4,
                false,
                None,
                Some(json!({
                    "tool_call_id": "tool_complete_2",
                    "tool_name": TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY,
                    "execution_status": "completed",
                    "approval_status": "approved"
                })),
            )
            .await
            .expect("add task2 completion")
            .0;

        // --- Task 3: complete with assistant having tool_calls ---
        let _ = context
            .add_message(
                "user".to_string(),
                "task 3".to_string(),
                None,
                None,
                None,
                5,
                false,
                None,
                None,
            )
            .await
            .expect("add user");
        // Simulate assistant calling complete_workflow_with_summary
        let tool_call_id = "tool_complete_3";
        let _ = context
            .add_message(
                "assistant".to_string(),
                "".to_string(),
                None,
                Some("reasoning text".to_string()),
                None,
                6,
                false,
                None,
                Some(json!({
                    "tool_calls": [{
                        "id": tool_call_id,
                        "type": "function",
                        "function": {
                            "name": TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY,
                            "arguments": "{\"summary\":\"done\"}"
                        }
                    }]
                })),
            )
            .await
            .expect("add assistant");
        let _completion_3 = context
            .add_message(
                "tool".to_string(),
                "Completed".to_string(),
                None,
                None,
                None,
                7,
                false,
                None,
                Some(json!({
                    "tool_call_id": tool_call_id,
                    "tool_name": TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY,
                    "execution_status": "completed",
                    "approval_status": "approved"
                })),
            )
            .await
            .expect("add task3 tool response");

        // --- New user message triggers task boundary compression ---
        let _ = context
            .add_message(
                "user".to_string(),
                "new task".to_string(),
                None,
                None,
                None,
                8,
                false,
                None,
                None,
            )
            .await
            .expect("add new user");

        // Verify compression candidate exists and stops at correct boundary
        let candidate = context.build_task_boundary_compression_candidate();
        assert!(candidate.is_some(), "should have 3+ completions for rollup");
        let (candidate_msgs, compressed_until_id) = candidate.unwrap();
        assert_eq!(
            candidate_msgs.last().and_then(|m| m.id),
            completion_2.id,
            "compression should stop at second-to-last completion"
        );

        // Run compression
        context
            .compress(
                "<state_snapshot>compressed</state_snapshot>".to_string(),
                99,
                compressed_until_id,
            )
            .await
            .expect("compression should succeed");

        // ===== THE KEY CHECK =====
        // After compression, the latest task's complete_workflow_with_summary
        // tool response (with tool_call_id = "tool_complete_3") MUST be in ai_context_messages

        let tool_in_ai_ctx = context.ai_context_messages.iter().any(|m| {
            m.role == "tool"
                && m.metadata
                    .as_ref()
                    .and_then(|meta| meta.get("tool_call_id").and_then(|v| v.as_str()))
                    == Some(tool_call_id)
        });

        assert!(
            tool_in_ai_ctx,
            "BUG: complete_workflow_with_summary tool response missing from ai_context_messages after compression"
        );

        // Also check that the assistant with tool_calls has its tool response right after
        let llm_messages = context.get_messages_for_llm();
        let assistant_pos = llm_messages.iter().position(|m| {
            m.role == "assistant"
                && m.metadata
                    .as_ref()
                    .and_then(|meta| meta.get("tool_calls").and_then(|tc| tc.as_array()))
                    .is_some_and(|tc| {
                        tc.iter()
                            .any(|t| t.get("id").and_then(|v| v.as_str()) == Some(tool_call_id))
                    })
        });
        assert!(
            assistant_pos.is_some(),
            "assistant with tool_calls should be in LLM messages"
        );
        let assistant_pos = assistant_pos.unwrap();
        let tool_after = &llm_messages[assistant_pos + 1];
        assert_eq!(tool_after.role, "tool");
        assert_eq!(
            tool_after
                .metadata
                .as_ref()
                .and_then(|meta| meta.get("tool_call_id").and_then(|v| v.as_str())),
            Some(tool_call_id)
        );
    }
}
