use crate::ai::chat::openai::OpenAIChat;
use crate::ai::error::AiError;
use crate::ai::interaction::chat_completion::{AiChatEnum, ChatState};
use crate::ai::traits::chat::ChatMetadata;
use crate::db::WorkflowMessage;
use crate::tools::TOOL_ASK_USER;
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
                let keep = Self::should_keep_message_content_for_compression(m, &content);
                keep.then(|| serde_json::json!({ "role": m.role, "content": content }))
            })
            .collect();

        if purified_history.is_empty() {
            return Ok("No meaningful progress to compress.".to_string());
        }

        let completed_tasks = Self::render_completed_tasks(incremental_messages);
        let completed_history_only =
            Self::compression_scope_contains_only_completed_history(incremental_messages);

        // 4. Strategic Summary (LLM Call)
        let user_prompt = if last_summary_idx.is_some() {
            if completed_history_only {
                "Please update the existing state snapshot with the new completed-task archive information provided above. This compression slice contains only already-completed tasks and does NOT contain the currently active user request or active workspace. Do NOT preserve or invent an `overall_goal` from older work unless this slice itself contains a still-active cross-task objective. For this archive slice, `task_state` should describe an archive/no-active-task state rather than the live current request. Output only the updated JSON object."
            } else {
                "Please update the existing state snapshot with the new information provided above. Output only the updated JSON object."
            }
        } else {
            if completed_history_only {
                "Create the first completed-task archive snapshot JSON object based on the conversation history provided above. This compression slice contains only already-completed tasks and does NOT contain the currently active user request or active workspace. Do NOT add an `overall_goal` unless this slice itself contains a still-active cross-task objective. `task_state` should describe an archive/no-active-task state rather than a live current request."
            } else {
                "Create the first state snapshot JSON object based on the conversation history provided above."
            }
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

        let mut sanitized = Self::strip_system_reminders(content);
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

    fn strip_system_reminders(content: &str) -> String {
        let mut sanitized = content.to_string();

        loop {
            let Some(start) = sanitized.find("<SYSTEM_REMINDER>") else {
                break;
            };
            let Some(end) = sanitized[start..].find("</SYSTEM_REMINDER>") else {
                sanitized.truncate(start);
                break;
            };
            let end_idx = start + end + "</SYSTEM_REMINDER>".len();
            sanitized.replace_range(start..end_idx, "");
        }

        sanitized
    }

    fn should_skip_message_for_compression(message: &WorkflowMessage) -> bool {
        let Some(meta) = message.metadata.as_ref() else {
            return false;
        };

        let tool_name = meta.get("tool_name").and_then(|value| value.as_str());
        let approval_status = meta.get("approval_status").and_then(|value| value.as_str());

        tool_name == Some("submit_plan") && approval_status == Some("approved")
    }

    fn should_keep_message_content_for_compression(
        message: &WorkflowMessage,
        content: &str,
    ) -> bool {
        if content.is_empty() {
            return false;
        }

        if Self::is_successful_completion_message(message) {
            return false;
        }

        let Some(meta) = message.metadata.as_ref() else {
            return true;
        };

        let tool_name = meta.get("tool_name").and_then(|value| value.as_str());
        let approval_status = meta.get("approval_status").and_then(|value| value.as_str());
        let execution_status = meta
            .get("execution_status")
            .and_then(|value| value.as_str());

        if tool_name == Some(TOOL_ASK_USER) {
            return false;
        }

        if matches!(approval_status, Some("pending" | "rejected")) {
            return false;
        }

        if matches!(
            execution_status,
            Some("pending_approval" | "approval_submitted" | "running" | "waiting")
        ) {
            return false;
        }

        true
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

    fn compression_scope_contains_only_completed_history(messages: &[WorkflowMessage]) -> bool {
        let latest_completion_idx = messages
            .iter()
            .rposition(Self::is_successful_completion_message);

        let Some(latest_completion_idx) = latest_completion_idx else {
            return false;
        };

        !messages
            .iter()
            .skip(latest_completion_idx + 1)
            .any(|message| message.message_kind != "summary")
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
            if let Some(content) = parsed.get("content") {
                match content {
                    serde_json::Value::String(text) => {
                        let content = text.trim();
                        if !content.is_empty() {
                            return content.to_string();
                        }
                    }
                    serde_json::Value::Object(_) => {
                        if let Ok(text) = serde_json::to_string_pretty(content) {
                            return text;
                        }
                    }
                    _ => {}
                }
            }

            if parsed.is_object() {
                return serde_json::to_string_pretty(&parsed).unwrap_or_default();
            }

            return trimmed.to_string();
        }

        trimmed.to_string()
    }

    fn validate_summary_result(normalized: &str) -> Result<String, String> {
        let trimmed = normalized.trim();
        if trimmed.is_empty() {
            return Err("empty response".to_string());
        }

        if trimmed.starts_with("<state_snapshot>") && trimmed.ends_with("</state_snapshot>") {
            let required_tags = [
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

            return Ok(trimmed.to_string());
        }

        let parsed = serde_json::from_str::<serde_json::Value>(trimmed)
            .map_err(|_| "response must be a single JSON object".to_string())?;
        let object = parsed
            .as_object()
            .ok_or_else(|| "response must be a single JSON object".to_string())?;

        let required_keys = [
            "prev_tasks",
            "key_knowledge",
            "error_log",
            "file_system_state",
            "recent_actions",
            "task_state",
        ];

        let missing_keys = required_keys
            .iter()
            .copied()
            .filter(|key| !object.contains_key(*key))
            .collect::<Vec<_>>();
        if !missing_keys.is_empty() {
            return Err(format!(
                "missing required keys: {}",
                missing_keys.join(", ")
            ));
        }

        if object.contains_key("overall_goal") {
            Self::validate_non_empty_string_field(&parsed["overall_goal"], "overall_goal")?;
        }
        Self::validate_prev_tasks(&parsed["prev_tasks"])?;
        Self::validate_string_array(&parsed["key_knowledge"], "key_knowledge")?;
        Self::validate_string_array(&parsed["error_log"], "error_log")?;
        Self::validate_string_array(&parsed["file_system_state"], "file_system_state")?;
        Self::validate_string_array(&parsed["recent_actions"], "recent_actions")?;
        Self::validate_task_state(&parsed["task_state"])?;

        serde_json::to_string_pretty(&parsed)
            .map_err(|error| format!("failed to normalize snapshot json: {}", error))
    }

    fn validate_non_empty_string_field(
        value: &serde_json::Value,
        field: &str,
    ) -> Result<(), String> {
        let text = value
            .as_str()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .ok_or_else(|| format!("{} must be a non-empty string", field))?;
        if text.eq_ignore_ascii_case("null") {
            return Err(format!("{} must not be the string 'null'", field));
        }
        Ok(())
    }

    fn validate_string_array(value: &serde_json::Value, field: &str) -> Result<(), String> {
        let items = value
            .as_array()
            .ok_or_else(|| format!("{} must be an array of strings", field))?;
        for (index, item) in items.iter().enumerate() {
            let text = item
                .as_str()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .ok_or_else(|| format!("{}[{}] must be a non-empty string", field, index))?;
            if text.eq_ignore_ascii_case("null") {
                return Err(format!("{}[{}] must not be 'null'", field, index));
            }
        }
        Ok(())
    }

    fn validate_prev_tasks(value: &serde_json::Value) -> Result<(), String> {
        let tasks = value
            .as_array()
            .ok_or_else(|| "prev_tasks must be an array".to_string())?;
        for (index, task) in tasks.iter().enumerate() {
            let obj = task
                .as_object()
                .ok_or_else(|| format!("prev_tasks[{}] must be an object", index))?;
            if !obj
                .get("task_index")
                .map(|value| value.is_i64() || value.is_u64())
                .unwrap_or(false)
            {
                return Err(format!("prev_tasks[{}].task_index must be a number", index));
            }

            let has_brief = obj
                .get("brief")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .is_some();
            let has_detailed = obj
                .get("user_query")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .is_some()
                && obj
                    .get("result_summary")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .is_some();

            if !has_brief && !has_detailed {
                return Err(format!(
                    "prev_tasks[{}] must contain either brief or user_query + result_summary",
                    index
                ));
            }
        }
        Ok(())
    }

    fn validate_task_state(value: &serde_json::Value) -> Result<(), String> {
        let task_state = value
            .as_object()
            .ok_or_else(|| "task_state must be an object".to_string())?;
        for field in ["status", "current_focus"] {
            Self::validate_non_empty_string_field(
                task_state
                    .get(field)
                    .ok_or_else(|| format!("task_state.{} is required", field))?,
                &format!("task_state.{}", field),
            )?;
        }
        for field in ["next_steps", "open_questions", "blockers"] {
            Self::validate_string_array(
                task_state
                    .get(field)
                    .ok_or_else(|| format!("task_state.{} is required", field))?,
                &format!("task_state.{}", field),
            )?;
        }

        let todos = task_state
            .get("todos")
            .and_then(|value| value.as_array())
            .ok_or_else(|| "task_state.todos must be an array".to_string())?;
        for (index, todo) in todos.iter().enumerate() {
            let todo_obj = todo
                .as_object()
                .ok_or_else(|| format!("task_state.todos[{}] must be an object", index))?;
            for field in ["text", "status"] {
                Self::validate_non_empty_string_field(
                    todo_obj.get(field).ok_or_else(|| {
                        format!("task_state.todos[{}].{} is required", index, field)
                    })?,
                    &format!("task_state.todos[{}].{}", index, field),
                )?;
            }
        }

        Ok(())
    }

    fn build_retry_instruction(validation_error: &str, completed_tasks: &str) -> String {
        let prev_tasks_requirement = if completed_tasks.trim() == "None" {
            "If no completed tasks exist, keep `prev_tasks` and set it to an empty array or a short empty-state placeholder."
        } else {
            "Completed tasks exist. You MUST preserve them inside `prev_tasks`. Keep the key even if you need to compress older tasks into `brief` entries."
        };

        format!(
            "\n\n<SYSTEM_REMINDER>Your previous compression reply was invalid. Reason: {}. Return exactly one non-empty JSON object and nothing else. You MUST include all required keys every time, even when a section has no information: prev_tasks, key_knowledge, error_log, file_system_state, recent_actions, task_state. `overall_goal` is optional and should be omitted for completed-history archive slices unless a still-active cross-task objective truly remains in the compressed scope. `prev_tasks` must be an array of objects with task_index plus either brief or user_query + result_summary. `task_state` must be an object with status, current_focus, next_steps, open_questions, blockers, and todos. The lists key_knowledge, error_log, file_system_state, and recent_actions must be arrays of strings. Do NOT return XML, reasoning-only text, markdown fences, or explanations. {}</SYSTEM_REMINDER>",
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
    use crate::{db::WorkflowMessage, workflow::react::constants::TASK_FINISHED};

    #[test]
    fn normalize_summary_result_extracts_content_field_from_json() {
        let raw = r#"{"content":{"overall_goal":"goal","prev_tasks":[],"key_knowledge":[],"error_log":[],"file_system_state":[],"recent_actions":[],"task_state":"active"},"reasoning":"ignored"}"#;
        let normalized = ContextCompressor::normalize_summary_result(raw);
        assert!(normalized.contains("\"overall_goal\": \"goal\""));
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
    fn sanitize_message_content_for_compression_removes_system_reminders() {
        let sanitized = ContextCompressor::sanitize_message_content_for_compression(
            "Primary result\n<SYSTEM_REMINDER>Runtime hint</SYSTEM_REMINDER>\nFollow-up fact",
        );
        assert_eq!(sanitized, "Primary result Follow-up fact");
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
    fn validate_summary_result_accepts_json_snapshot_shape() {
        let valid = r#"{"overall_goal":"x","prev_tasks":[],"key_knowledge":[],"error_log":[],"file_system_state":[],"recent_actions":[],"task_state":{"status":"in_progress","current_focus":"focus","next_steps":["step"],"open_questions":[],"blockers":[],"todos":[{"text":"todo","status":"in_progress"}]}}"#;
        assert!(ContextCompressor::validate_summary_result(valid).is_ok());
    }

    #[test]
    fn validate_summary_result_accepts_json_snapshot_without_overall_goal() {
        let valid = r#"{"prev_tasks":[],"key_knowledge":[],"error_log":[],"file_system_state":[],"recent_actions":[],"task_state":{"status":"completed_archive","current_focus":"No active task in compressed segment","next_steps":[],"open_questions":[],"blockers":[],"todos":[]}}"#;
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
                message: TASK_FINISHED.to_string(),
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
                message: TASK_FINISHED.to_string(),
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

    #[test]
    fn compression_filters_ask_user_wait_state_by_tool_metadata() {
        let message = WorkflowMessage {
            id: Some(1),
            session_id: "s".to_string(),
            role: "tool".to_string(),
            message: "[{\"title\":\"Need a choice\",\"options\":[\"A\",\"B\"]}]".to_string(),
            reasoning: None,
            message_kind: "message".to_string(),
            message_subtype: None,
            segment_id: 1,
            source_event_type: None,
            metadata: Some(serde_json::json!({
                "tool_name": "ask_user",
                "execution_status": "waiting"
            })),
            attached_context: None,
            step_type: Some("observe".to_string()),
            step_index: 0,
            is_error: false,
            error_type: None,
            created_at: None,
        };

        assert!(
            !ContextCompressor::should_keep_message_content_for_compression(
                &message,
                &message.message
            )
        );
    }

    #[test]
    fn compression_filters_successful_completion_by_metadata_not_message_text() {
        let message = WorkflowMessage {
            id: Some(1),
            session_id: "s".to_string(),
            role: "tool".to_string(),
            message: "Any completion text".to_string(),
            reasoning: None,
            message_kind: "message".to_string(),
            message_subtype: None,
            segment_id: 1,
            source_event_type: None,
            metadata: Some(serde_json::json!({
                "tool_name": "complete_workflow_with_summary",
                "execution_status": "completed",
                "approval_status": "approved"
            })),
            attached_context: None,
            step_type: Some("observe".to_string()),
            step_index: 0,
            is_error: false,
            error_type: None,
            created_at: None,
        };

        assert!(
            !ContextCompressor::should_keep_message_content_for_compression(
                &message,
                &message.message
            )
        );
    }
}
