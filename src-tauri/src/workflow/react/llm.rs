use crate::ai::error::AiError;
use crate::ai::interaction::chat_completion::{AiChatEnum, ChatState};
use crate::ai::traits::chat::{ChatMetadata, CustomHeader, MCPToolDeclaration, MessageType};
use crate::db::{Agent, WorkflowMessage};
use crate::workflow::react::agents_md::AgentsMdScanner;
use crate::workflow::react::context::ContextManager;
use crate::workflow::react::error::WorkflowEngineError;
use crate::workflow::react::gateway::Gateway;
use crate::workflow::react::memory::{MemoryManager, MemoryScope};
use crate::workflow::react::policy::{ExecutionPhase, ExecutionPolicy};
use crate::workflow::react::runtime_observation::{
    render_runtime_observation_for_llm, RuntimeObservationPlacement,
};
use crate::workflow::react::security::PathGuard;
use crate::workflow::react::signals::{
    parse_runtime_signal, stash_runtime_signal, stash_user_message, RuntimeSignal,
};
use crate::workflow::react::skills::SkillManifest;
use crate::workflow::react::types::GatewayPayload;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};

use crate::workflow::react::prompts::{
    CHILD_AGENT_COMPLETION_PROMPT, CHILD_AGENT_CORE_SYSTEM_PROMPT, CHILD_AGENT_DIRECTORY_PROMPT,
    CORE_SYSTEM_PROMPT, DRAFTING_PROMPT, EXECUTION_MODE_PROMPT,
    FINAL_AUDIT_COMPLETION_REPORT_PROMPT, PLANNING_MODE_PROMPT,
};

pub struct LlmProcessor {
    pub session_id: String,
    pub agent_config: Agent,
    pub child_agents: Vec<Agent>,
    pub available_skills: HashMap<String, SkillManifest>,
    pub path_guard: Arc<RwLock<PathGuard>>,
    pub active_provider_id: i64,
    pub active_model_name: String,
    pub reasoning: bool,
    pub mcp_tool_summaries: Vec<MCPToolDeclaration>,
    // New fields for memory and AGENTS.md support
    pub memory_manager: Arc<MemoryManager>,
    pub project_root: Option<PathBuf>,
}

impl LlmProcessor {
    fn preview_for_log(value: &str, max_chars: usize) -> String {
        let sanitized = value.replace('\n', "\\n").replace('\r', "\\r");
        let mut preview = sanitized.chars().take(max_chars).collect::<String>();
        let total_chars = sanitized.chars().count();
        if total_chars > max_chars {
            preview.push_str(&format!("... [truncated, chars={}]", total_chars));
        }
        preview
    }

    fn usage_for_log(metadata: Option<&serde_json::Value>) -> String {
        metadata
            .and_then(|meta| meta.get("tokens"))
            .map(|tokens| tokens.to_string())
            .unwrap_or_else(|| "null".to_string())
    }

    fn drain_stop_signal_for_retry_boundary(
        session_id: &str,
        signal_rx: &mut tokio::sync::mpsc::Receiver<String>,
    ) -> bool {
        let mut should_stop = false;
        while let Ok(signal) = signal_rx.try_recv() {
            match parse_runtime_signal(&signal) {
                RuntimeSignal::Stop => {
                    log::info!(
                        "WorkflowExecutor {}: Stop signal received before retry notification",
                        session_id
                    );
                    should_stop = true;
                }
                RuntimeSignal::UserMessage {
                    content,
                    attached_context,
                    metadata,
                    queued_user_message_id,
                } => {
                    let queued_id = queued_user_message_id
                        .unwrap_or_else(|| format!("queued_{}", crate::ccproxy::get_tool_id()));
                    stash_user_message(session_id, queued_id, content, attached_context, metadata);
                }
                RuntimeSignal::Other { .. } => stash_runtime_signal(session_id, signal),
            }
        }
        should_stop
    }

    fn normalize_and_validate_tool_calls(
        raw: &str,
        allowed_tool_names: &HashSet<String>,
    ) -> Result<serde_json::Value, String> {
        let mut tool_calls_val: serde_json::Value =
            serde_json::from_str(raw).unwrap_or(serde_json::json!([]));

        if let Some(tool_calls) = tool_calls_val.get("tool_calls").cloned() {
            tool_calls_val = tool_calls;
        }

        let validate_name = |tool_name: Option<&str>| -> Result<(), String> {
            let Some(name) = tool_name else {
                return Err("LLM returned malformed tool call without tool name".to_string());
            };

            if !allowed_tool_names.contains(name) {
                return Err(format!("LLM returned unsupported tool call '{}'", name));
            }

            Ok(())
        };

        if let Some(tool_calls_array) = tool_calls_val.as_array_mut() {
            for (i, tool_call) in tool_calls_array.iter_mut().enumerate() {
                let Some(tool_call_obj) = tool_call.as_object_mut() else {
                    return Err("LLM returned malformed tool call entry".to_string());
                };

                let tool_name = tool_call_obj
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|v| v.as_str())
                    .or_else(|| tool_call_obj.get("name").and_then(|v| v.as_str()));
                validate_name(tool_name)?;

                let existing_id = tool_call_obj.get("id").and_then(|v| v.as_str());
                if existing_id.map_or(true, |id| !id.starts_with("tool_")) {
                    tool_call_obj.insert(
                        "id".to_string(),
                        serde_json::json!(crate::ccproxy::get_tool_id()),
                    );
                }
                tool_call_obj.insert("index".to_string(), serde_json::json!(i));
            }
        } else if let Some(tool_wrapper) = tool_calls_val.get_mut("tool") {
            let Some(tool_obj) = tool_wrapper.as_object_mut() else {
                return Err("LLM returned malformed wrapped tool call".to_string());
            };
            let tool_name = tool_obj
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str())
                .or_else(|| tool_obj.get("name").and_then(|v| v.as_str()));
            validate_name(tool_name)?;

