use crate::ai::chat::openai::OpenAIChat;
use crate::ai::interaction::chat_completion::{AiChatEnum, ChatState};
use crate::ai::traits::chat::{ChatMetadata, MessageType, WorkflowUsageAttribution};
use crate::ccproxy::utils::token_estimator::estimate_tokens;
use crate::db::WorkflowMessage;
use crate::tools::TOOL_COMPLETE_WORKFLOW;
use crate::workflow::react::context::ContextManager;
use crate::workflow::react::error::WorkflowEngineError;

use std::sync::Arc;
use tokio::time::{sleep, Duration};

/// IntelligenceManager handles lightweight AI helper tasks such as workflow
/// title generation, smart tool-approval review, and user input language
/// detection.
pub struct IntelligenceManager {
    pub session_id: String,
    pub chat_state: Arc<ChatState>,
    pub active_provider_id: i64,
    pub active_model_name: String,
    pub utility_provider_id: i64,
    pub utility_model_name: String,
    pub lite_provider_id: i64,
    pub lite_model_name: String,
    pub approval_provider_id: i64,
    pub approval_model_name: String,
    pub workflow_task_run_id: String,
    pub root_session_id: String,
    pub root_task_run_id: String,
}

#[derive(Debug, Clone)]
pub struct ToolApprovalReview {
    pub approved: bool,
    pub reason: String,
    pub risk_level: String,
}

