use crate::ai::chat::openai::OpenAIChat;
use crate::ai::interaction::chat_completion::{AiChatEnum, ChatState};
use crate::ai::traits::chat::{ChatMetadata, MessageType};
use crate::db::WorkflowMessage;
use crate::tools::TOOL_COMPLETE_WORKFLOW;
use crate::workflow::react::context::ContextManager;
use crate::workflow::react::error::WorkflowEngineError;

use std::sync::Arc;
use tokio::time::{sleep, Duration};

/// IntelligenceManager handles high-level AI decision making tasks
/// like content summarization and quality auditing.
pub struct IntelligenceManager {
    pub session_id: String,
    pub chat_state: Arc<ChatState>,
    pub active_provider_id: i64,
    pub active_model_name: String,
    pub utility_provider_id: i64,
    pub utility_model_name: String,
    pub audit_provider_id: i64,
    pub audit_model_name: String,
    pub approval_provider_id: i64,
    pub approval_model_name: String,
}

#[derive(Debug, Clone)]
pub struct ToolApprovalReview {
    pub approved: bool,
    pub reason: String,
    pub risk_level: String,
}

impl IntelligenceManager {
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
        audit_provider_id: i64,
        audit_model_name: String,
    ) -> Self {
        Self {
            session_id,
            chat_state,
            active_provider_id,
            active_model_name: active_model_name.clone(),
            utility_provider_id: active_provider_id,
            utility_model_name: active_model_name.clone(),
            audit_provider_id,
            audit_model_name,
            approval_provider_id: active_provider_id,
            approval_model_name: active_model_name,
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

        let result = result.trim();
        let review_json: serde_json::Value = match serde_json::from_str(
            crate::libs::util::format_json_str(result).as_str(),
        ) {
            Ok(v) => v,
            Err(_) => {
                let result_lower = result.to_lowercase();
                if result_lower.contains("approve") && !result_lower.contains("reject") {
                    serde_json::json!({
                        "approved": true,
                        "reason": if result.is_empty() { "approved".to_string() } else { result.to_string() },
                        "risk_level": "low"
                    })
                } else {
                    serde_json::json!({
                        "approved": false,
                        "reason": if result.is_empty() { "approval review rejected the call".to_string() } else { result.to_string() },
                        "risk_level": "medium"
                    })
                }
            }
        };

        Ok(ToolApprovalReview {
            approved: review_json["approved"].as_bool().unwrap_or(false),
            reason: review_json["reason"]
                .as_str()
                .unwrap_or("Approval review unavailable")
                .trim()
                .to_string(),
            risk_level: review_json["risk_level"]
                .as_str()
                .unwrap_or("medium")
                .trim()
                .to_string(),
        })
    }

    /// Generates a concise title for the workflow session based on the user's initial query.
    pub async fn generate_workflow_title(
        &self,
        user_query: &str,
    ) -> Result<String, WorkflowEngineError> {
        let (provider_id, model_name) = {
            let store = self
                .chat_state
                .main_store
                .read()
                .map_err(|e| WorkflowEngineError::General(e.to_string()))?;
            let gen_model_config: serde_json::Value =
                store.get_config("conversation_title_gen_model", serde_json::json!({}));

            if gen_model_config.is_object() {
                let p_id = gen_model_config["id"].as_i64();
                let m_name = gen_model_config["model"].as_str();
                if let (Some(provider_id), Some(model_name)) = (p_id, m_name) {
                    if provider_id > 0 && !model_name.trim().is_empty() {
                        (provider_id, model_name.to_string())
                    } else if !self.utility_model_name.trim().is_empty() {
                        (self.utility_provider_id, self.utility_model_name.clone())
                    } else {
                        (self.active_provider_id, self.active_model_name.clone())
                    }
                } else if !self.utility_model_name.trim().is_empty() {
                    (self.utility_provider_id, self.utility_model_name.clone())
                } else {
                    (self.active_provider_id, self.active_model_name.clone())
                }
            } else {
                if !self.utility_model_name.trim().is_empty() {
                    (self.utility_provider_id, self.utility_model_name.clone())
                } else {
                    (self.active_provider_id, self.active_model_name.clone())
                }
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
                    None,
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
            let store = self
                .chat_state
                .main_store
                .write()
                .map_err(|e| WorkflowEngineError::General(e.to_string()))?;
            let _ = store.update_workflow_title(&self.session_id, &final_title);
        }

        Ok(final_title)
    }
}