            let existing_id = tool_obj.get("id").and_then(|v| v.as_str());
            if existing_id.map_or(true, |id| !id.starts_with("tool_")) {
                tool_obj.insert(
                    "id".to_string(),
                    serde_json::json!(crate::ccproxy::get_tool_id()),
                );
            }
            tool_obj.insert("index".to_string(), serde_json::json!(0));
        } else if tool_calls_val.is_object() {
            let Some(tool_obj) = tool_calls_val.as_object_mut() else {
                return Err("LLM returned malformed tool call object".to_string());
            };

            let tool_name = tool_obj
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str())
                .or_else(|| tool_obj.get("name").and_then(|v| v.as_str()));
            validate_name(tool_name)?;

            let existing_id = tool_obj.get("id").and_then(|v| v.as_str());
            if existing_id.map_or(true, |id| !id.starts_with("tool_")) {
                tool_obj.insert(
                    "id".to_string(),
                    serde_json::json!(crate::ccproxy::get_tool_id()),
                );
            }
            tool_obj.insert("index".to_string(), serde_json::json!(0));
        }

        Ok(tool_calls_val)
    }

    pub fn new(
        session_id: String,
        agent_config: Agent,
        child_agents: Vec<Agent>,
        available_skills: HashMap<String, SkillManifest>,
        path_guard: Arc<RwLock<PathGuard>>,
        _chat_state: Arc<ChatState>,
        active_provider_id: i64,
        active_model_name: String,
        reasoning: bool,
        mcp_tool_summaries: Vec<MCPToolDeclaration>,
        // New parameters
        memory_manager: Arc<MemoryManager>,
        project_root: Option<PathBuf>,
    ) -> Self {
        Self {
            session_id,
            agent_config,
            child_agents,
            available_skills,
            path_guard,
            active_provider_id,
            active_model_name,
            reasoning,
            mcp_tool_summaries,
            memory_manager,
            project_root,
        }
    }

    /// Prepares and calls the LLM with the current context.
    /// Implements exponential backoff for 429 errors and drafting instructions for non-reasoning models.
    pub async fn call(
        &self,
        context: &mut ContextManager,
        current_step: usize,
        chat_interface: AiChatEnum,
        gateway: Arc<dyn Gateway>,
        tools: Vec<MCPToolDeclaration>,
        max_steps: usize,
        policy: &ExecutionPolicy,
        signal_rx: &mut tokio::sync::mpsc::Receiver<String>,
        require_tool_call: bool,
    ) -> Result<(String, String, String, Option<serde_json::Value>), WorkflowEngineError> {
        let raw_history = context.get_messages_for_llm();

        // 1. Context Normalization & Prompt Injection
        let state_snapshot =
            raw_history.iter().rev().find_map(|m| {
                if m.role == "system" && m.metadata.as_ref().is_some_and(|_| {
                    crate::workflow::react::context::ContextManager::is_compression_summary_message(
                        m,
                    )
                }) {
                    Some(m.message.clone())
                } else {
                    None
                }
            });

        // Extract the next pending todo item for progress display.
        let next_pending_task = None::<String>;

        let history = self.normalize_history(raw_history);
        let final_history = self.inject_prompts(
            history,
            current_step,
            max_steps,
            state_snapshot,
            next_pending_task,
            policy,
        );

        // 2. Retry Loop for transient LLM failures with exponential backoff
        let mut retry_count = 0;
        let max_retries = 10;
        let mut last_error = None;

        while retry_count <= max_retries {
            let (tx, mut rx) = mpsc::channel::<Arc<crate::ai::traits::chat::ChatResponse>>(100);
            let session_id_for_rx = self.session_id.clone();
            let gateway_for_rx = gateway.clone();
            let allowed_tool_names: HashSet<String> =
                tools.iter().map(|tool| tool.name.clone()).collect();

            // Task to process streaming chunks
            let rx_processor =
                tokio::spawn(async move {
                    let mut plain_text = String::new();
                    let mut tool_calls_json = String::new();
                    let mut full_reasoning = String::new();
                    let mut final_metadata = None;
                    let mut invalid_tool_call_error = None::<String>;

                    while let Some(chunk) = rx.recv().await {
                        match chunk.r#type {
                            MessageType::Text => {
                                gateway_for_rx
                                    .send(
                                        &session_id_for_rx,
                                        GatewayPayload::Chunk {
                                            content: chunk.chunk.clone(),
                                        },
                                    )
                                    .await?;
                                plain_text.push_str(&chunk.chunk);
                            }
                            MessageType::Reasoning => {
                                gateway_for_rx
                                    .send(
                                        &session_id_for_rx,
                                        GatewayPayload::ReasoningChunk {
                                            content: chunk.chunk.clone(),
                                        },
                                    )
                                    .await?;
                                full_reasoning.push_str(&chunk.chunk);
                            }
                            MessageType::ToolCalls => {
                                match Self::normalize_and_validate_tool_calls(
                                    &chunk.chunk,
                                    &allowed_tool_names,
                                ) {
                                    Ok(tool_calls_val) => {
                                        tool_calls_json = serde_json::to_string(&tool_calls_val)
                                            .unwrap_or(chunk.chunk.clone());
                                    }
                                    Err(error) => {
                                        invalid_tool_call_error = Some(error);
                                        tool_calls_json.clear();
                                    }
                                }
                            }
                            MessageType::Finished => {
                                final_metadata = chunk.metadata.clone();
                            }
                            MessageType::Error => {
                                return Err(WorkflowEngineError::General(chunk.chunk.clone()));
                            }
                            _ => {}
                        }
                    }
                    if let Some(error) = invalid_tool_call_error {
                        let mut meta = final_metadata.unwrap_or_else(|| serde_json::json!({}));
                        meta["invalid_tool_call_error"] = serde_json::json!(error);
                        final_metadata = Some(meta);
                    }

                    Ok::<(String, String, String, Option<serde_json::Value>), WorkflowEngineError>(
                        (plain_text, tool_calls_json, full_reasoning, final_metadata),
                    )
                });

            let tx_for_chat = tx.clone();

            // Construct custom headers to disable silent proxy-level retries
            let custom_headers = vec![CustomHeader {
                key: "x-cs-retry-max-count".to_string(),
                value: "0".to_string(),
            }];

            // Extract model parameters (temperature, max_tokens) from agent config.
            // Negative temperature is the project-wide sentinel for "unset/off".
            let mut temperature = None;
            let mut max_tokens = None;
            let mut thinking = None;

            // Search through configured workflow model roles to find the active model.
            if let Some(ref models) = self.agent_config.models {
                for model_config in [
                    models.plan.as_ref(),
                    models.act.as_ref(),
                    models.utility.as_ref(),
                ]
                .into_iter()
                .flatten()
                {
                    if model_config.model == self.active_model_name {
                        // Temperature: any value < 0 is treated as "Off/Unset"
                        if let Some(temp) = model_config.temperature {
                            if temp >= 0.0 {
                                temperature = Some(temp as f32);
                            }
                        }
                        // Max Output Tokens: 0 or less is treated as "Unset"
                        if let Some(mt) = model_config.max_tokens {
                            if mt > 0 {
                                max_tokens = Some(mt as u32);
                            }
                        }
                        thinking = model_config.thinking.clone();
                        break;
                    }
                }
            }

            let chat_res = chat_interface
                .chat(
                    self.active_provider_id,
                    &self.active_model_name,
                    self.session_id.clone(),
                    final_history.clone(),
                    Some(tools.clone()),
                    Some(ChatMetadata {
                        reasoning: Some(self.reasoning),
                        thinking,
                        custom_headers: Some(custom_headers),
                        temperature,
                        max_tokens,
                        tool_choice: if require_tool_call && !tools.is_empty() {
                            Some(serde_json::json!("required"))
                        } else {
                            None
                        },
                        ..Default::default()
                    }),
                    move |chunk| {
                        let _ = tx_for_chat.try_send(chunk);
                    },
                )
                .await;

            drop(tx); // Close channel

            match chat_res {
                Ok(_) => {
                    let (mut plain_text, tool_calls_json, mut full_reasoning, final_metadata) =
                        rx_processor.await.map_err(|e| {
                            WorkflowEngineError::General(format!("RX task failed: {}", e))
                        })??;

                    // Only treat model-native thinking tags as hidden reasoning when they
                    // appear at the beginning of the visible content after leading whitespace.
                    // Older domestic reasoning models emit a single leading think block before
                    // the final answer; scanning the entire content can incorrectly strip
                    // literal tags that appear mid-response.
                    if let Some((cleaned_content, extracted_reasoning)) =
                        Self::extract_leading_native_think_block(&plain_text)
                    {
                        if !extracted_reasoning.is_empty() {
                            if !full_reasoning.is_empty() {
                                full_reasoning.push_str("\n\n");
                            }
                            full_reasoning.push_str(&extracted_reasoning);
                        }
                        plain_text = cleaned_content;
                    }

                    let has_invalid_tool_call = final_metadata
                        .as_ref()
                        .and_then(|meta| meta.get("invalid_tool_call_error"))
                        .is_some();
                    if plain_text.trim().is_empty()
                        && tool_calls_json.trim().is_empty()
                        && !has_invalid_tool_call
                    {
                        if Self::drain_stop_signal_for_retry_boundary(&self.session_id, signal_rx) {
                            return Err(WorkflowEngineError::Cancelled(
                                "Stopped before empty-response retry notification".into(),
                            ));
                        }

                        log::info!(
                            "[Workflow][session={}][phase=llm][event=empty_response_diagnostic] content_chars={}, reasoning_chars={}, tool_calls_chars={}, has_invalid_tool_call={}, token_usage={}, reasoning_preview=\"{}\", metadata={}",
                            self.session_id,
                            plain_text.chars().count(),
                            full_reasoning.chars().count(),
                            tool_calls_json.chars().count(),
                            has_invalid_tool_call,
                            Self::usage_for_log(final_metadata.as_ref()),
                            Self::preview_for_log(&full_reasoning, 800),
                            final_metadata
                                .as_ref()
                                .map(|value| value.to_string())
                                .unwrap_or_else(|| "null".to_string())
                        );

                        let e = WorkflowEngineError::General(
                            "LLM returned empty content and no tool calls".to_string(),
                        );
                        if retry_count < max_retries {
                            retry_count += 1;
                            let wait_secs = 2u32.pow(retry_count - 1);

                            log::info!(
		                                "WorkflowExecutor {}: Empty LLM response encountered. Retrying in {}s (attempt {}/{})",
		                                self.session_id,
		                                wait_secs,
	                                retry_count,
	                                max_retries
	                            );

                            gateway
                                .send(
                                    &self.session_id,
                                    GatewayPayload::RetryStatus {
                                        attempt: retry_count,
                                        total_attempts: max_retries,
                                        next_retry_in_seconds: wait_secs,
                                    },
                                )
                                .await?;

                            gateway
	                                .send(
	                                    &self.session_id,
	                                    GatewayPayload::Notification {
	                                        message: format!(
	                                            "AI server returned an empty response. Retrying in {}s (Attempt {}/{})",
	                                            wait_secs, retry_count, max_retries
	                                        ),
	                                        category: Some("warning".to_string()),
	                                    },
	                                )
	                                .await?;

                            tokio::select! {
                                _ = sleep(Duration::from_secs(wait_secs as u64)) => {},
                                sig = signal_rx.recv() => {
                                    if let Some(sig_str) = sig {
                                        match parse_runtime_signal(&sig_str) {
                                            RuntimeSignal::Stop => {
                                                log::info!(
                                                    "WorkflowExecutor {}: Stop signal received during empty-response retry backoff",
                                                    self.session_id
                                                );
                                                return Err(WorkflowEngineError::Cancelled(
                                                    "Stopped during empty-response retry backoff".into(),
                                                ));
                                            }
                                            RuntimeSignal::UserMessage { content, attached_context, metadata, queued_user_message_id } => {
                                                let queued_id = queued_user_message_id.unwrap_or_else(|| {
                                                    format!("queued_{}", crate::ccproxy::get_tool_id())
                                                });
                                                stash_user_message(
                                                    &self.session_id,
                                                    queued_id.clone(),
                                                    content.clone(),
                                                    attached_context,
                                                    metadata.clone(),
                                                );
                                                let mut ui_metadata = metadata.unwrap_or_else(|| serde_json::json!({}));
                                                if !ui_metadata.is_object() {
                                                    ui_metadata = serde_json::json!({});
                                                }
                                                ui_metadata["queued_user_message_id"] = serde_json::json!(queued_id);
                                                ui_metadata["queue_status"] = serde_json::json!("queued");
                                                let _ = gateway.send(
                                                    &self.session_id,
                                                    GatewayPayload::Message {
                                                        role: "user".to_string(),
                                                        content,
                                                        reasoning: None,
                                                        step_type: None,
                                                        step_index: 0,
                                                        is_error: false,
                                                        error_type: None,
                                                        metadata: Some(ui_metadata),
                                                    },
                                                ).await;
                                            }
                                            RuntimeSignal::Other { .. } => {
                                                stash_runtime_signal(&self.session_id, sig_str);
                                            }
                                        }
                                    }
                                }
                            }

                            continue;
                        }
                        return Err(e);
                    }

                    let _ = gateway
                        .send(
                            &self.session_id,
                            GatewayPayload::Notification {
                                message: String::new(),
                                category: Some("info".to_string()),
                            },
                        )
                        .await;

                    return Ok((plain_text, tool_calls_json, full_reasoning, final_metadata));
                }
                Err(e) => {
                    if Self::drain_stop_signal_for_retry_boundary(&self.session_id, signal_rx) {
                        return Err(WorkflowEngineError::Cancelled(
                            "Stopped before LLM error retry notification".into(),
                        ));
                    }

                    let should_retry = match &e {
                        AiError::ApiRequestFailed { status_code, .. } => {
                            // Do NOT retry on auth/not-found errors.
                            // Some providers return transient upstream/runtime issues as HTTP 400,
                            // so 400 must still get bounded retries instead of crashing the workflow.
                            !matches!(*status_code, 401 | 403 | 404)
                        }
                        // Retry on stream errors, network timeouts, etc.
                        _ => true,
                    };

                    if should_retry && retry_count < max_retries {
                        retry_count += 1;
                        let wait_secs = 2u32.pow(retry_count - 1);

                        log::info!("WorkflowExecutor {}: LLM error encountered. Retrying in {}s (attempt {}/{}) - Error: {}",
                            self.session_id, wait_secs, retry_count, max_retries, e);

                        // Send standard retry status for timer logic
                        gateway
                            .send(
                                &self.session_id,
                                GatewayPayload::RetryStatus {
                                    attempt: retry_count,
                                    total_attempts: max_retries,
                                    next_retry_in_seconds: wait_secs,
                                },
                            )
                            .await?;

                        // Also send a user-friendly notification
                        gateway
                            .send(
                                &self.session_id,
                                GatewayPayload::Notification {
                                    message: format!(
                                        "AI server error. Retrying in {}s (Attempt {}/{})",
                                        wait_secs, retry_count, max_retries
                                    ),
                                    category: Some("warning".to_string()),
                                },
                            )
                            .await?;

                        tokio::select! {
                            _ = sleep(Duration::from_secs(wait_secs as u64)) => {},
                            sig = signal_rx.recv() => {
                                if let Some(sig_str) = sig {
                                    match parse_runtime_signal(&sig_str) {
                                        RuntimeSignal::Stop => {
                                            log::info!(
                                                "WorkflowExecutor {}: Stop signal received during retry backoff",
                                                self.session_id
                                            );
                                            return Err(WorkflowEngineError::Cancelled(
                                                "Stopped during retry backoff".into(),
                                            ));
                                        }
                                        RuntimeSignal::UserMessage { content, attached_context, metadata, queued_user_message_id } => {
                                            let queued_id = queued_user_message_id.unwrap_or_else(|| {
                                                format!("queued_{}", crate::ccproxy::get_tool_id())
                                            });
                                            stash_user_message(
                                                &self.session_id,
                                                queued_id.clone(),
                                                content.clone(),
                                                attached_context,
                                                metadata.clone(),
                                            );
                                            let mut ui_metadata = metadata.unwrap_or_else(|| serde_json::json!({}));
                                            if !ui_metadata.is_object() {
                                                ui_metadata = serde_json::json!({});
                                            }
                                            ui_metadata["queued_user_message_id"] = serde_json::json!(queued_id);
                                            ui_metadata["queue_status"] = serde_json::json!("queued");
                                            let _ = gateway.send(
                                                &self.session_id,
                                                GatewayPayload::Message {
                                                    role: "user".to_string(),
                                                    content,
                                                    reasoning: None,
                                                    step_type: None,
                                                    step_index: current_step as i32,
                                                    is_error: false,
                                                    error_type: None,
                                                    metadata: Some(ui_metadata),
                                                },
                                            ).await;
                                        }
                                        RuntimeSignal::Other { .. } => {
                                            stash_runtime_signal(&self.session_id, sig_str);
                                        }
                                    }
                                }
                            }
                        }
                        last_error = Some(WorkflowEngineError::Ai(e));
                        continue;
                    }
                    return Err(WorkflowEngineError::Ai(e));
                }
            }
        }

        Err(last_error
            .unwrap_or_else(|| WorkflowEngineError::General("Max retries exceeded".to_string())))
    }

    fn normalize_history(&self, raw_history: Vec<WorkflowMessage>) -> Vec<serde_json::Value> {
        Self::normalize_history_messages(raw_history)
    }

    fn sanitize_history_reasoning(role: &str, reasoning: Option<&str>) -> Option<String> {
        if role != "assistant" {
            return None;
        }

        crate::workflow::react::context::ContextManager::sanitize_reasoning_content(
            reasoning.map(|value| value.to_string()),
        )
    }

    fn extract_leading_native_think_block(plain_text: &str) -> Option<(String, String)> {
        let trimmed_start = plain_text.trim_start();
        let leading_ws_len = plain_text.len().saturating_sub(trimmed_start.len());
        let trimmed_lower = trimmed_start.to_lowercase();
        let tag_pairs = [("<think>", "</think>"), ("<thought>", "</thought>")];

        for (start_tag, end_tag) in tag_pairs {
            if !trimmed_lower.starts_with(start_tag) {
                continue;
            }

            let content_start = leading_ws_len + start_tag.len();
            let reasoning = if let Some(end_idx) = trimmed_lower[start_tag.len()..].find(end_tag) {
                let absolute_end = content_start + end_idx;
                let mut cleaned_content = String::new();
                cleaned_content.push_str(&plain_text[..leading_ws_len]);
                cleaned_content.push_str(&plain_text[absolute_end + end_tag.len()..]);
                return Some((
                    cleaned_content.trim().to_string(),
                    plain_text[content_start..absolute_end].trim().to_string(),
                ));
            } else {
                plain_text[content_start..].trim().to_string()
            };

            return Some((String::new(), reasoning));
        }

        None
    }

    fn normalize_history_messages(raw_history: Vec<WorkflowMessage>) -> Vec<serde_json::Value> {
        let mut history: Vec<serde_json::Value> = Vec::new();
        let mut deferred_system_observations: Vec<(Option<String>, String)> = Vec::new();

        // If a tool call has both an approval preview (pending) and a later final observation
        // (approved/executed), keep only the final one in LLM context to avoid duplicate tool
        // observations for the same action.
        let resolved_tool_call_ids: HashSet<String> = raw_history
            .iter()
            .filter_map(|m| {
                if m.role != "tool" {
                    return None;
                }
                let meta = m.metadata.as_ref()?;
                let tool_call_id = meta.get("tool_call_id")?.as_str()?;
                let approval_status = meta
                    .get("approval_status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                if approval_status != "pending" {
                    Some(tool_call_id.to_string())
                } else {
                    None
                }
            })
            .collect();

        for m in raw_history {
            // As part of defensive programming, filter out system messages
            if m.role == "system" {
                continue;
            }

            let runtime_observation_content = if let Some(rendered) =
                render_runtime_observation_for_llm(&m)
            {
                match rendered.placement {
                    RuntimeObservationPlacement::Preserve => Some(rendered.content),
                    RuntimeObservationPlacement::Defer => {
                        deferred_system_observations
                            .push((Self::runtime_observation_tool_call_id(&m), rendered.content));
                        continue;
                    }
                    RuntimeObservationPlacement::Hide => continue,
                }
            } else {
                None
            };

            if m.role == "tool" {
                if let Some(meta) = &m.metadata {
                    let approval_status = meta
                        .get("approval_status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let tool_call_id = meta.get("tool_call_id").and_then(|v| v.as_str());

                    if approval_status == "pending"
                        && tool_call_id
                            .map(|id| resolved_tool_call_ids.contains(id))
                            .unwrap_or(false)
                    {
                        continue;
                    }
                }
            }

            let role = m.role.clone();
            let has_runtime_observation = runtime_observation_content.is_some();
            let mut content = runtime_observation_content.unwrap_or_else(|| m.message.clone());
            let mut reasoning_content = if has_runtime_observation {
                None
            } else {
                Self::sanitize_history_reasoning(&role, m.reasoning.as_deref())
            };
            let tool_calls = m
                .metadata
                .as_ref()
                .and_then(|meta| meta.get("tool_calls").cloned());

            let tool_call_id = if role == "tool" {
                m.metadata
                    .as_ref()
                    .and_then(|meta| meta.get("tool_call_id").and_then(|v| v.as_str()))
                    .map(|s| s.to_string())
            } else {
                None
            };

            // Restore: Dynamic System Reminder for Errors
            if role == "tool" && m.is_error {
                let meta = m.metadata.as_ref();
                let tool_name = meta
                    .and_then(|m| m.get("tool_name").and_then(|v| v.as_str())) // New priority
                    .or_else(|| {
                        meta.and_then(|m| {
                            m.get("tool_call").and_then(|tc| {
                                tc.get("function")
                                    .and_then(|f| f.get("name").and_then(|n| n.as_str()))
                            })
                        })
                    })
                    .or_else(|| {
                        meta.and_then(|m| {
                            m.get("tool_call")
                                .and_then(|tc| tc.get("name").and_then(|n| n.as_str()))
                        })
                    })
                    .unwrap_or("unknown");

                let error_type = meta
                    .and_then(|meta| meta.get("error_type").and_then(|v| v.as_str()))
                    .unwrap_or("Other");

                let reminder = generate_error_reminder(error_type, tool_name, &content);
                content = format!("{}\n\n{}", content, reminder);
            }

            if let Some(last) = history.last_mut() {
                if last["role"] == role && role != "tool" && role != "system" {
                    let last_content = last["content"].as_str().unwrap_or("");
                    if !content.is_empty() {
                        last["content"] =
                            serde_json::json!(format!("{}\n\n{}", last_content, content));
                    }

                    if let Some(new_reasoning) = reasoning_content.take() {
                        let merged_reasoning = last["reasoning_content"]
                            .as_str()
                            .filter(|existing| !existing.trim().is_empty())
                            .map(|existing| format!("{}\n\n{}", existing, new_reasoning))
                            .unwrap_or(new_reasoning);
                        last["reasoning_content"] = serde_json::json!(merged_reasoning);
                    }

                    // Merge tool_calls instead of overwriting
                    if let Some(new_tc) = tool_calls {
                        if let Some(existing_tc) = last["tool_calls"].as_array_mut() {
                            if let Some(new_tc_array) = new_tc.as_array() {
                                existing_tc.extend(new_tc_array.clone());
                            } else {
                                existing_tc.push(new_tc);
                            }
                        } else {
                            last["tool_calls"] = new_tc;
                        }

                        // For assistant messages with tool_calls, if content is still empty, set it to Null
                        if role == "assistant"
                            && last["content"]
                                .as_str()
                                .map_or(true, |s| s.trim().is_empty())
                        {
                            last["content"] = serde_json::Value::Null;
                        }
                    }
                    continue;
                }
            }

            if role == "user" && content.trim().is_empty() {
                continue;
            }

            if role == "assistant"
                && content.trim().is_empty()
                && tool_calls.is_none()
                && reasoning_content.is_none()
            {
                continue;
            }

            let mut msg =
                if role == "assistant" && tool_calls.is_some() && content.trim().is_empty() {
                    serde_json::json!({ "role": role, "content": serde_json::Value::Null })
                } else {
                    serde_json::json!({ "role": role, "content": content })
                };
            if let Some(tc) = tool_calls {
                msg["tool_calls"] = tc;
            }
            if let Some(reasoning) = reasoning_content {
                msg["reasoning_content"] = serde_json::json!(reasoning);
            }
            if let Some(tid) = tool_call_id {
                msg["tool_call_id"] = serde_json::json!(tid);
            }
            history.push(msg);
        }

        // Defensive pass: normalize tool_calls[].index to match array position
        // for all assistant messages. Some providers stream chunks with indices
        // that do not align with the final array order, causing strict providers
        // (e.g., minimax) to reject the replayed history with a 500 error.
        for msg in &mut history {
            if msg["role"] == "assistant" {
                if let Some(tool_calls) = msg["tool_calls"].as_array_mut() {
                    for (i, tc) in tool_calls.iter_mut().enumerate() {
                        if let Some(obj) = tc.as_object_mut() {
                            obj.insert("index".to_string(), serde_json::json!(i));
                        }
                    }
                }
            }
        }

        if !deferred_system_observations.is_empty() {
            let mut deferred_by_tool_call_id: HashMap<String, Vec<String>> = HashMap::new();
            let mut trailing_deferred_observations: Vec<String> = Vec::new();

            for (tool_call_id, content) in deferred_system_observations {
                if let Some(tool_call_id) = tool_call_id.filter(|id| !id.trim().is_empty()) {
                    deferred_by_tool_call_id
                        .entry(tool_call_id)
                        .or_default()
                        .push(content);
                } else {
                    trailing_deferred_observations.push(content);
                }
            }

            if !deferred_by_tool_call_id.is_empty() {
                let mut anchored_history = Vec::with_capacity(history.len());
                for msg in history {
                    let tool_call_id = msg
                        .get("tool_call_id")
                        .and_then(|value| value.as_str())
                        .map(str::to_string);
                    anchored_history.push(msg);

                    if let Some(tool_call_id) = tool_call_id {
                        if let Some(contents) = deferred_by_tool_call_id.remove(&tool_call_id) {
                            if let Some(last_msg) = anchored_history.last_mut() {
                                if let Some(existing_content) =
                                    last_msg.get("content").and_then(|value| value.as_str())
                                {
                                    let merged_content = if existing_content.trim().is_empty() {
                                        contents.join("\n\n")
                                    } else {
                                        format!("{}\n\n{}", existing_content, contents.join("\n\n"))
                                    };
                                    last_msg["content"] = serde_json::json!(merged_content);
                                }
                            }
                        }
                    }
                }

                for (_, contents) in deferred_by_tool_call_id {
                    trailing_deferred_observations.extend(contents);
                }

                history = anchored_history;
            }

            if !trailing_deferred_observations.is_empty()
                && !history.last().is_some_and(|msg| {
                    msg["role"] == "assistant"
                        && msg
                            .get("tool_calls")
                            .and_then(|calls| calls.as_array())
                            .is_some_and(|calls| !calls.is_empty())
                })
            {
                history.push(serde_json::json!({
                    "role": "user",
                    "content": trailing_deferred_observations.join("\n\n")
                }));
            }
        }

        history
    }

    fn runtime_observation_tool_call_id(message: &WorkflowMessage) -> Option<String> {
        let metadata = message.metadata.as_ref()?;
        if !crate::workflow::react::runtime_observation::is_runtime_observation(Some(metadata)) {
            return None;
        }

        metadata
            .get("data")
            .and_then(|data| data.get("tool_call_id"))
            .and_then(|value| value.as_str())
            .map(str::to_string)
    }

    fn inject_prompts(
        &self,
        mut history: Vec<serde_json::Value>,
        current_step: usize,
        max_steps: usize,
        state_snapshot: Option<String>,
        _next_pending_task: Option<String>,
        policy: &ExecutionPolicy,
    ) -> Vec<serde_json::Value> {
        let mut system_parts = Vec::new();

        // 1. Core System Prompt
        if self.agent_config.role.as_deref() == Some("child") {
            system_parts.push(CHILD_AGENT_CORE_SYSTEM_PROMPT.to_string());
        } else {
            system_parts.push(CORE_SYSTEM_PROMPT.to_string());
        }

        // 2. Agent Specific Instructions
        let agent_prompt = match policy.phase {
            ExecutionPhase::Planning => self
                .agent_config
                .planning_prompt
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(PLANNING_MODE_PROMPT),
            _ => self.agent_config.system_prompt.as_str(),
        };
        if !agent_prompt.trim().is_empty() {
            system_parts.push(format!(
                "<AGENT_SPECIFIC_INSTRUCTIONS>\n{}\n</AGENT_SPECIFIC_INSTRUCTIONS>",
                agent_prompt
            ));
        }

        // 2.1 Sub Agents
        if !self.child_agents.is_empty() {
            let child_agent_lines = self
                .child_agents
                .iter()
                .map(|child| {
                    let description = child
                        .description
                        .as_deref()
                        .unwrap_or("No description provided");
                    format!(
                        "- Name: {}\n  child_agent_id: {}\n  Description: {}",
                        child.name, child.id, description
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");

            system_parts
                .push(CHILD_AGENT_DIRECTORY_PROMPT.replace("{{child_agents}}", &child_agent_lines));
        }

        if self.agent_config.role.as_deref() == Some("child") {
            system_parts.push(CHILD_AGENT_COMPLETION_PROMPT.to_string());
        }

        // 3. Drafting for non-reasoning models
        if !self.reasoning {
            system_parts.push(DRAFTING_PROMPT.to_string());
        }

        if self.agent_config.final_audit.unwrap_or(false) {
            system_parts.push(FINAL_AUDIT_COMPLETION_REPORT_PROMPT.to_string());
        }

        // === NEW: AGENTS.md and Memory ===

        // Get memory and AGENTS.md content
        let (global_agents, project_agents) = AgentsMdScanner::scan(self.project_root.clone());

        // Use restricted read (last 300 non-empty lines) to stay within context limits
        let global_memory = self
            .memory_manager
            .read_last_n_lines(MemoryScope::Global, 300)
            .ok()
            .flatten();

        let project_memory = self
            .memory_manager
            .read_last_n_lines(MemoryScope::Project, 300)
            .ok()
            .flatten();

        // 4. Global Instructions (AGENTS.md)
        if let Some(content) = global_agents {
            system_parts.push(format!(
                "<GLOBAL_INSTRUCTIONS>\n{}\n\
                <SYSTEM_REMINDER>\n\
                This provides global guidance that may or may not relate to the current project. \
                Please evaluate carefully and apply when relevant.\n\
                </SYSTEM_REMINDER>\n\
                </GLOBAL_INSTRUCTIONS>",
                content
            ));
        }

        // 5. Project Instructions (AGENTS.md)
        if let Some(content) = project_agents {
            system_parts.push(format!(
                "<PROJECT_INSTRUCTIONS>\n{}\n\
                <SYSTEM_REMINDER>\n\
                This provides project-specific guidance for the current codebase.\n\
                </SYSTEM_REMINDER>\n\
                </PROJECT_INSTRUCTIONS>",
                content
            ));
        }

        // 6. Extend tool(skills, mcp) Context
        let extend_tools = self.build_extend_tools_prompt();
        if !extend_tools.is_empty() {
            system_parts.push(format!(
                "<EXTEND_TOOLS_CONTEXT>\n{}\n</EXTEND_TOOLS_CONTEXT>",
                extend_tools
            ));
        }

        // 7. State Snapshot
        if let Some(snapshot) = state_snapshot {
            system_parts.push(format!(
                "<PREVIOUS_CONTEXT_SNAPSHOT>\n{}\n</PREVIOUS_CONTEXT_SNAPSHOT>",
                snapshot
            ));
        }

        // 8. Global Memory
        if let Some(content) = global_memory {
            system_parts.push(format!(
                "<GLOBAL_MEMORY>\n{}\n\
                <SYSTEM_REMINDER>\n\
                These memories may or may not relate to the current project. \
                Please evaluate them carefully. For those relevant to the context, \
                adhere to the user's documented preferences.\n\
                </SYSTEM_REMINDER>\n\
                </GLOBAL_MEMORY>",
                content
            ));
        }

        // 9. Project Memory
        if let Some(content) = project_memory {
            system_parts.push(format!(
                "<PROJECT_MEMORY>\n{}\n\
                <SYSTEM_REMINDER>\n\
                These memories are specific to the current project. \
                If a conflict exists between project-level memory and global memory, \
                project-level instructions MUST take precedence.\n\
                </SYSTEM_REMINDER>\n\
                </PROJECT_MEMORY>",
                content
            ));
        }

        // 10. Environment Context
        let reminders = self.build_reminders(current_step, max_steps);
        system_parts.push(format!(
            "<ENVIRONMENT_CONTEXT>\n{}\n</ENVIRONMENT_CONTEXT>",
            reminders
        ));

        match policy.phase {
            ExecutionPhase::Implementation => {
                system_parts.push(format!(
                    "<PHASE_INSTRUCTIONS>\n{}\n</PHASE_INSTRUCTIONS>",
                    EXECUTION_MODE_PROMPT
                ));
            }
            ExecutionPhase::Standard | ExecutionPhase::Planning => {}
        }

        // === END NEW ===

        let combined_system_prompt = system_parts.join("\n\n---\n\n");

        // 1. Remove all existing system messages
        history.retain(|m| m["role"] != "system");

        // 2. Insert the single consolidated system message at the top
        history.insert(
            0,
            serde_json::json!({ "role": "system", "content": combined_system_prompt }),
        );

        history
    }

    fn build_extend_tools_prompt(&self) -> String {
        let mut reminders = String::new();
        let skills_enabled = self.agent_config.skill_enabled.unwrap_or(true);

        // MCP tools (before skills)
        if !self.mcp_tool_summaries.is_empty() {
            reminders.push_str("## AVAILABLE MCP TOOLS:\n");
            for tool in &self.mcp_tool_summaries {
                reminders.push_str(&format!(
                    "- {}: {}\n",
                    tool.name,
                    tool.description.replace("\n", " ")
                ));
            }
            reminders.push_str("<SYSTEM_REMINDER>Use `mcp_tool_load` tool to get detailed parameter schemas before calling MCP tools.</SYSTEM_REMINDER>\n\n");
        }

        // Skills
        if skills_enabled && !self.available_skills.is_empty() {
            reminders.push_str("## AVAILABLE SKILLS:\n");
            for skill in self.available_skills.values() {
                reminders.push_str(&format!(
                    "- {}: {}\n",
                    skill.name,
                    skill.description.replace("\n", " ")
                ));
            }
            reminders.push_str("<SYSTEM_REMINDER>You can use the above specialized skills via the `skill` tool.</SYSTEM_REMINDER>\n");
        }

        reminders
    }

    fn build_reminders(&self, _current_step: usize, _max_steps: usize) -> String {
        let (allowed_roots, cwd) = if let Ok(guard) = self.path_guard.read() {
            let roots = guard.workspace_roots();
            let primary = guard.get_primary_root().map(|p| p.to_path_buf());
            (roots, primary)
        } else {
            (vec![], None)
        };

        let cwd = cwd
            .or_else(|| allowed_roots.first().cloned())
            .unwrap_or_default();

        let is_git = if !cwd.as_os_str().is_empty() {
            std::process::Command::new("git")
                .args([
                    "-C",
                    &cwd.to_string_lossy(),
                    "rev-parse",
                    "--is-inside-work-tree",
                ])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "true")
                .unwrap_or(false)
        } else {
            false
        };

        let mut env_info =
            String::from("Environment\nYou have been invoked in the following environment:\n");
        if !cwd.as_os_str().is_empty() {
            env_info.push_str(&format!(" - Primary working directory: {:?}\n", cwd));

            if is_git {
                let branch = std::process::Command::new("git")
                    .args([
                        "-C",
                        &cwd.to_string_lossy(),
                        "rev-parse",
                        "--abbrev-ref",
                        "HEAD",
                    ])
                    .output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_else(|_| "unknown".into());

                env_info.push_str(&format!("  - Git Repository: [Branch: {}]\n", branch));
            } else {
                env_info.push_str("  - Is a git repository: false\n");
            }
        }

        if allowed_roots.len() > 1 {
            env_info.push_str(" - Additional working directories:\n");
            for root in allowed_roots.iter().skip(1) {
                env_info.push_str(&format!("  - {:?}\n", root));
            }
        }

        let platform = std::env::consts::OS;
        let shell = if platform == "windows" {
            std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".into())
        } else {
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
        };
        let os_version = if platform == "windows" {
            std::process::Command::new("cmd")
                .args(["/c", "ver"])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_else(|_| "Windows".into())
        } else {
            std::process::Command::new("uname")
                .args(["-sr"])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_else(|_| platform.to_string())
        };

        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        env_info.push_str(&format!(
            " - Platform: {}\n - Shell: {}\n - OS Version: {}\n - Today's date is {}.",
            platform, shell, os_version, today
        ));

        format!(
            "{}\n\n<SYSTEM_REMINDER>\nIMPORTANT: this context may or may not be relevant to your tasks. You should not respond to this context unless it is highly relevant to your task.\n</SYSTEM_REMINDER>\n",
            env_info
        )
    }

    pub fn build_workspace_context(&self) -> String {
        let (allowed_roots, cwd) = if let Ok(guard) = self.path_guard.read() {
            let roots = guard.workspace_roots();
            let primary = guard.get_primary_root().map(|p| p.to_path_buf());
            (roots, primary)
        } else {
            (vec![], None)
        };

        let cwd = cwd
            .or_else(|| allowed_roots.first().cloned())
            .unwrap_or_default();

        let mut workspace_context = String::from("Workspace Context\n");
        if !cwd.as_os_str().is_empty() {
            workspace_context.push_str(&format!(" - Primary working directory: {:?}\n", cwd));
        }

        if allowed_roots.len() > 1 {
            workspace_context.push_str(" - Additional working directories:\n");
            for root in allowed_roots.iter().skip(1) {
                workspace_context.push_str(&format!("   - {:?}\n", root));
            }
        }

        workspace_context.push_str(
            " - Relative paths resolve against the primary working directory.\n\
             - Any file operation outside the primary or additional working directories is not allowed.\n",
        );
        workspace_context
    }
}

pub fn generate_error_reminder(error_type: &str, tool_name: &str, content: &str) -> String {
    match error_type {
        "Security" => {
            "<SYSTEM_REMINDER>Path is outside the authorized workspace. Use 'list_dir' to verify valid paths or ask the user to add access.</SYSTEM_REMINDER>".to_string()
        }
        "Io" => {
            let is_permission = content.contains("Permission denied");
            let is_not_found = content.contains("No such file")
                || content.contains("Not found")
                || content.contains("does not exist");
            match tool_name {
                "read_file" | "list_dir" | "edit_file" | "write_file" => {
                    if is_permission {
                        "<SYSTEM_REMINDER>Permission denied. Check file permissions or ask the user to grant access.</SYSTEM_REMINDER>".to_string()
                    } else if is_not_found {
                        "<SYSTEM_REMINDER>Path not found. Use 'list_dir' to locate it, or create parent directories first if writing.</SYSTEM_REMINDER>".to_string()
                    } else {
                        "<SYSTEM_REMINDER>I/O error. Verify the path and use 'list_dir' if needed.</SYSTEM_REMINDER>".to_string()
                    }
                }
                _ => "<SYSTEM_REMINDER>Review the error and try a different approach if needed.</SYSTEM_REMINDER>".to_string()
            }
        }
        "InvalidParams" => {
            "<SYSTEM_REMINDER>Check the tool schema and required argument types.</SYSTEM_REMINDER>".to_string()
        }
        "Other" => {
            if tool_name == "edit_file" {
                if content.contains("old_string was not found") {
                    return "<SYSTEM_REMINDER>`edit_file` could not find old_string exactly. Re-read the file and copy old_string from inside the structured `<file_content ...>...</file_content>` block only. Do not include visible read_file line numbers, separators, or guessed whitespace.</SYSTEM_REMINDER>".to_string();
                }
                if content.contains("old_string is not unique") {
                    return "<SYSTEM_REMINDER>`edit_file` found multiple matches for old_string. Re-read a narrower region and include more surrounding context from the structured `<file_content ...>...</file_content>` block so the target becomes unique, or use replace_all only if every occurrence should change.</SYSTEM_REMINDER>".to_string();
                }
            }
            "<SYSTEM_REMINDER>Review the error and try a different approach if needed.</SYSTEM_REMINDER>".to_string()
        }
        "NoToolCall" => {
            if tool_name == crate::tools::TOOL_SUBMIT_RESULT {
                "<SYSTEM_REMINDER>This delegated workflow is action-oriented. Brief reasoning-only turns are allowed, but you should soon choose a concrete tool action. If the delegated task is finished, call 'submit_result' with both `result` and `summary`.</SYSTEM_REMINDER>".to_string()
            } else {
                "<SYSTEM_REMINDER>This workflow is action-oriented. Brief reasoning-only turns are allowed, but you should soon choose a concrete tool action that advances the task. If the task is finished, call 'complete_workflow_with_summary' with a complete summary.</SYSTEM_REMINDER>".to_string()
            }
        }
        "Timeout" => {
            "<SYSTEM_REMINDER>Operation timed out. Break it into smaller steps or use a lighter approach.</SYSTEM_REMINDER>".to_string()
        }
        "NetworkError" => {
            match tool_name {
                "web_search" => {
                    "<SYSTEM_REMINDER>Search failed. Retry once; if it still fails, rephrase the query. Use Chinese for China-centric topics. If still blocked, mark data_missing.</SYSTEM_REMINDER>"
                        .to_string()
                }
                "web_fetch" => {
                    "<SYSTEM_REMINDER>Fetch failed. Do not retry the same URL immediately; try another source or mark the data unavailable.</SYSTEM_REMINDER>"
                        .to_string()
                }
                _ => {
                    "<SYSTEM_REMINDER>Network error. Retry once; if it persists, treat the service as unavailable.</SYSTEM_REMINDER>"
                        .to_string()
                }
            }
        }
        "AuthError" => {
            "<SYSTEM_REMINDER>Authentication failed. Check API keys or credentials.</SYSTEM_REMINDER>".to_string()
        }
        "McpServerNotFound" => {
            "<SYSTEM_REMINDER>MCP server not found. Verify the server name or inspect available tools.</SYSTEM_REMINDER>".to_string()
        }
        "Fatal" => {
            "<SYSTEM_REMINDER>Fatal error. Stop and ask the user for guidance.</SYSTEM_REMINDER>".to_string()
        }
        _ => {
            match tool_name {
                "bash" => {
                    let is_cmd_not_found = content.contains("command not found");
                    let is_syntax_error = content.contains("syntax error");
                    if is_cmd_not_found {
                        "<SYSTEM_REMINDER>Command not found. Verify it is installed or use an alternative.</SYSTEM_REMINDER>".to_string()
                    } else if is_syntax_error {
                        "<SYSTEM_REMINDER>Shell syntax error. Check quoting and special-character escaping.</SYSTEM_REMINDER>".to_string()
                    } else {
                        "<SYSTEM_REMINDER>Command failed. Check syntax and required dependencies.</SYSTEM_REMINDER>".to_string()
                    }
                }
        _ => "<SYSTEM_REMINDER>Use this error to choose the next step.</SYSTEM_REMINDER>".to_string()
    }
}
    }
}

#[cfg(test)]
mod tests {
    use super::LlmProcessor;
    use crate::db::WorkflowMessage;
    use serde_json::json;

    fn message(
        role: &str,
        content: &str,
        step_type: Option<&str>,
        metadata: Option<serde_json::Value>,
    ) -> WorkflowMessage {
        WorkflowMessage {
            id: None,
            session_id: "test-session".to_string(),
            role: role.to_string(),
            message: content.to_string(),
            reasoning: None,
            message_kind: "message".to_string(),
            message_subtype: None,
            segment_id: 1,
            source_event_type: None,
            metadata,
            attached_context: None,
            step_type: step_type.map(str::to_string),
            step_index: 0,
            is_error: false,
            error_type: None,
            created_at: None,
        }
    }

    fn message_with_reasoning(
        role: &str,
        content: &str,
        reasoning: &str,
        step_type: Option<&str>,
        metadata: Option<serde_json::Value>,
    ) -> WorkflowMessage {
        WorkflowMessage {
            id: None,
            session_id: "test-session".to_string(),
            role: role.to_string(),
            message: content.to_string(),
            reasoning: Some(reasoning.to_string()),
            message_kind: "message".to_string(),
            message_subtype: None,
            segment_id: 1,
            source_event_type: None,
            metadata,
            attached_context: None,
            step_type: step_type.map(str::to_string),
            step_index: 0,
            is_error: false,
            error_type: None,
            created_at: None,
        }
    }

    #[test]
    fn normalize_history_defers_internal_runtime_observation_until_after_tool_result() {
        let history = LlmProcessor::normalize_history_messages(vec![
            message("user", "Fix the bug", None, None),
            message(
                "assistant",
                "",
                Some("think"),
                Some(json!({
                    "tool_calls": [{
                        "id": "tool_1",
                        "type": "function",
                        "function": { "name": "read_file", "arguments": "{}" }
                    }]
                })),
            ),
            message(
                "user",
                "<SYSTEM_REMINDER>Internal runtime note</SYSTEM_REMINDER>",
                Some("observe"),
                Some(json!({ "message_kind": "runtime_observation" })),
            ),
            message(
                "tool",
                "file content",
                Some("observe"),
                Some(json!({
                    "tool_call_id": "tool_1",
                    "tool_name": "read_file"
                })),
            ),
        ]);

        assert_eq!(history[1]["role"], "assistant");
        assert_eq!(history[2]["role"], "tool");
        assert_eq!(history[2]["tool_call_id"], "tool_1");
        assert_eq!(history[3]["role"], "user");
        assert!(history[3]["content"]
            .as_str()
            .unwrap_or_default()
            .contains("Internal runtime note"));
    }

    #[test]
    fn normalize_history_keeps_sub_agent_tool_result_in_original_order() {
        let history = LlmProcessor::normalize_history_messages(vec![
            message("user", "Investigate the issue", None, None),
            message(
                "assistant",
                "",
                Some("think"),
                Some(json!({
                    "tool_calls": [{
                        "id": "tool_sub",
                        "type": "function",
                        "function": { "name": "sub_agent_run", "arguments": "{}" }
                    }]
                })),
            ),
            message(
                "tool",
                "{\"status\":\"waiting\",\"task_id\":\"subagent_test\"}",
                Some("observe"),
                Some(json!({
                    "tool_call_id": "tool_sub",
                    "tool_name": "sub_agent_run"
                })),
            ),
            message(
                "user",
                "<tool_result tool=\"sub_agent_run\" id=\"subagent_test\" mode=\"call\" status=\"completed\">\n<Result>\nSub-agent findings\n</Result>\n</tool_result>\n<SYSTEM_REMINDER>Use the result.</SYSTEM_REMINDER>",
                Some("observe"),
                Some(json!({
                    "message_kind": "runtime_observation",
                    "observation_type": "sub_agent_completion",
                    "sub_agent_id": "subagent_test"
                })),
            ),
            message(
                "assistant",
                "I will apply the findings.",
                Some("think"),
                None,
            ),
        ]);

        assert_eq!(history[2]["role"], "tool");
        assert_eq!(history[3]["role"], "user");
        assert!(history[3]["content"]
            .as_str()
            .unwrap_or_default()
            .contains("<tool_result tool=\"sub_agent_run\""));
        assert_eq!(history[4]["role"], "assistant");
    }

    #[test]
    fn normalize_history_anchors_deferred_runtime_observation_after_matching_tool_result() {
        let history = LlmProcessor::normalize_history_messages(vec![
            message("user", "Handle both tools", None, None),
            message(
                "assistant",
                "",
                Some("think"),
                Some(json!({
                    "tool_calls": [
                        {
                            "id": "tool_1",
                            "type": "function",
                            "function": { "name": "read_file", "arguments": "{}" }
                        },
                        {
                            "id": "tool_2",
                            "type": "function",
                            "function": { "name": "grep", "arguments": "{}" }
                        }
                    ]
                })),
            ),
            message(
                "user",
                "<SYSTEM_REMINDER>Reminder for tool 1</SYSTEM_REMINDER>",
                Some("observe"),
                Some(json!({
                    "message_kind": "runtime_observation",
                    "llm_visibility": "defer",
                    "data": {
                        "tool_call_id": "tool_1"
                    }
                })),
            ),
            message(
                "tool",
                "file content",
                Some("observe"),
                Some(json!({
                    "tool_call_id": "tool_1",
                    "tool_name": "read_file"
                })),
            ),
            message(
                "tool",
                "grep result",
                Some("observe"),
                Some(json!({
                    "tool_call_id": "tool_2",
                    "tool_name": "grep"
                })),
            ),
        ]);

        assert_eq!(history[1]["role"], "assistant");
        assert_eq!(history[2]["role"], "tool");
        assert_eq!(history[2]["tool_call_id"], "tool_1");
        assert!(history[2]["content"]
            .as_str()
            .unwrap_or_default()
            .contains("Reminder for tool 1"));
        assert_eq!(history[3]["role"], "tool");
        assert_eq!(history[3]["tool_call_id"], "tool_2");
        assert_eq!(history.len(), 4);
    }

    #[test]
    fn normalize_history_keeps_active_todos_block_observation_in_original_order() {
        let history = LlmProcessor::normalize_history_messages(vec![
            message("user", "Finish the task", None, None),
            message(
                "assistant",
                "",
                Some("think"),
                Some(json!({
                    "tool_calls": [{
                        "id": "tool_finish",
                        "type": "function",
                        "function": { "name": "complete_workflow_with_summary", "arguments": "{}" }
                    }]
                })),
            ),
            message(
                "tool",
                "<SYSTEM_REMINDER>Block: You still have active tasks.</SYSTEM_REMINDER>",
                Some("observe"),
                Some(json!({
                    "message_kind": "runtime_observation",
                    "observation_type": "active_todos_blocked",
                    "llm_visibility": "preserve_position",
                    "ui_visibility": "show"
                })),
            ),
            message(
                "tool",
                "Task updated.\n<SYSTEM_REMINDER>Todos are terminal.</SYSTEM_REMINDER>",
                Some("observe"),
                Some(json!({
                    "tool_call_id": "tool_todo",
                    "tool_name": "todo_update"
                })),
            ),
        ]);

        assert_eq!(history[1]["role"], "assistant");
        assert_eq!(history[2]["role"], "tool");
        assert!(history[2]["content"]
            .as_str()
            .unwrap_or_default()
            .contains("active tasks"));
        assert_eq!(history[3]["role"], "tool");
        assert!(history.iter().all(|msg| {
            !(msg["role"] == "user"
                && msg["content"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("active tasks"))
        }));
    }

    #[test]
    fn normalize_history_replays_assistant_reasoning_in_native_field() {
        let history = LlmProcessor::normalize_history_messages(vec![
            message("user", "Implement the fix", None, None),
            message_with_reasoning(
                "assistant",
                "",
                "I already have enough context. Next I should edit main.rs.",
                Some("think"),
                Some(json!({
                    "tool_calls": [{
                        "id": "tool_edit",
                        "type": "function",
                        "function": { "name": "edit_file", "arguments": "{}" }
                    }]
                })),
            ),
        ]);

        assert_eq!(history[1]["role"], "assistant");
        assert!(history[1]["content"].is_null());
        assert!(history[1]["reasoning_content"]
            .as_str()
            .unwrap_or_default()
            .contains("enough context"));
        assert!(history[1]["tool_calls"].is_array());
    }

    #[test]
    fn normalize_history_strips_existing_think_tags_from_reasoning_field() {
        let history = LlmProcessor::normalize_history_messages(vec![
            message("user", "Explain it", None, None),
            message_with_reasoning(
                "assistant",
                "",
                "<think>Need to inspect the scheduler",
                Some("think"),
                None,
            ),
        ]);

        assert_eq!(
            history[1]["reasoning_content"].as_str().unwrap_or_default(),
            "Need to inspect the scheduler"
        );
    }

    #[test]
    fn normalize_history_merges_consecutive_assistant_reasoning_content() {
        let history = LlmProcessor::normalize_history_messages(vec![
            message_with_reasoning(
                "assistant",
                "First visible step",
                "First hidden step",
                Some("think"),
                None,
            ),
            message_with_reasoning(
                "assistant",
                "Second visible step",
                "Second hidden step",
                Some("think"),
                None,
            ),
        ]);

        assert_eq!(history.len(), 1);
        assert_eq!(
            history[0]["reasoning_content"].as_str().unwrap_or_default(),
            "First hidden step\n\nSecond hidden step"
        );
    }

    #[test]
    fn extract_leading_native_think_block_ignores_mid_content_tags() {
        assert!(LlmProcessor::extract_leading_native_think_block(
            "Answer first\n<think>literal tag example</think>"
        )
        .is_none());
    }
}