impl IntelligenceManager {
    fn parse_tool_approval_review(result: &str) -> ToolApprovalReview {
        let invalid_review = || ToolApprovalReview {
            approved: false,
            reason: "Approval reviewer returned invalid structured output; manual review required"
                .to_string(),
            risk_level: "medium".to_string(),
        };

        let Ok(review_json) = serde_json::from_str::<serde_json::Value>(
            crate::libs::util::format_json_str(result).as_str(),
        ) else {
            return invalid_review();
        };
        let Some(review) = review_json.as_object() else {
            return invalid_review();
        };
        let Some(approved) = review.get("approved").and_then(|value| value.as_bool()) else {
            return invalid_review();
        };
        let Some(reason) = review
            .get("reason")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return invalid_review();
        };
        let Some(risk_level) = review
            .get("risk_level")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| matches!(*value, "low" | "medium" | "high"))
        else {
            return invalid_review();
        };

        ToolApprovalReview {
            approved: approved && risk_level == "low",
            reason: reason.to_string(),
            risk_level: risk_level.to_string(),
        }
    }

    pub(crate) fn extract_completion_summary(message: &WorkflowMessage) -> String {
        let visible_content = message.message.trim();
        let tool_summary = message
            .metadata
            .as_ref()
            .and_then(|meta| meta.get("tool_calls"))
            .and_then(|tool_calls| tool_calls.as_array())
            .and_then(|tool_calls| {
                tool_calls.iter().rev().find_map(|call| {
                    let tool_name = call
                        .get("name")
                        .or_else(|| {
                            call.get("function")
                                .and_then(|function| function.get("name"))
                        })
                        .and_then(|value| value.as_str())
                        .unwrap_or_default();
                    if tool_name != TOOL_COMPLETE_WORKFLOW {
                        return None;
                    }

                    let args_value = call.get("arguments").or_else(|| {
                        call.get("function")
                            .and_then(|function| function.get("arguments"))
                    })?;
                    let parsed_args = if let Some(args_text) = args_value.as_str() {
                        serde_json::from_str::<serde_json::Value>(args_text).ok()
                    } else {
                        Some(args_value.clone())
                    }?;
                    parsed_args
                        .get("summary")
                        .and_then(|summary| summary.as_str())
                        .map(str::trim)
                        .filter(|summary| !summary.is_empty())
                        .map(str::to_string)
                })
            });

        match (visible_content.is_empty(), tool_summary) {
            (true, Some(summary)) => summary,
            (false, Some(summary)) => format!("{}\n\n{}", visible_content, summary),
            (false, None) => visible_content.to_string(),
            (true, None) => String::new(),
        }
    }

    fn truncate_text(value: &str, max_chars: usize) -> String {
        let mut text: String = value.chars().take(max_chars).collect();
        if value.chars().count() > max_chars {
            text.push_str("...");
        }
        text
    }

    fn format_message_excerpt(message: &WorkflowMessage, max_chars: usize) -> String {
        let role = &message.role;
        let step_type = message.step_type.as_deref().unwrap_or_default();
        let tool_name = message
            .metadata
            .as_ref()
            .and_then(|meta| meta.get("tool_name"))
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let mut content = message.message.clone();
        if !message
            .reasoning
            .as_deref()
            .unwrap_or_default()
            .trim()
            .is_empty()
        {
            content.push_str("\n<reasoning>\n");
            content.push_str(message.reasoning.as_deref().unwrap_or_default());
            content.push_str("\n</reasoning>");
        }

        format!(
            "<message role=\"{}\" step_type=\"{}\" tool_name=\"{}\">{}\n</message>",
            role,
            step_type,
            tool_name,
            Self::truncate_text(&content, max_chars).replace('\n', "\n    ")
        )
    }

    fn format_recent_messages(
        messages: &[WorkflowMessage],
        limit: usize,
        max_chars: usize,
    ) -> String {
        let start = messages.len().saturating_sub(limit);
        messages[start..]
            .iter()
            .map(|message| Self::format_message_excerpt(message, max_chars))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn sanitize_generated_title(raw: &str) -> String {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return String::new();
        }

        if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(content) = value.get("content").and_then(|v| v.as_str()) {
                return content
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .trim()
                    .to_string();
            }
        }

        trimmed
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .trim()
            .to_string()
    }

    fn fallback_workflow_title(user_query: &str) -> String {
        let trimmed = user_query.trim();
        if trimmed.is_empty() {
            return String::new();
        }

        let max_chars = 50;
        let mut title: String = trimmed.chars().take(max_chars).collect();
        if trimmed.chars().count() > max_chars {
            title.push_str("...");
        }
        title
    }

    pub fn new(
        session_id: String,
        chat_state: Arc<ChatState>,
        active_provider_id: i64,
        active_model_name: String,
        lite_provider_id: i64,
        lite_model_name: String,
        workflow_task_run_id: String,
        root_session_id: String,
        root_task_run_id: String,
    ) -> Self {
        Self {
            session_id,
            chat_state,
            active_provider_id,
            active_model_name: active_model_name.clone(),
            utility_provider_id: active_provider_id,
            utility_model_name: active_model_name.clone(),
            lite_provider_id,
            lite_model_name,
            approval_provider_id: active_provider_id,
            approval_model_name: active_model_name,
            workflow_task_run_id,
            root_session_id,
            root_task_run_id,
        }
    }

    /// Resolves the helper model for lightweight tasks (title generation,
    /// language detection): dedicated lite model first, then the utility
    /// model, then the active model.
    fn lite_model_selection(&self) -> (i64, String) {
        if !self.lite_model_name.trim().is_empty() {
            (self.lite_provider_id, self.lite_model_name.clone())
        } else if !self.utility_model_name.trim().is_empty() {
            (self.utility_provider_id, self.utility_model_name.clone())
        } else {
            (self.active_provider_id, self.active_model_name.clone())
        }
    }

    /// Reviews a proposed tool call in smart approval mode.
    pub async fn review_tool_approval(
        &self,
        context: &ContextManager,
        workspace_context: &str,
        tool_name: &str,
        tool_category: &str,
        tool_scope: &str,
        tool_description: &str,
        tool_args: &serde_json::Value,
        assistant_text: &str,
    ) -> Result<ToolApprovalReview, WorkflowEngineError> {
        log::info!(
            "IntelligenceManager {}: Reviewing tool approval for '{}'",
            self.session_id,
            tool_name
        );

        let current_goal = context.current_user_request_since_last_completion();
        let work_messages = context.messages_since_last_completion();
        let recent_messages = Self::format_recent_messages(&work_messages, 10, 700);
        let args_text =
            serde_json::to_string_pretty(tool_args).unwrap_or_else(|_| "{}".to_string());

        let system_prompt = crate::workflow::react::prompts::TOOL_APPROVAL_REVIEW_PROMPT;
        let user_prompt = format!(
            "<approval_context>\n\
<workspace_context>\n{}\n</workspace_context>\n\
<current_user_goal>\n{}\n</current_user_goal>\n\
<assistant_intent>\n{}\n</assistant_intent>\n\
<tool_call>\n\
  <name>{}</name>\n\
  <category>{}</category>\n\
  <scope>{}</scope>\n\
  <description>{}</description>\n\
  <arguments>\n{}\n</arguments>\n</tool_call>\n\
<recent_work>\n{}\n</recent_work>\n</approval_context>",
            workspace_context,
            current_goal,
            Self::truncate_text(assistant_text, 1500),
            tool_name,
            tool_category,
            tool_scope,
            tool_description,
            args_text,
            recent_messages
        );

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

        let chat_interface = {
            let mut chats_guard = self.chat_state.chats.lock().await;
            chats_guard
                .entry(crate::ccproxy::ChatProtocol::OpenAI)
                .or_default()
                .entry(self.session_id.clone() + "_approval_reviewer")
                .or_insert_with(|| crate::create_chat!(self.chat_state.main_store))
                .clone()
        };

        let (tx, mut rx) = tokio::sync::mpsc::channel(100);
        let session_id_review = self.session_id.clone() + "_approval_reviewer";
        let provider_id = self.approval_provider_id;
        let model_name = self.approval_model_name.clone();
        let workflow_usage_attribution = WorkflowUsageAttribution {
            workflow_session_id: self.session_id.clone(),
            workflow_task_run_id: self.workflow_task_run_id.clone(),
            workflow_segment_id: context.current_segment_id,
            root_session_id: self.root_session_id.clone(),
            root_task_run_id: self.root_task_run_id.clone(),
            request_kind: "smart_approval".to_string(),
        };

        tokio::spawn(async move {
            if let Err(e) = chat_interface
                .chat(
                    provider_id,
                    &model_name,
                    session_id_review,
                    messages,
                    None,
                    Some(ChatMetadata {
                        reasoning: Some(true),
                        workflow_usage_attribution: Some(workflow_usage_attribution),
                        ..Default::default()
                    }),
                    move |chunk| {
                        let _ = tx.try_send(chunk);
                    },
                )
                .await
            {
                log::error!("IntelligenceManager approval review task failed: {}", e);
            }
        });

        let mut result = String::new();
        while let Some(chunk) = rx.recv().await {
            match chunk.r#type {
                MessageType::Text => result.push_str(&chunk.chunk),
                MessageType::Finished => break,
                MessageType::Error => {
                    log::error!(
                        "IntelligenceManager approval review LLM error: {}",
                        chunk.chunk
                    );
                    return Err(WorkflowEngineError::General(
                        "Approval review model failed".to_string(),
                    ));
                }
                _ => {}
            }
        }

        Ok(Self::parse_tool_approval_review(result.trim()))
    }

    /// Generates a concise title for the workflow session based on the user's initial query.
    pub async fn generate_workflow_title(
        &self,
        user_query: &str,
    ) -> Result<String, WorkflowEngineError> {
        let (provider_id, model_name) = {
            let store = self.chat_state.main_store.as_ref();
            let gen_model_config: serde_json::Value =
                store.get_config("conversation_title_gen_model", serde_json::json!({}));

            let global_title_model = if gen_model_config.is_object() {
                match (
                    gen_model_config["id"].as_i64(),
                    gen_model_config["model"].as_str(),
                ) {
                    (Some(provider_id), Some(model_name))
                        if provider_id > 0 && !model_name.trim().is_empty() =>
                    {
                        Some((provider_id, model_name.to_string()))
                    }
                    _ => None,
                }
            } else {
                None
            };

            // Priority: dedicated lite model > global title model > utility >
            // active. The lite fields are only populated when explicitly
            // configured, so an empty name means "not configured".
            if !self.lite_model_name.trim().is_empty() {
                (self.lite_provider_id, self.lite_model_name.clone())
            } else if let Some(global) = global_title_model {
                global
            } else {
                self.lite_model_selection()
            }
        };

        let system_prompt = "You generate concise titles for workflow tasks, not chat conversations. \
                             Focus on the concrete task being worked on: the target module/file/system, the main issue, bug, feature, investigation, or fix. \
                             Prefer titles that describe the actual work item rather than the discussion. \
                             Keep the title under 10 words, preferably 3-5 words. Use the same language as the user's input. \
                             Return only the title text with no explanation, no reasoning, no JSON, and no quotes.";
        let user_prompt = format!(
            "Generate a workflow task title for this initial request. Emphasize the main task or issue being handled.\n\n{}",
            user_query
        );

        let messages = vec![
            serde_json::json!({ "role": "system", "content": system_prompt }),
            serde_json::json!({ "role": "user", "content": user_prompt }),
        ];

        let chat_interface = {
            let mut chats_guard = self.chat_state.chats.lock().await;
            chats_guard
                .entry(crate::ccproxy::ChatProtocol::OpenAI)
                .or_default()
                .entry(self.session_id.clone() + "_title_gen")
                .or_insert_with(|| crate::create_chat!(self.chat_state.main_store))
                .clone()
        };

        let mut generated_title = String::new();
        let mut last_error: Option<WorkflowEngineError> = None;

        for attempt in 0..=1 {
            if attempt > 0 {
                sleep(Duration::from_secs(2)).await;
            }
            let session_id_title = format!("{}_title_gen_{}", self.session_id, attempt + 1);
            match chat_interface
                .chat(
                    provider_id,
                    &model_name,
                    session_id_title,
                    messages.clone(),
                    None,
                    Some(ChatMetadata {
                        workflow_usage_attribution: Some(WorkflowUsageAttribution {
                            workflow_session_id: self.session_id.clone(),
                            workflow_task_run_id: self.workflow_task_run_id.clone(),
                            workflow_segment_id: 1,
                            root_session_id: self.root_session_id.clone(),
                            root_task_run_id: self.root_task_run_id.clone(),
                            request_kind: "title_generation".to_string(),
                        }),
                        ..Default::default()
                    }),
                    move |_chunk| {},
                )
                .await
            {
                Ok(title) => {
                    generated_title = Self::sanitize_generated_title(&title);
                    if !generated_title.is_empty() {
                        break;
                    }

                    log::warn!(
                        "[Workflow][session={}][title] Empty title generated on attempt {}/2",
                        self.session_id,
                        attempt + 1
                    );
                }
                Err(error) => {
                    log::warn!(
                        "[Workflow][session={}][title] Title generation failed on attempt {}/2: {}",
                        self.session_id,
                        attempt + 1,
                        error
                    );
                    last_error = Some(WorkflowEngineError::Ai(error));
                }
            }
        }

        let final_title = if generated_title.is_empty() {
            let fallback_title = Self::fallback_workflow_title(user_query);
            if !fallback_title.is_empty() {
                log::warn!(
                    "[Workflow][session={}][title] Falling back to truncated user query after title generation failure",
                    self.session_id
                );
                fallback_title
            } else if let Some(error) = last_error {
                return Err(error);
            } else {
                String::new()
            }
        } else {
            generated_title
        };

        if !final_title.is_empty() {
            let store = self.chat_state.main_store.as_ref();
            let _ = store.update_workflow_title(&self.session_id, &final_title);
        }

        Ok(final_title)
    }

    /// Detects the natural language of the user's raw input with the lite
    /// model. This is a one-shot prerequisite step, never repeated per LLM
    /// call. Failed or empty detections are retried with exponential backoff
    /// (3 attempts total); when all attempts fail it returns `None` and the
    /// caller continues without a language directive.
    /// `segment_id` is the active workflow segment used for usage attribution.
    pub async fn detect_input_language(
        &self,
        user_input: &str,
        max_input_tokens: usize,
        segment_id: i32,
    ) -> Option<String> {
        const MAX_DETECTION_ATTEMPTS: u32 = 3;

        let trimmed = user_input.trim();
        if trimmed.is_empty() {
            return None;
        }

        let (provider_id, model_name) = self.lite_model_selection();
        if model_name.trim().is_empty() {
            return None;
        }
        let bounded_input = Self::truncate_to_token_budget(trimmed, max_input_tokens);

        let messages = vec![
            serde_json::json!({
                "role": "system",
                "content": crate::workflow::react::prompts::LANGUAGE_DETECTION_SYSTEM_PROMPT
            }),
            serde_json::json!({
                "role": "user",
                "content": bounded_input
            }),
        ];

        let chat_interface = {
            let mut chats_guard = self.chat_state.chats.lock().await;
            chats_guard
                .entry(crate::ccproxy::ChatProtocol::OpenAI)
                .or_default()
                .entry(self.session_id.clone() + "_language_detect")
                .or_insert_with(|| crate::create_chat!(self.chat_state.main_store))
                .clone()
        };

        let mut last_error: Option<String> = None;
        for attempt in 1..=MAX_DETECTION_ATTEMPTS {
            if attempt > 1 {
                let wait_secs = 2u64.pow(attempt - 1);
                log::info!(
                    "[Workflow][session={}][language] Retrying language detection in {}s (attempt {}/{})",
                    self.session_id,
                    wait_secs,
                    attempt,
                    MAX_DETECTION_ATTEMPTS
                );
                sleep(Duration::from_secs(wait_secs)).await;
            }

            let session_id_detect = format!("{}_language_detect_{}", self.session_id, attempt);
            match chat_interface
                .chat(
                    provider_id,
                    &model_name,
                    session_id_detect,
                    messages.clone(),
                    None,
                    Some(ChatMetadata {
                        stream: Some(false),
                        workflow_usage_attribution: Some(WorkflowUsageAttribution {
                            workflow_session_id: self.session_id.clone(),
                            workflow_task_run_id: self.workflow_task_run_id.clone(),
                            workflow_segment_id: segment_id,
                            root_session_id: self.root_session_id.clone(),
                            root_task_run_id: self.root_task_run_id.clone(),
                            request_kind: "language_detection".to_string(),
                        }),
                        ..Default::default()
                    }),
                    |_| {},
                )
                .await
            {
                Ok(language) => {
                    let language = Self::sanitize_detected_language(&language);
                    if language.is_empty() {
                        log::warn!(
                            "[Workflow][session={}][language] Empty language detection result on attempt {}/{}",
                            self.session_id,
                            attempt,
                            MAX_DETECTION_ATTEMPTS
                        );
                        continue;
                    }
                    return Some(language);
                }
                Err(error) => {
                    log::warn!(
                        "[Workflow][session={}][language] Language detection failed on attempt {}/{}: {}",
                        self.session_id,
                        attempt,
                        MAX_DETECTION_ATTEMPTS,
                        error
                    );
                    last_error = Some(error.to_string());
                }
            }
        }

        log::warn!(
            "[Workflow][session={}][language] Language detection failed after {} attempts; continuing without a language directive{}",
            self.session_id,
            MAX_DETECTION_ATTEMPTS,
            last_error
                .map(|error| format!(": {}", error))
                .unwrap_or_default()
        );
        None
    }

    /// Truncates text to a rough token budget using the shared estimator.
    /// Keeps the head (2/3) and tail (1/3) of the input so language cues at
    /// either end survive, and always cuts on char boundaries (CJK-safe).
    fn truncate_to_token_budget(text: &str, max_tokens: usize) -> String {
        if max_tokens == 0 || estimate_tokens(text) <= max_tokens as f64 {
            return text.to_string();
        }

        let chars: Vec<char> = text.chars().collect();
        let head_budget = ((max_tokens as f64) * 2.0 / 3.0).max(2.0);
        let tail_budget = (max_tokens as f64) / 3.0;

        let mut head_end = 0usize;
        let mut head_tokens = 0.0;
        for (index, character) in chars.iter().enumerate() {
            let weight = estimate_tokens(&character.to_string());
            if head_tokens + weight > head_budget {
                break;
            }
            head_tokens += weight;
            head_end = index + 1;
        }

        let mut tail_start = chars.len();
        let mut tail_tokens = 0.0;
        for index in (head_end..chars.len()).rev() {
            let weight = estimate_tokens(&chars[index].to_string());
            if tail_tokens + weight > tail_budget {
                break;
            }
            tail_tokens += weight;
            tail_start = index;
        }

        format!(
            "{}\n\n[...]\n\n{}",
            chars[..head_end].iter().collect::<String>(),
            chars[tail_start..].iter().collect::<String>()
        )
    }

    /// Normalizes a raw language-detection reply into a short language label.
    /// The chat interface returns a JSON envelope (`{"content": ...,
    /// "reasoning": ...}`), so the visible reply text must be extracted from
    /// `content` before label normalization.
    fn sanitize_detected_language(raw: &str) -> String {
        let visible = match serde_json::from_str::<serde_json::Value>(raw.trim()) {
            Ok(value) => value
                .get("content")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| raw.to_string()),
            Err(_) => raw.to_string(),
        };
        let first_line = visible
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("");
        let trimmed = first_line
            .trim()
            .trim_matches(|character: char| {
                matches!(character, '"' | '\'' | '`' | '*' | '.' | '。')
            })
            .trim();
        trimmed.chars().take(40).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::IntelligenceManager;
    use crate::ccproxy::utils::token_estimator::estimate_tokens;

    #[test]
    fn language_truncation_respects_budget_and_char_boundaries() {
        let cjk_input = "这是一段很长的中文输入，用于验证截断逻辑。".repeat(500);
        let truncated = IntelligenceManager::truncate_to_token_budget(&cjk_input, 100);

        // Head + tail budgets plus the small ASCII marker must stay near the cap.
        assert!(
            estimate_tokens(&truncated) < 110.0,
            "truncated input exceeded the token budget: {}",
            estimate_tokens(&truncated)
        );
        assert!(truncated.starts_with("这是一段"));
        assert!(truncated.ends_with("验证截断逻辑。"));
        assert!(truncated.contains("[...]"));
    }

    #[test]
    fn language_truncation_keeps_short_input_intact() {
        let short_input = "帮我修复登录 bug";
        assert_eq!(
            IntelligenceManager::truncate_to_token_budget(short_input, 8192),
            short_input
        );
    }

    #[test]
    fn language_sanitizer_extracts_short_label() {
        assert_eq!(
            IntelligenceManager::sanitize_detected_language("\"中文\"\nextra"),
            "中文"
        );
        assert_eq!(
            IntelligenceManager::sanitize_detected_language("  English. "),
            "English"
        );
        assert_eq!(IntelligenceManager::sanitize_detected_language(""), "");
    }

    #[test]
    fn language_sanitizer_parses_chat_json_envelope() {
        assert_eq!(
            IntelligenceManager::sanitize_detected_language(r#"{"content":"中文","reasoning":""}"#),
            "中文"
        );
        assert_eq!(
            IntelligenceManager::sanitize_detected_language(
                r#"{"reasoning":"thinking","content":" English "}"#
            ),
            "English"
        );
    }

    #[test]
    fn smart_approval_requires_valid_low_risk_json() {
        let approved = IntelligenceManager::parse_tool_approval_review(
            r#"{"approved":true,"reason":"Read-only inspection","risk_level":"low"}"#,
        );
        assert!(approved.approved);

        for invalid in [
            "I approve this action",
            "I do not recommend approving this action",
            r#"{"approved":true,"reason":"Mutation","risk_level":"medium"}"#,
            r#"{"approved":"true","reason":"Invalid type","risk_level":"low"}"#,
            r#"{"approved":true,"reason":"Missing risk"}"#,
        ] {
            let review = IntelligenceManager::parse_tool_approval_review(invalid);
            assert!(!review.approved, "invalid review was approved: {invalid}");
            assert_eq!(review.risk_level, "medium");
        }
    }

    #[test]
    fn smart_approval_preserves_structured_rejection() {
        let review = IntelligenceManager::parse_tool_approval_review(
            r#"{"approved":false,"reason":"Needs user review","risk_level":"high"}"#,
        );
        assert!(!review.approved);
        assert_eq!(review.reason, "Needs user review");
        assert_eq!(review.risk_level, "high");
    }
}
