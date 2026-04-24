use crate::ai::chat::openai::OpenAIChat;
use crate::ai::error::AiError;
use crate::ai::interaction::chat_completion::{AiChatEnum, ChatState};
use crate::ai::traits::chat::ChatMetadata;
use crate::db::WorkflowMessage;
use crate::workflow::react::error::WorkflowEngineError;
use crate::workflow::react::prompts::CONTEXT_COMPRESSION_PROMPT;

use serde_json::json;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

#[derive(Clone)]
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
            .filter_map(|m| {
                let content = Self::sanitize_message_content_for_compression(&m.message);
                let keep = !content.is_empty()
                    && content != "Finished"
                    && !content.contains("Waiting for user");
                keep.then(|| serde_json::json!({ "role": m.role, "content": content }))
            })
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
        let transcript = Self::render_history_as_transcript(&history_json);
        let full_history = vec![
            json!({ "role": "system", "content": CONTEXT_COMPRESSION_PROMPT }),
            json!({
                "role": "user",
                "content": format!(
                    "<conversation_history>\n{}\n</conversation_history>\n\n{}",
                    transcript,
                    user_prompt
                )
            }),
        ];

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

        let max_attempts = 3;
        let mut attempt = 0;
        let result = loop {
            attempt += 1;
            match chat_interface
                .chat(
                    self.provider_id,
                    &self.model,
                    "compressor_session".to_string(),
                    full_history.clone(),
                    None,
                    Some(ChatMetadata {
                        stream: Some(false),
                        ..Default::default()
                    }),
                    |_| {},
                )
                .await
            {
                Ok(result) => break result,
                Err(err)
                    if attempt < max_attempts && Self::should_retry_compression_error(&err) =>
                {
                    let wait_secs = 2u64.pow(attempt - 1);
                    log::warn!(
                        "ContextCompressor: compression attempt {}/{} failed, retrying in {}s: {}",
                        attempt,
                        max_attempts,
                        wait_secs,
                        err
                    );
                    sleep(Duration::from_secs(wait_secs)).await;
                }
                Err(err) => return Err(WorkflowEngineError::Ai(err)),
            }
        };

        Ok(Self::normalize_summary_result(&result))
    }

    fn sanitize_message_content_for_compression(content: &str) -> String {
        const SPECIAL_TOKENS: [&str; 33] = [
            "<｜end▁of▁sentence｜>",
            "<|end_of_sentence|>",
            "<｜begin▁of▁sentence｜>",
            "<|begin_of_sentence|>",
            "<｜endoftext｜>",
            "<|endoftext|>",
            "<｜end_of_text｜>",
            "<|end_of_text|>",
            "<｜begin_of_text｜>",
            "<|begin_of_text|>",
            "<｜im_start｜>",
            "<|im_start|>",
            "<｜im_end｜>",
            "<|im_end|>",
            "<｜im_middle｜>",
            "<|im_middle|>",
            "<｜system｜>",
            "<|system|>",
            "<｜user｜>",
            "<|user|>",
            "<｜assistant｜>",
            "<|assistant|>",
            "<｜observation｜>",
            "<|observation|>",
            "<|SYSTEM|>",
            "<|USER|>",
            "<|ASSISTANT|>",
            "<|OBSERVATION|>",
            "<|EOT|>",
            "<|eot|>",
            "[gMASK]",
            "[MASK]",
            "[sMASK]",
        ];

        let mut sanitized = content.to_string();
        for token in SPECIAL_TOKENS {
            sanitized = sanitized.replace(token, "");
        }
        sanitized = sanitized
            .split_whitespace()
            .filter(|part| !matches!(*part, "gMASK" | "sop" | "eop"))
            .collect::<Vec<_>>()
            .join(" ");
        sanitized.trim().to_string()
    }

    fn should_retry_compression_error(error: &AiError) -> bool {
        match error {
            AiError::ApiRequestFailed { status_code, .. } => {
                *status_code == 408 || *status_code == 429 || *status_code >= 500
            }
            AiError::InitFailed(_)
            | AiError::InvalidInput(_)
            | AiError::ToolCallSerializationFailed { .. } => false,
            AiError::ResponseParseFailed { .. }
            | AiError::StreamProcessingFailed { .. }
            | AiError::FailedToGetOrCreateWindowChannel(_) => true,
        }
    }

    fn render_history_as_transcript(history_json: &[serde_json::Value]) -> String {
        history_json
            .iter()
            .filter_map(|message| {
                let role = message
                    .get("role")
                    .and_then(|value| value.as_str())
                    .unwrap_or("unknown");
                let content = message
                    .get("content")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
                    .trim();
                if content.is_empty() {
                    return None;
                }
                Some(format!(
                    "<message role=\"{}\">\n{}\n</message>",
                    role, content
                ))
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    fn normalize_summary_result(result: &str) -> String {
        let trimmed = result.trim();
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(content) = parsed.get("content").and_then(|value| value.as_str()) {
                let content = content.trim();
                if !content.is_empty() {
                    return content.to_string();
                }
            }
        }

        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::ContextCompressor;

    #[test]
    fn normalize_summary_result_extracts_content_field_from_json() {
        let raw = r#"{"content":"<state_snapshot>\n  <overall_goal>goal</overall_goal>\n</state_snapshot>","reasoning":"ignored"}"#;
        let normalized = ContextCompressor::normalize_summary_result(raw);
        assert_eq!(
            normalized,
            "<state_snapshot>\n  <overall_goal>goal</overall_goal>\n</state_snapshot>"
        );
    }

    #[test]
    fn normalize_summary_result_preserves_plain_xml() {
        let raw = "<state_snapshot><overall_goal>goal</overall_goal></state_snapshot>";
        let normalized = ContextCompressor::normalize_summary_result(raw);
        assert_eq!(normalized, raw);
    }

    #[test]
    fn sanitize_message_content_for_compression_removes_provider_special_tokens() {
        let sanitized = ContextCompressor::sanitize_message_content_for_compression(
            "用户请求<｜end▁of▁sentence｜><|endoftext|><|im_end|>[gMASK] sop",
        );
        assert_eq!(sanitized, "用户请求");
    }

    #[test]
    fn render_history_as_transcript_preserves_roles_without_replaying_chat_roles() {
        let transcript = ContextCompressor::render_history_as_transcript(&[
            serde_json::json!({"role":"user","content":"task"}),
            serde_json::json!({"role":"tool","content":"result"}),
        ]);

        assert!(transcript.contains("<message role=\"user\">"));
        assert!(transcript.contains("<message role=\"tool\">"));
        assert!(transcript.contains("result"));
    }
}
