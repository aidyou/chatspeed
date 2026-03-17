use crate::ai::chat::openai::OpenAIChat;
use crate::ai::interaction::chat_completion::{AiChatEnum, ChatState};
use crate::ai::traits::chat::{ChatMetadata, MessageType};
use crate::tools::{TOOL_FINISH_TASK, TOOL_TODO_CREATE, TOOL_WEB_SEARCH};
use crate::workflow::react::context::ContextManager;
use crate::workflow::react::error::WorkflowEngineError;
use crate::workflow::react::prompts::{CONTENT_FILTERING_PROMPT, SELF_REFLECTION_AUDIT_PROMPT};

use std::sync::Arc;

/// IntelligenceManager handles high-level AI decision making tasks
/// like content summarization and quality auditing.
pub struct IntelligenceManager {
    pub session_id: String,
    pub chat_state: Arc<ChatState>,
    pub active_provider_id: i64,
    pub active_model_name: String,
}

impl IntelligenceManager {
    pub fn new(
        session_id: String,
        chat_state: Arc<ChatState>,
        active_provider_id: i64,
        active_model_name: String,
    ) -> Self {
        Self {
            session_id,
            chat_state,
            active_provider_id,
            active_model_name,
        }
    }

    /// Summarizes long web content to extract mission-relevant information.
    pub async fn summarize_web_content(
        &self,
        content: &str,
        context: &ContextManager,
    ) -> Result<String, WorkflowEngineError> {
        // 1. Extract Global Goal (First user message)
        let global_goal = context
            .messages
            .first()
            .map(|m| m.message.clone())
            .unwrap_or_else(|| "General research".to_string());

        // 2. Extract Current Task (Directly from Store)
        let current_task = if let Ok(store) = context.main_store.read() {
            store
                .get_todo_list_for_workflow(&self.session_id)
                .ok()
                .and_then(|tasks| {
                    tasks
                        .iter()
                        .find(|t| t["status"] == "in_progress")
                        .and_then(|t| t["subject"].as_str().map(|s| s.to_string()))
                })
        } else {
            None
        }
        .unwrap_or_else(|| "Executing planned steps".to_string());

        // 3. Extract Immediate Intent (Last search query + last reasoning)
        let last_search = context
            .messages
            .iter()
            .rev()
            .find_map(|m| {
                if let Some(meta) = &m.metadata {
                    if let Some(tc) = meta.get("tool_call") {
                        let name = tc
                            .get("name")
                            .or_else(|| tc.get("function").and_then(|f| f.get("name")))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if name == TOOL_WEB_SEARCH {
                            let args_val = tc
                                .get("arguments")
                                .or_else(|| tc.get("function").and_then(|f| f.get("arguments")));

                            if let Some(val) = args_val {
                                // OpenAI protocol often provides arguments as a JSON string
                                let parsed_args = if let Some(s) = val.as_str() {
                                    serde_json::from_str::<serde_json::Value>(s)
                                        .unwrap_or(val.clone())
                                } else {
                                    val.clone()
                                };

                                // Return the 'query' field or the whole JSON if not found
                                return parsed_args
                                    .get("query")
                                    .and_then(|q| q.as_str())
                                    .map(|s| s.to_string())
                                    .or_else(|| Some(parsed_args.to_string()));
                            }
                        }
                    }
                }
                None
            })
            .unwrap_or_else(|| "N/A".to_string());

        let last_reasoning = context
            .messages
            .iter()
            .rev()
            .find(|m| m.role == "assistant" && !m.message.trim().is_empty())
            .map(|m| {
                let text = m.message.clone();
                // Character-safe truncation to avoid panic on multi-byte characters (CJK)
                if text.chars().count() > 500 {
                    text.chars().take(500).collect::<String>() + "..."
                } else {
                    text
                }
            })
            .unwrap_or_else(|| "Analyzing content".to_string());

        let immediate_intent = format!(
            "Search Query: {} | Reasoning: {}",
            last_search, last_reasoning
        );

        // 4. Build Structure-aware Prompt
        let system_prompt = CONTENT_FILTERING_PROMPT
            .replace("{global_goal}", &global_goal)
            .replace("{current_task}", &current_task)
            .replace("{immediate_intent}", &immediate_intent);

        let messages = vec![
            serde_json::json!({
                "role": "system",
                "content": system_prompt
            }),
            serde_json::json!({
                "role": "user",
                "content": format!("<source_content>\n{}\n</source_content>", content)
            }),
        ];

        let chat_interface = {
            let mut chats_guard = self.chat_state.chats.lock().await;
            let protocol = crate::ccproxy::ChatProtocol::OpenAI;
            let chat_map = chats_guard
                .entry(protocol)
                .or_insert_with(std::collections::HashMap::new);
            chat_map
                .entry(self.session_id.clone() + "_summarizer")
                .or_insert_with(|| crate::create_chat!(self.chat_state.main_store))
                .clone()
        };

        let (tx, mut rx) = tokio::sync::mpsc::channel(100);
        chat_interface
            .chat(
                self.active_provider_id,
                &self.active_model_name,
                self.session_id.clone() + "_summarizer",
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
            .map_err(WorkflowEngineError::Ai)?;

        let mut summary = String::new();
        while let Some(chunk) = rx.recv().await {
            match chunk.r#type {
                crate::ai::traits::chat::MessageType::Text => {
                    summary.push_str(&chunk.chunk);
                }
                crate::ai::traits::chat::MessageType::Finished => {
                    break;
                }
                crate::ai::traits::chat::MessageType::Error => {
                    return Err(WorkflowEngineError::Ai(
                        crate::ai::error::AiError::ApiRequestFailed {
                            status_code: 500,
                            provider: "Summarizer".to_string(),
                            details: chunk.chunk.clone(),
                        },
                    ));
                }
                _ => {}
            }
        }

        Ok(summary)
    }

