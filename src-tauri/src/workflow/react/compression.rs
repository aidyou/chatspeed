use crate::ai::chat::openai::OpenAIChat;
use crate::ai::interaction::chat_completion::{AiChatEnum, ChatState};
use crate::ai::traits::chat::ChatMetadata;
use crate::db::WorkflowMessage;
use crate::workflow::react::error::WorkflowEngineError;
use serde_json::json;
use std::sync::Arc;

use crate::workflow::react::prompts::CONTEXT_COMPRESSION_PROMPT;

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
        let mut full_history = vec![json!({ "role": "system", "content": CONTEXT_COMPRESSION_PROMPT })];
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
