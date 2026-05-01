use crate::ai::chat::openai::OpenAIChat;
use crate::ai::error::AiError;
use crate::ai::interaction::chat_completion::{AiChatEnum, ChatState};
use crate::ai::traits::chat::ChatMetadata;
use crate::db::WorkflowMessage;
use crate::tools::TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY;
use crate::workflow::react::context::ContextManager;
use crate::workflow::react::error::WorkflowEngineError;
use crate::workflow::react::intelligence::IntelligenceManager;
use crate::workflow::react::prompts::{
    BLOCKING_CONTEXT_COMPRESSION_PROMPT, ROLLUP_CONTEXT_COMPRESSION_PROMPT,
};

use serde_json::json;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

#[derive(Clone)]
pub struct ContextCompressor {
    pub chat_state: Arc<ChatState>,
    pub provider_id: i64,
    pub model: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompressionMode {
    #[allow(dead_code)]
    Rollup,
    Blocking,
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
        mode: CompressionMode,
    ) -> Result<String, WorkflowEngineError> {
        if messages.is_empty() {
            return Ok(String::new());
        }

        // 1. Locate the LATEST summary to implement incremental compression
        let last_summary_idx = messages
            .iter()
            .rposition(ContextManager::is_compression_summary_message);

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
                if Self::should_skip_message_for_compression(m) {
                    return None;
                }
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

        let completed_tasks = Self::render_completed_tasks(incremental_messages);

        // 4. Strategic Summary (LLM Call)
        let user_prompt = if last_summary_idx.is_some() {
            "Please update the existing <state_snapshot> with the new information provided above. Output only the updated XML block."
        } else {
            "Create the first <state_snapshot> based on the conversation history provided above."
        };

        let summary = self
            .extract_fact_from_history(purified_history, &completed_tasks, user_prompt, mode)
            .await?;
        Ok(summary)
    }