    /// Performs a hidden quality audit before allowing the task to finish.
    pub async fn run_final_audit(
        &self,
        context: &ContextManager,
    ) -> Result<Option<String>, WorkflowEngineError> {
        log::info!(
            "IntelligenceManager {}: Running final quality audit...",
            self.session_id
        );

        let messages = context.get_messages_for_llm();

        // 1. Consolidate Mission Context
        let user_queries: Vec<String> = messages
            .iter()
            .filter(|m| m.role == "user" && m.step_type.as_deref().unwrap_or_default() != "observe")
            .map(|m| m.message.clone())
            .collect();

        let plan_text = messages
            .iter()
            .find(|m| {
                m.step_type.as_deref() == Some("Plan") || m.message.contains(TOOL_TODO_CREATE)
            })
            .map(|m| m.message.as_str())
            .unwrap_or("No initial plan provided.");

        let todo_json = if let Ok(store) = context.main_store.read() {
            store
                .get_todo_list_for_workflow(&self.session_id)
                .map(|t| serde_json::to_string_pretty(&t).unwrap_or_default())
                .unwrap_or_default()
        } else {
            String::new()
        };

        // 2. Extract Audit Rejection History to ensure consistency
        let mut audit_history = String::new();
        for (i, m) in messages.iter().enumerate() {
            if m.error_type.as_deref() == Some("AuditRejected") {
                // Find the assistant response that for audit
                if let Some(next_m) = messages.get(i - 1) {
                    if next_m.role == "assistant" {
                        audit_history.push_str(&format!(
                            "<agent_message>\n{}\n</agent_message>\n\n",
                            next_m.message
                        ));
                    }
                }
                audit_history.push_str(&format!(
                    "<rejection_feedback>\n{}\n</rejection_feedback>\n\n",
                    m.message
                ));
            }
        }

        let mut history = vec![
            (serde_json::json!({
                "role": "system",
                "content": format!(
                    "{}\n\n\
                    AUDIT CONTEXT:\n\n\
                    <user_missions>\n{}\n</user_missions>\n\n\
                    <initial_plan>\n{}\n</initial_plan>\n\n\
                    <current_todo_status>\n{}\n</current_todo_status>\n\n\
                    <previous_audit_feedback>\n{}\n</previous_audit_feedback>",
                    SELF_REFLECTION_AUDIT_PROMPT,
                    user_queries.join("\n---\n"),
                    plan_text,
                    todo_json,
                    if audit_history.is_empty() { "None".to_string() } else { audit_history }
                )
            })),
        ];

        // 3. Add the Target Conclusion (The Assistant message calling finish_task)
        if let Some(target_msg) = messages.iter().rev().find(|m| {
            m.role == "assistant"
                && (m.message.contains(TOOL_FINISH_TASK)
                    || m.metadata.as_ref().map_or(false, |meta| {
                        meta.get("tool_calls")
                            .and_then(|tc| tc.as_array())
                            .map_or(false, |arr| {
                                arr.iter().any(|call| {
                                    call["name"] == TOOL_FINISH_TASK
                                        || call["function"]["name"] == TOOL_FINISH_TASK
                                })
                            })
                    }))
        }) {
            history.push(serde_json::json!({
                "role": "user",
                "content": format!("<proposed_conclusion>\n{}\n</proposed_conclusion>", target_msg.message)
            }));
        } else {
            // Fallback: If we can't find it exactly, use the very last assistant message
            if let Some(last_msg) = messages.iter().rev().find(|m| m.role == "assistant") {
                history.push(serde_json::json!({
                    "role": "user",
                    "content": format!("<proposed_conclusion>\n{}\n</proposed_conclusion>", last_msg.message)
                }));
            }
        }

        let chat_interface = {
            let mut chats_guard = self.chat_state.chats.lock().await;
            chats_guard
                .entry(crate::ccproxy::ChatProtocol::OpenAI)
                .or_default()
                .entry(self.session_id.clone() + "_auditor")
                .or_insert_with(|| crate::create_chat!(self.chat_state.main_store))
                .clone()
        };

        let (tx, mut rx) = tokio::sync::mpsc::channel(100);

        // Spawn the LLM call in a separate task so we can process the stream concurrently
        let session_id_audit = self.session_id.clone() + "_auditor";
        let provider_id = self.active_provider_id;
        let model_name = self.active_model_name.clone();

        tokio::spawn(async move {
            if let Err(e) = chat_interface
                .chat(
                    provider_id,
                    &model_name,
                    session_id_audit,
                    history,
                    None,
                    None,
                    move |chunk| {
                        let _ = tx.try_send(chunk);
                    },
                )
                .await
            {
                log::error!("IntelligenceManager audit background task failed: {}", e);
            }
        });

        let mut result = String::new();
        while let Some(chunk) = rx.recv().await {
            match chunk.r#type {
                MessageType::Text => {
                    result.push_str(&chunk.chunk);
                }
                MessageType::Finished => {
                    break;
                }
                MessageType::Error => {
                    log::error!("IntelligenceManager audit LLM error: {}", chunk.chunk);
                    return Ok(None); // Fail open
                }
                _ => {}
            }
        }

