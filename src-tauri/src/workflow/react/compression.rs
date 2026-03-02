use crate::ai::chat::openai::OpenAIChat;
use crate::ai::interaction::chat_completion::{AiChatEnum, ChatState};
use crate::ai::traits::chat::ChatMetadata;
use crate::db::WorkflowMessage;
use crate::workflow::react::error::WorkflowEngineError;
use serde_json::json;
use std::sync::Arc;

pub const SUMMARY_PROMPT: &str = r#"You are a high-performance context compressor.
Your goal is to maintain and update a structured <state_snapshot> XML block that represents the cumulative state of an Agent's task.

## RULES FOR COMPRESSION:
1. **Snapshot Update**: You will receive the LAST <state_snapshot> and the newest messages. You MUST merge the new progress into the snapshot. Produce ONE unified <state_snapshot>.
2. **Goal Preservation**: Always keep the user's primary objective. Update it only if the intent has shifted.
3. **Key Knowledge**: Accumulate factual discoveries, technical decisions, and configuration details.
4. **Error Log & Loop Prevention**:
    - Consolidate repeated identical errors into a single entry.
    - If the Agent has made the same mistake multiple times (e.g., repeatedly trying a non-existent path), summarize it as one event with a frequency count (e.g., "Failed to read X (attempted 5 times)").
    - Clearly mark whether an error is [RESOLVED] or [PERSISTENT/UNRESOLVED].
5. **Memory Externalization**: DO NOT summarize file contents or large data. Instead, list their FILE PATHS or URLs as reference pointers.
6. **Task Status**: Update the status of tasks: [DONE], [IN PROGRESS], [TODO].

## OUTPUT FORMAT:
Your output MUST be a valid XML structure:

<state_snapshot>
    <overall_goal>Current primary objective</overall_goal>
    <key_knowledge>Cumulative factual discoveries and decisions</key_knowledge>
    <error_log>Significant errors encountered and their specific resolutions</error_log>
    <file_system_state>Modified files and reference pointers (paths/URLs only)</file_system_state>
    <recent_actions>Summary of recent critical tool outputs and observations</recent_actions>
    <task_state>Current plan and updated task checklist</task_state>
</state_snapshot>"#;

pub struct ContextCompressor {
    pub chat_state: Arc<ChatState>,
    pub provider_id: i64,
    pub model: String,
}

impl ContextCompressor {
    pub fn new(chat_state: Arc<ChatState>, provider_id: i64, model: String) -> Self {
        Self {
            chat_state,
            provider_id,
            model,
        }
    }

    /// Compresses messages into a single factual snapshot.
    /// This follows an incremental strategy: [Last Snapshot] + [New Messages] -> [New Snapshot]
    pub async fn compress(
        &self,
        messages: &[WorkflowMessage],
    ) -> Result<String, WorkflowEngineError> {
        if messages.is_empty() {
            return Ok(String::new());
        }

        // 1. Locate the LATEST summary to implement incremental compression
        let last_summary_idx = messages.iter().rposition(|m| {
            m.metadata
                .as_ref()
                .map_or(false, |meta| meta["type"] == "summary")
        });

        // 2. Slice history from the last summary point to now
        let incremental_messages = if let Some(idx) = last_summary_idx {
            &messages[idx..]
        } else {
            messages
        };

        // 3. Layer 1: Purification (Filter out noise)
        let purified_history: Vec<serde_json::Value> = incremental_messages
            .iter()
            .filter(|m| {
                let content = m.message.trim();
                !content.is_empty()
                    && content != "Finished"
                    && !content.contains("Waiting for user")
            })
            .map(|m| serde_json::json!({ "role": m.role, "content": m.message }))
            .collect();

        if purified_history.is_empty() {
            return Ok("No meaningful progress to compress.".to_string());
        }

        // 4. Strategic Summary (LLM Call)
        let user_prompt = if last_summary_idx.is_some() {
            "Please update the existing <state_snapshot> with the new information provided above. Output only the updated XML block."
        } else {
            "Create the first <state_snapshot> based on the conversation history provided above."
        };

        let summary = self
            .extract_fact_from_history(purified_history, user_prompt)
            .await?;
        Ok(summary)
    }

    async fn extract_fact_from_history(
        &self,
        history_json: Vec<serde_json::Value>,
        user_prompt: &str,
    ) -> Result<String, WorkflowEngineError> {
        let mut full_history = vec![json!({ "role": "system", "content": SUMMARY_PROMPT })];
        full_history.extend(history_json);
        full_history.push(json!({ "role": "user", "content": user_prompt }));

        let chat_interface = {
            let mut chats_guard = self.chat_state.chats.lock().await;
            let protocol = crate::ccproxy::ChatProtocol::OpenAI;
            let chat_map = chats_guard
                .entry(protocol)
                .or_insert_with(std::collections::HashMap::new);
            chat_map
                .entry("compressor".to_string())
                .or_insert_with(|| crate::create_chat!(self.chat_state.main_store))
                .clone()
        };

        log::info!("ContextCompressor: Executing incremental compression...");

        let result = chat_interface
            .chat(
                self.provider_id,
                &self.model,
                "compressor_session".to_string(),
                full_history,
                None,
                Some(ChatMetadata {
                    stream: Some(false),
                    ..Default::default()
                }),
                |_| {},
            )
            .await
            .map_err(WorkflowEngineError::Ai)?;

        Ok(result.trim().to_string())
    }
}