    async fn extract_fact_from_history(
        &self,
        history_json: Vec<serde_json::Value>,
        completed_tasks: &str,
        user_prompt: &str,
        mode: CompressionMode,
    ) -> Result<String, WorkflowEngineError> {
        let transcript = Self::render_history_as_transcript(&history_json);
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
        let mut retry_instruction = String::new();
        loop {
            attempt += 1;
            let full_history = vec![
                json!({
                    "role": "system",
                    "content": match mode {
                        CompressionMode::Rollup => ROLLUP_CONTEXT_COMPRESSION_PROMPT,
                        CompressionMode::Blocking => BLOCKING_CONTEXT_COMPRESSION_PROMPT,
                    }
                }),
                json!({
                    "role": "user",
                    "content": format!(
                        "<completed_tasks>\n{}\n</completed_tasks>\n\n<conversation_history>\n{}\n</conversation_history>\n\n{}{}",
                        completed_tasks,
                        transcript,
                        user_prompt,
                        retry_instruction
                    )
                }),
            ];

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
                Ok(result) => {
                    let normalized = Self::normalize_summary_result(&result);
                    let validation_error = match Self::validate_summary_result(&normalized) {
                        Ok(validated) => return Ok(validated),
                        Err(err) => err,
                    };

                    if attempt < max_attempts {
                        let wait_secs = 2u64.pow(attempt - 1);
                        retry_instruction =
                            Self::build_retry_instruction(&validation_error, completed_tasks);
                        log::info!(
                            "ContextCompressor: compression attempt {}/{} returned invalid summary format, retrying in {}s. validation_error={}. normalized_preview={}",
                            attempt,
                            max_attempts,
                            wait_secs,
                            validation_error,
                            Self::preview_for_log(&normalized, 500)
                        );
                        sleep(Duration::from_secs(wait_secs)).await;
                        continue;
                    }

                    return Err(WorkflowEngineError::General(format!(
                        "Compression returned invalid summary format after {} attempts: {}",
                        max_attempts, validation_error
                    )));
                }
                Err(err)
                    if attempt < max_attempts && Self::should_retry_compression_error(&err) =>
                {
                    let wait_secs = 2u64.pow(attempt - 1);
                    log::info!(
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
        }
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

    fn should_skip_message_for_compression(message: &WorkflowMessage) -> bool {
        let Some(meta) = message.metadata.as_ref() else {
            return false;
        };

        let tool_name = meta.get("tool_name").and_then(|value| value.as_str());
        let approval_status = meta.get("approval_status").and_then(|value| value.as_str());

        tool_name == Some("submit_plan") && approval_status == Some("approved")
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

    fn render_completed_tasks(messages: &[WorkflowMessage]) -> String {
        let mut tasks = Vec::new();
        let mut segment_start = messages
            .iter()
            .position(|message| message.message_kind != "summary")
            .unwrap_or(messages.len());

        for (idx, message) in messages.iter().enumerate().skip(segment_start) {
            if !Self::is_successful_completion_message(message) {
                continue;
            }

            let segment = &messages[segment_start..=idx];
            let user_query = segment
                .iter()
                .filter(|message| {
                    message.role == "user"
                        && message.step_type.as_deref() != Some("observe")
                        && !message.message.trim().is_empty()
                })
                .map(|message| message.message.trim())
                .collect::<Vec<_>>()
                .join("\n---\n");

            let result_summary = segment
                .iter()
                .rev()
                .find(|message| message.role == "assistant")
                .map(IntelligenceManager::extract_completion_summary)
                .unwrap_or_default();

            if !user_query.trim().is_empty() || !result_summary.trim().is_empty() {
                tasks.push(format!(
                    "<task>\n<task_index>{}</task_index>\n<user_query>{}</user_query>\n<result_summary>{}</result_summary>\n</task>",
                    idx,
                    Self::escape_xml_text(&user_query),
                    Self::escape_xml_text(result_summary.trim())
                ));
            }

            segment_start = idx + 1;
        }

        if tasks.is_empty() {
            "None".to_string()
        } else {
            tasks.join("\n")
        }
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
            .and_then(|value| value.as_str())
            .map(|tool_name| tool_name == TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY)
            .unwrap_or(false);
        if !is_completion_tool {
            return false;
        }

        let execution_status = meta
            .get("execution_status")
            .and_then(|value| value.as_str())
            .unwrap_or("completed");
        let approval_status = meta
            .get("approval_status")
            .and_then(|value| value.as_str())
            .unwrap_or("approved");

        execution_status == "completed"
            && approval_status != "pending"
            && approval_status != "rejected"
    }

    fn escape_xml_text(value: &str) -> String {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
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

            return String::new();
        }

        trimmed.to_string()
    }

    fn validate_summary_result(normalized: &str) -> Result<String, String> {
        let trimmed = normalized.trim();
        if trimmed.is_empty() {
            return Err("empty response".to_string());
        }

        if !trimmed.starts_with("<state_snapshot>") || !trimmed.ends_with("</state_snapshot>") {
            return Err(
                "response must contain exactly one top-level <state_snapshot>...</state_snapshot> block"
                    .to_string(),
            );
        }

        let required_tags = [
            "<overall_goal>",
            "<prev_tasks>",
            "<key_knowledge>",
            "<error_log>",
            "<file_system_state>",
            "<recent_actions>",
            "<task_state>",
        ];

        let missing_tags = required_tags
            .iter()
            .copied()
            .filter(|tag| !trimmed.contains(tag))
            .collect::<Vec<_>>();
        if !missing_tags.is_empty() {
            return Err(format!(
                "missing required tags: {}",
                missing_tags.join(", ")
            ));
        }

        Ok(trimmed.to_string())
    }

    fn build_retry_instruction(validation_error: &str, completed_tasks: &str) -> String {
        let prev_tasks_requirement = if completed_tasks.trim() == "None" {
            "If no completed tasks exist, keep <prev_tasks> and set it to None or an empty placeholder."
        } else {
            "Completed tasks exist. You MUST preserve them inside <prev_tasks>. Keep the tag even if you need to compress older tasks into <brief> entries."
        };

        format!(
            "\n\n<SYSTEM_REMINDER>Your previous compression reply was invalid. Reason: {}. Return exactly one non-empty <state_snapshot>...</state_snapshot> XML block and nothing else. You MUST include all required tags every time, even when a section has no information: <overall_goal>, <prev_tasks>, <key_knowledge>, <error_log>, <file_system_state>, <recent_actions>, <task_state>. If a section has no content, write `None` or a short empty-state note inside the tag. Do NOT return JSON, reasoning-only text, markdown fences, or explanations. {}</SYSTEM_REMINDER>",
            validation_error,
            prev_tasks_requirement
        )
    }

    fn preview_for_log(value: &str, max_chars: usize) -> String {
        let mut text: String = value.chars().take(max_chars).collect();
        if value.chars().count() > max_chars {
            text.push_str("...");
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::ContextCompressor;
    use crate::db::WorkflowMessage;

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
    fn normalize_summary_result_rejects_reasoning_only_json() {
        let raw = r#"{"content":"","reasoning":"Let me analyze this first"}"#;
        let normalized = ContextCompressor::normalize_summary_result(raw);
        assert!(normalized.is_empty());
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

    #[test]
    fn validate_summary_result_requires_full_state_snapshot_shape() {
        let invalid = "<state_snapshot><overall_goal>x</overall_goal></state_snapshot>";
        assert!(ContextCompressor::validate_summary_result(invalid).is_err());

        let valid = "<state_snapshot><overall_goal>x</overall_goal><prev_tasks>None</prev_tasks><key_knowledge>a</key_knowledge><error_log>b</error_log><file_system_state>c</file_system_state><recent_actions>d</recent_actions><task_state>e</task_state></state_snapshot>";
        assert!(ContextCompressor::validate_summary_result(valid).is_ok());
    }

    #[test]
    fn render_completed_tasks_keeps_each_finished_segment_summary() {
        let messages = vec![
            WorkflowMessage {
                id: Some(1),
                session_id: "s".to_string(),
                role: "user".to_string(),
                message: "First question".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: None,
                attached_context: None,
                step_type: None,
                step_index: 0,
                is_error: false,
                error_type: None,
                created_at: None,
            },
            WorkflowMessage {
                id: Some(2),
                session_id: "s".to_string(),
                role: "assistant".to_string(),
                message: "First answer".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(serde_json::json!({ "message_kind": "completion_report" })),
                attached_context: None,
                step_type: Some("think".to_string()),
                step_index: 1,
                is_error: false,
                error_type: None,
                created_at: None,
            },
            WorkflowMessage {
                id: Some(3),
                session_id: "s".to_string(),
                role: "tool".to_string(),
                message: "Finished".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(
                    serde_json::json!({ "tool_name": "complete_workflow_with_summary" }),
                ),
                attached_context: None,
                step_type: Some("observe".to_string()),
                step_index: 2,
                is_error: false,
                error_type: None,
                created_at: None,
            },
            WorkflowMessage {
                id: Some(4),
                session_id: "s".to_string(),
                role: "user".to_string(),
                message: "Second question".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 2,
                source_event_type: None,
                metadata: None,
                attached_context: None,
                step_type: None,
                step_index: 3,
                is_error: false,
                error_type: None,
                created_at: None,
            },
            WorkflowMessage {
                id: Some(5),
                session_id: "s".to_string(),
                role: "assistant".to_string(),
                message: "Second answer".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 2,
                source_event_type: None,
                metadata: Some(serde_json::json!({ "message_kind": "completion_report" })),
                attached_context: None,
                step_type: Some("think".to_string()),
                step_index: 4,
                is_error: false,
                error_type: None,
                created_at: None,
            },
            WorkflowMessage {
                id: Some(6),
                session_id: "s".to_string(),
                role: "tool".to_string(),
                message: "Finished".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 2,
                source_event_type: None,
                metadata: Some(
                    serde_json::json!({ "tool_name": "complete_workflow_with_summary" }),
                ),
                attached_context: None,
                step_type: Some("observe".to_string()),
                step_index: 5,
                is_error: false,
                error_type: None,
                created_at: None,
            },
        ];

        let rendered = ContextCompressor::render_completed_tasks(&messages);
        assert!(rendered.contains("<user_query>First question</user_query>"));
        assert!(rendered.contains("<result_summary>First answer</result_summary>"));
        assert!(rendered.contains("<user_query>Second question</user_query>"));
        assert!(rendered.contains("<result_summary>Second answer</result_summary>"));
    }

    #[test]
    fn compression_skips_approved_submit_plan_observation_messages() {
        let message = WorkflowMessage {
            id: Some(1),
            session_id: "s".to_string(),
            role: "tool".to_string(),
            message: "# Approved Plan\n\nplan body\n\n<SYSTEM_REMINDER>approved</SYSTEM_REMINDER>"
                .to_string(),
            reasoning: None,
            message_kind: "message".to_string(),
            message_subtype: None,
            segment_id: 1,
            source_event_type: None,
            metadata: Some(serde_json::json!({
                "tool_name": "submit_plan",
                "approval_status": "approved"
            })),
            attached_context: None,
            step_type: Some("observe".to_string()),
            step_index: 0,
            is_error: false,
            error_type: None,
            created_at: None,
        };

        assert!(ContextCompressor::should_skip_message_for_compression(
            &message
        ));
    }
}