        let result = result.trim();

        let audit_json: serde_json::Value =
            match serde_json::from_str(crate::libs::util::format_json_str(result).as_str()) {
                Ok(v) => v,
                Err(_) => {
                    // Fallback for non-JSON responses
                    let result_upper = result.to_uppercase();
                    if result_upper.contains("APPROVED") && !result_upper.contains("REJECTED") {
                        serde_json::json!({ "approved": true })
                    } else {
                        let feedback = if result_upper.contains("REJECTED:") {
                            let idx = result_upper.find("REJECTED:").unwrap();
                            result[idx + 9..].trim().to_string()
                        } else {
                            result.to_string()
                        };
                        serde_json::json!({ "approved": false, "reason": feedback })
                    }
                }
            };

        if audit_json["approved"].as_bool().unwrap_or(false) {
            Ok(None)
        } else {
            let feedback = audit_json["reason"].as_str().unwrap_or("").trim();
            if feedback.is_empty() || feedback.to_uppercase() == "REJECTED" {
                Ok(Some("The conclusion was rejected by audit. Please review the global goals and ensure all requirements are addressed in your final report.".to_string()))
            } else {
                Ok(Some(feedback.to_string()))
            }
        }
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
                let p_id = gen_model_config["id"]
                    .as_i64()
                    .unwrap_or(self.active_provider_id);
                let m_name = gen_model_config["model"]
                    .as_str()
                    .unwrap_or(&self.active_model_name)
                    .to_string();
                (p_id, m_name)
            } else {
                (self.active_provider_id, self.active_model_name.clone())
            }
        };

        let system_prompt = "You are a helpful assistant that generates extremely concise, descriptive titles for chat conversations. \
                             Your title must be under 10 words, preferably 3-5 words. Do not use quotes or special formatting. \
                             IMPORTANT: Generate the title in the same language as the user's input (Chinese for Chinese input, English for English input, etc.).";
        let user_prompt = format!(
            "Generate a title for a workflow with the following initial request:\n\n\"{}\"",
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

        let (tx, mut rx) = tokio::sync::mpsc::channel(10);
        let session_id_title = self.session_id.clone() + "_title_gen";

        tokio::spawn(async move {
            let _ = chat_interface
                .chat(
                    provider_id,
                    &model_name,
                    session_id_title,
                    messages,
                    None,
                    None,
                    move |chunk| {
                        let _ = tx.try_send(chunk);
                    },
                )
                .await;
        });

        let mut title = String::new();
        while let Some(chunk) = rx.recv().await {
            if chunk.r#type == MessageType::Text {
                title.push_str(&chunk.chunk);
            } else if chunk.r#type == MessageType::Finished {
                break;
            }
        }

        let final_title = title.trim().trim_matches('"').to_string();
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
