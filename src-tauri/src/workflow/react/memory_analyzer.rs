//! Memory analyzer for ChatSpeed workflow engine.
//!
//! Performs memory analysis using a separate LLM session after task completion.
//! This runs as an independent analysis step to determine what memories should
//! be recorded from the user's inputs.

use std::sync::Arc;
use tokio::sync::mpsc;

use crate::ai::chat::openai::OpenAIChat;
use crate::ai::interaction::chat_completion::AiChatEnum;
use crate::ai::interaction::chat_completion::ChatState;
use crate::ai::traits::chat::{ChatMetadata, ChatResponse};
use crate::ccproxy::ChatProtocol;

use super::error::WorkflowEngineError;
use super::memory::MemoryAnalysisResult;

/// Memory analyzer for task completion memory updates.
///
/// Analyzes user inputs and existing memories to determine what should be
/// recorded for future sessions.
pub struct MemoryAnalyzer;

impl MemoryAnalyzer {
    /// Analyzes user inputs and updates memory files.
    ///
    /// This method:
    /// 1. Collects user inputs from the session
    /// 2. Reads current memories
    /// 3. Calls the AI with a specialized prompt
    /// 4. Parses the AI's response
    /// 5. Updates memory files if changes are needed
    ///
    /// # Arguments
    /// * `user_inputs` - Vector of user messages from the session
    /// * `current_global_memory` - Current global memory content
    /// * `current_project_memory` - Current project memory content
    /// * `chat_state` - Chat state for LLM access
    /// * `session_id` - Session identifier
    /// * `provider_id` - AI provider ID
    /// * `model_name` - Model name to use
    ///
    /// # Returns
    /// * `Ok(MemoryAnalysisResult)` - Analysis result with any memory updates
    /// * `Err(WorkflowEngineError)` - If analysis fails
    pub async fn analyze(
        user_inputs: Vec<String>,
        current_global_memory: Option<String>,
        current_project_memory: Option<String>,
        chat_state: Arc<ChatState>,
        session_id: &str,
        provider_id: i64,
        model_name: &str,
    ) -> Result<MemoryAnalysisResult, WorkflowEngineError> {
        // Skip if no user inputs
        if user_inputs.is_empty() {
            return Ok(MemoryAnalysisResult {
                global_memory: None,
                project_memory: None,
                reasoning: Some("No user inputs to analyze".to_string()),
            });
        }

        // Get chat interface
        let chat_interface = {
            let mut chats_guard = chat_state.chats.lock().await;
            let protocol = ChatProtocol::OpenAI;
            let chat_map = chats_guard
                .entry(protocol)
                .or_insert_with(std::collections::HashMap::new);
            chat_map
                .entry(session_id.to_string())
                .or_insert_with(|| crate::create_chat!(chat_state.main_store.clone()))
                .clone()
        };

        // Build prompts
        let system_prompt = super::prompts::MEMORY_ANALYZER_SYSTEM_PROMPT;

        let user_prompt = format!(
            "{}",
            super::prompts::MEMORY_ANALYZER_USER_PROMPT_TEMPLATE
                .replace("{global_memory}", &current_global_memory.unwrap_or_default())
                .replace("{project_memory}", &current_project_memory.unwrap_or_default())
                .replace("{user_inputs}", &user_inputs.join("\n\n---\n\n"))
        );

        // Build messages
        let messages = vec![
            serde_json::json!({
                "role": "system",
                "content": system_prompt
            }),
            serde_json::json!({
                "role": "user",
                "content": user_prompt
            }),
        ];

        // Call LLM with streaming
        let (tx, _rx) = mpsc::channel::<Arc<ChatResponse>>(100);

        let response = chat_interface
            .chat(
                provider_id,
                model_name,
                format!("{}-memory", session_id),
                messages,
                None, // No tools needed for analysis
                Some(ChatMetadata {
                    temperature: Some(0.3), // Low temperature for consistent analysis
                    ..Default::default()
                }),
                move |chunk| {
                    let _ = tx.try_send(chunk);
                },
            )
            .await
            .map_err(|e| WorkflowEngineError::General(format!("Memory analysis LLM call failed: {}", e)))?;

        // Parse JSON response
        // The response should be a JSON object with global_memory and project_memory
        let result = Self::parse_response(&response)?;

        Ok(result)
    }

    /// Parses the AI response to extract memory updates.
    fn parse_response(response: &str) -> Result<MemoryAnalysisResult, WorkflowEngineError> {
        // AI responses are often wrapped in markdown code blocks or contain reasoning text.
        // Use our utility function to extract and clean the JSON.
        let json_str = crate::libs::util::format_json_str(response);

        // Parse JSON
        let result: MemoryAnalysisResult = serde_json::from_str(&json_str).map_err(|e| {
            WorkflowEngineError::General(format!(
                "Failed to parse memory analysis result: {}. Response: {}",
                e, response
            ))
        })?;

        Ok(result)
    }

    // Removed manual extract_json as it's replaced by crate::libs::util::format_json_str
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json_with_code_block() {
        let response = r#"
Some text before
```json
{"global_memory": null, "project_memory": null, "reasoning": "No changes needed"}
```
Some text after
"#;
        let json = MemoryAnalyzer::extract_json(response).unwrap();
        assert!(json.contains("global_memory"));
    }

    #[test]
    fn test_extract_json_plain() {
        let response = r#"{"global_memory": null, "project_memory": null}"#;
        let json = MemoryAnalyzer::extract_json(response).unwrap();
        assert!(json.contains("global_memory"));
    }

    #[test]
    fn test_parse_response() {
        let json = r#"{"global_memory": null, "project_memory": null, "reasoning": "Test"}"#;
        let result: MemoryAnalysisResult = serde_json::from_str(json).unwrap();
        assert!(result.global_memory.is_none());
        assert!(result.project_memory.is_none());
        assert!(result.reasoning.is_some());
    }
}