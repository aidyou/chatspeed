use crate::ai::chat::openai::OpenAIChat;
use crate::ai::error::AiError;
use crate::ai::interaction::chat_completion::{AiChatEnum, ChatState};
use crate::ai::traits::chat::{ChatMetadata, WorkflowUsageAttribution};
use crate::db::WorkflowMessage;
use crate::tools::TOOL_ASK_USER;
use crate::tools::TOOL_COMPLETE_WORKFLOW;
use crate::workflow::react::context::ContextManager;
use crate::workflow::react::error::WorkflowEngineError;
use crate::workflow::react::intelligence::IntelligenceManager;
use crate::workflow::react::prompts::{
    BLOCKING_CONTEXT_COMPRESSION_PROMPT, ROLLUP_CONTEXT_COMPRESSION_PROMPT,
};

use serde_json::{json, Value};
use std::sync::Arc;
use tokio::time::{sleep, Duration};

#[derive(Clone)]
pub struct ContextCompressor {
    pub chat_state: Arc<ChatState>,
    pub provider_id: i64,
    pub model: String,
    pub workflow_usage_attribution: WorkflowUsageAttribution,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompressionMode {
    Rollup,
    Blocking,
}

impl ContextCompressor {
    pub fn new(
        chat_state: Arc<ChatState>,
        provider_id: i64,
        model: String,
        workflow_usage_attribution: WorkflowUsageAttribution,
    ) -> Self {
        Self {
            chat_state,
            provider_id,
            model,
            workflow_usage_attribution,
        }
    }

    /// Compresses messages into a single factual snapshot.
    /// This follows an incremental strategy: [Last Snapshot] + [New Messages] -> [New Snapshot]
    pub async fn compress(
        &self,
        messages: &[WorkflowMessage],
        mode: CompressionMode,
        compressed_until_message_id: i64,
        max_output_tokens: u32,
    ) -> Result<String, WorkflowEngineError> {
        if messages.is_empty() {
            return Ok(String::new());
        }

        // 1. Locate the LATEST summary to implement incremental compression
        let last_summary_idx = messages
            .iter()
            .rposition(ContextManager::is_compression_summary_message);

        // 2. Compress only the incremental boundary history. The active LLM projection
        // separately carries the original request or approved plan, current raw tail, and
        // todos; repeating user anchors here wastes pressure-handoff budget.
        let incremental_messages = if let Some(idx) = last_summary_idx {
            &messages[idx..]
        } else {
            messages
        };
        let compression_input =
            Self::incremental_compression_input_messages(messages, last_summary_idx);

        // 3. Layer 1: Purification (Filter out noise)
        let purified_history: Vec<serde_json::Value> = compression_input
            .iter()
            .filter_map(|m| {
                if Self::should_skip_message_for_compression(m) {
                    return None;
                }
                let merged_content = ContextManager::content_for_context_projection(m);
                let content = Self::sanitize_message_content_for_compression(&merged_content);
                let keep = Self::should_keep_message_content_for_compression(m, &content);
                keep.then(|| {
                    serde_json::json!({
                        "message_id": m.id,
                        "role": m.role,
                        "content": content,
                        "metadata": Self::compression_metadata(m),
                    })
                })
            })
            .collect();

        if purified_history.is_empty() {
            return Ok("No meaningful progress to compress.".to_string());
        }

        let completed_tasks = Self::render_completed_tasks(incremental_messages);
        let review_rounds = Self::render_canonical_review_rounds(messages, incremental_messages);
        let fact_pack = Self::render_fact_pack(messages, incremental_messages);
        let canonical_file_changes = Self::file_changes_from_fact_pack(&fact_pack);

        // 4. Strategic Summary (LLM Call)
        let user_prompt = match mode {
            CompressionMode::Blocking => "Create or update only the v2 pressure_handoff checkpoint from the supplied fact packs. It is boundary-scoped: do not restate the live user goal or a next action. Later raw-tail messages are authoritative deltas.",
            CompressionMode::Rollup => "Create or update only the v2 completed_task_rollup from the supplied completed historical work. Preserve supplied review rounds and typed evidence for completed work, but do not represent live task state.",
        };

        let user_prompt = format!(
            "{}\nThe required compressed_until_message_id is {}.",
            user_prompt, compressed_until_message_id
        );
        let summary = self
            .extract_fact_from_history(
                purified_history,
                &completed_tasks,
                &review_rounds,
                &fact_pack,
                &canonical_file_changes,
                &user_prompt,
                mode,
                compressed_until_message_id,
                max_output_tokens,
            )
            .await?;
        Ok(summary)
    }

    async fn extract_fact_from_history(
        &self,
        history_json: Vec<serde_json::Value>,
        completed_tasks: &str,
        review_rounds: &str,
        fact_pack: &str,
        canonical_file_changes: &[String],
        user_prompt: &str,
        mode: CompressionMode,
        compressed_until_message_id: i64,
        max_output_tokens: u32,
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
                        "<completed_tasks>\n{}\n</completed_tasks>\n\n<review_rounds>\n{}\n</review_rounds>\n\n<handoff_fact_pack>\n{}\n</handoff_fact_pack>\n\n<conversation_history>\n{}\n</conversation_history>\n\n{}{}",
                        completed_tasks,
                        review_rounds,
                        fact_pack,
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
                        max_tokens: Some(max_output_tokens),
                        workflow_usage_attribution: Some(WorkflowUsageAttribution {
                            request_kind: "compression".to_string(),
                            ..self.workflow_usage_attribution.clone()
                        }),
                        ..Default::default()
                    }),
                    |_| {},
                )
                .await
            {
                Ok(result) => {
                    // File paths are deterministic tool-observation evidence. Normalize them
                    // before validation so a weak compressor cannot waste a retry by listing a
                    // read-only or retained-tail path; semantic shape remains validated below.
                    let normalized = Self::inject_runtime_handoff_fields(
                        &Self::normalize_summary_result(&result),
                        review_rounds,
                        canonical_file_changes,
                        mode,
                        compressed_until_message_id,
                    );
                    let normalized =
                        Self::normalize_handoff_file_changes(&normalized, canonical_file_changes);
                    let validation_error = match Self::validate_compression_result(
                        &normalized,
                        completed_tasks,
                        review_rounds,
                        fact_pack,
                        mode,
                        compressed_until_message_id,
                    ) {
                        Ok(validated) => return Ok(validated),
                        Err(err) => err,
                    };

                    if attempt < max_attempts {
                        let wait_secs = 2u64.pow(attempt - 1);
                        retry_instruction = Self::build_retry_instruction(&validation_error, mode);
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

    fn incremental_compression_input_messages<'a>(
        messages: &'a [WorkflowMessage],
        last_summary_idx: Option<usize>,
    ) -> Vec<&'a WorkflowMessage> {
        let start_idx = last_summary_idx.unwrap_or(0);
        messages[start_idx..].iter().collect()
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

    pub(crate) fn should_skip_message_for_compression(message: &WorkflowMessage) -> bool {
        if message.message_subtype.as_deref() == Some("approved_plan") {
            return true;
        }

        let Some(meta) = message.metadata.as_ref() else {
            return false;
        };
        let tool_name = meta.get("tool_name").and_then(|value| value.as_str());
        let approval_status = meta.get("approval_status").and_then(|value| value.as_str());

        (tool_name == Some("submit_plan") && approval_status == Some("approved"))
            || tool_name.is_some_and(|name| name.starts_with("todo_"))
            || crate::workflow::react::runtime_observation::is_runtime_observation(Some(meta))
    }

    fn compression_metadata(message: &WorkflowMessage) -> serde_json::Value {
        let Some(metadata) = message.metadata.as_ref() else {
            return json!({});
        };
        let mut compact = serde_json::Map::new();
        for key in ["review_display_state", "review_summary", "review_verdict"] {
            if let Some(value) = metadata.get(key) {
                compact.insert(key.to_string(), value.clone());
            }
        }
        if matches!(
            metadata.get("execution_status").and_then(Value::as_str),
            Some("failed" | "rejected" | "interrupted")
        ) {
            compact.insert(
                "execution_status".to_string(),
                metadata
                    .get("execution_status")
                    .cloned()
                    .unwrap_or_default(),
            );
        }
        if message.is_error {
            if let Some(error_type) = metadata
                .get("error_type")
                .cloned()
                .or_else(|| message.error_type.as_ref().map(|value| json!(value)))
            {
                compact.insert("error_type".to_string(), error_type);
            }
        }
        serde_json::Value::Object(compact)
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
                let escaped_role = Self::escape_xml_text(role);
                let escaped_content = Self::escape_xml_text(content);
                let metadata = message
                    .get("metadata")
                    .filter(|metadata| {
                        !metadata.as_object().is_some_and(|object| object.is_empty())
                    })
                    .map(|metadata| {
                        format!(
                            "\n<metadata>{}</metadata>",
                            Self::escape_xml_text(&metadata.to_string())
                        )
                    })
                    .unwrap_or_default();
                Some(format!(
                    "<message role=\"{}\">\n{}{}\n</message>",
                    escaped_role, escaped_content, metadata
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
                .filter(|message| ContextManager::is_user_authored_task_message(message))
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

    fn render_canonical_review_rounds(
        all_messages: &[WorkflowMessage],
        incremental_messages: &[WorkflowMessage],
    ) -> String {
        let mut rounds = Vec::new();
        for message in all_messages {
            if ContextManager::is_compression_summary_message(message) {
                if let Some(summary) = Self::parse_v2_handoff(message) {
                    if let Some(previous_rounds) = summary
                        .get("review_rounds")
                        .and_then(|value| value.as_array())
                    {
                        rounds.extend(previous_rounds.iter().cloned().map(|mut round| {
                            let approved = round
                                .get("verdict")
                                .and_then(|verdict| verdict.get("approved"))
                                .and_then(serde_json::Value::as_bool)
                                == Some(true);
                            if approved
                                && round
                                    .get("resolution_status")
                                    .and_then(serde_json::Value::as_str)
                                    == Some("verified_by_re_review")
                            {
                                round["resolution_status"] = json!("approved_review");
                            }
                            round
                        }));
                    }
                }
            }
        }
        for message in incremental_messages {
            let Some(metadata) = message.metadata.as_ref() else {
                continue;
            };
            let Some(display_state) = metadata
                .get("review_display_state")
                .and_then(|value| value.as_str())
            else {
                continue;
            };
            if !matches!(
                display_state,
                "final_review_rejected" | "final_review_approved"
            ) {
                continue;
            }
            let Some(message_id) = message.id else {
                continue;
            };
            let verdict = metadata.get("review_verdict").cloned().unwrap_or_else(|| {
                json!({
                    "approved": display_state == "final_review_approved",
                    "summary": metadata.get("review_summary").cloned().unwrap_or_default(),
                    "findings": [],
                    "required_fixes": [],
                })
            });
            let findings = verdict
                .get("findings")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default();
            let required_fixes = verdict
                .get("required_fixes")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default();
            if display_state == "final_review_approved" {
                for prior_round in &mut rounds {
                    let Some(_rejected_round_id) =
                        prior_round.get("id").and_then(serde_json::Value::as_i64)
                    else {
                        continue;
                    };
                    let rejected = prior_round
                        .get("verdict")
                        .and_then(|verdict| verdict.get("approved"))
                        .and_then(serde_json::Value::as_bool)
                        == Some(false);
                    if !rejected
                        || prior_round.get("resolution_status").and_then(Value::as_str)
                            == Some("verified_by_re_review")
                    {
                        continue;
                    }
                    // A final-review approval is the durable semantic evidence that the same
                    // reviewer contract re-evaluated the earlier required fixes. Generic edit or
                    // test messages remain candidates only and never establish verification.
                    prior_round["resolution_status"] = json!("verified_by_re_review");
                    prior_round["evidence_refs"] = json!([message_id]);
                }
            }
            rounds.push(json!({
                "id": message_id,
                "verdict": verdict,
                "findings": findings,
                "required_fixes": required_fixes,
                "evidence_refs": [],
                "resolution_status": if display_state == "final_review_approved" { "approved_review" } else { "open" },
            }));
        }
        let mut unique = std::collections::BTreeMap::new();
        for round in rounds {
            if let Some(id) = round.get("id").and_then(|value| value.as_i64()) {
                unique.entry(id).or_insert(round);
            }
        }
        let values = unique.into_values().collect::<Vec<_>>();
        let latest_approved_id = values.iter().rev().find_map(|round| {
            (round
                .get("verdict")
                .and_then(|verdict| verdict.get("approved"))
                .and_then(Value::as_bool)
                == Some(true))
            .then(|| round.get("id").and_then(Value::as_i64))
            .flatten()
        });
        let latest_resolved_rejection = values.iter().rev().find(|round| {
            round
                .get("verdict")
                .and_then(|verdict| verdict.get("approved"))
                .and_then(Value::as_bool)
                == Some(false)
                && round
                    .get("resolution_status")
                    .and_then(Value::as_str)
                    .is_some_and(|status| status != "open")
        });
        let latest_resolved_rejection_id = latest_resolved_rejection
            .and_then(|round| round.get("id"))
            .and_then(Value::as_i64);
        let retained_evidence_ids = latest_resolved_rejection
            .and_then(|round| round.get("evidence_refs"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let values = values
            .into_iter()
            .filter(|round| {
                let resolution = round
                    .get("resolution_status")
                    .and_then(Value::as_str)
                    .unwrap_or("open");
                let id = round.get("id").and_then(Value::as_i64);
                resolution == "open"
                    || id == latest_resolved_rejection_id
                    || id == latest_approved_id
                    || id.is_some_and(|id| retained_evidence_ids.contains(&json!(id)))
            })
            .collect::<Vec<_>>();
        if values.is_empty() {
            "None".to_string()
        } else {
            serde_json::to_string(&values).unwrap_or_else(|_| "None".to_string())
        }
    }

    fn parse_v2_handoff(message: &WorkflowMessage) -> Option<serde_json::Value> {
        let content = message
            .message
            .strip_prefix("## Previous Context Snapshot\n")
            .unwrap_or(&message.message);
        let value = serde_json::from_str::<serde_json::Value>(content).ok()?;
        (value
            .get("schema_version")
            .and_then(|version| version.as_i64())
            == Some(2))
        .then_some(value)
    }

    fn structured_tool_arguments(metadata: &Value) -> Option<Value> {
        let tool_call = metadata.get("tool_call")?;
        let function = tool_call.get("function").unwrap_or(tool_call);
        let arguments = function
            .get("arguments")
            .or_else(|| function.get("input"))?;
        match arguments {
            Value::Object(_) => Some(arguments.clone()),
            Value::String(raw) => serde_json::from_str(raw).ok(),
            _ => None,
        }
    }

    fn tool_path(metadata: &Value, arguments: Option<&Value>) -> Option<Value> {
        metadata
            .get("details")
            .and_then(|details| {
                details
                    .get("display_path")
                    .or_else(|| details.get("file_path"))
                    .or_else(|| details.get("path"))
            })
            .or_else(|| metadata.get("path"))
            .or_else(|| {
                arguments.and_then(|arguments| {
                    arguments.get("file_path").or_else(|| arguments.get("path"))
                })
            })
            .cloned()
    }

    fn is_file_mutation_tool(tool_name: Option<&str>) -> bool {
        matches!(
            tool_name,
            Some(crate::tools::TOOL_EDIT_FILE) | Some(crate::tools::TOOL_WRITE_FILE)
        )
    }

    fn normalize_handoff_path(path: &Value) -> Option<String> {
        let raw = path.as_str()?.trim().replace('\\', "/");
        if raw.is_empty() {
            return None;
        }

        let absolute = raw.starts_with('/');
        let mut components = Vec::new();
        for component in raw.split('/') {
            match component {
                "" | "." => {}
                ".." if components.last().is_some_and(|value| *value != "..") => {
                    components.pop();
                }
                ".." if !absolute => components.push(component),
                ".." => {}
                value => components.push(value),
            }
        }
        let normalized = components.join("/");
        Some(if absolute {
            format!("/{normalized}")
        } else if normalized.is_empty() {
            ".".to_string()
        } else {
            normalized
        })
    }

    fn render_fact_pack(
        all_messages: &[WorkflowMessage],
        incremental_messages: &[WorkflowMessage],
    ) -> String {
        // The handoff is an AI-to-AI status transfer, not a tool-event archive. File paths are
        // the only deterministic evidence that must survive outside semantic work summaries.
        // Start from the prior canonical handoff so a later pressure/rollup checkpoint does not
        // discard successful mutations already covered by an earlier checkpoint.
        let mut file_changes = std::collections::BTreeSet::new();
        if let Some(previous_handoff) = all_messages
            .iter()
            .rev()
            .find(|message| ContextManager::is_compression_summary_message(message))
            .and_then(Self::parse_v2_handoff)
        {
            file_changes.extend(
                previous_handoff
                    .get("file_changes")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Self::normalize_handoff_path),
            );
        }
        for message in incremental_messages {
            let Some(metadata) = message.metadata.as_ref() else {
                continue;
            };
            if Self::should_skip_message_for_compression(message) {
                continue;
            }
            let execution_status = metadata
                .get("execution_status")
                .cloned()
                .unwrap_or_else(|| {
                    json!(if message.is_error {
                        "failed"
                    } else {
                        "completed"
                    })
                });
            let arguments = Self::structured_tool_arguments(metadata);
            if message.role == "tool"
                && !message.is_error
                && execution_status.as_str() == Some("completed")
                && Self::is_file_mutation_tool(metadata.get("tool_name").and_then(Value::as_str))
            {
                if let Some(path) = Self::tool_path(metadata, arguments.as_ref())
                    .as_ref()
                    .and_then(Self::normalize_handoff_path)
                {
                    file_changes.insert(path);
                }
            }
        }
        let paths = file_changes.into_iter().collect::<Vec<_>>();
        serde_json::to_string(&json!({ "file_changes": paths }))
            .unwrap_or_else(|_| "{\"file_changes\":[]}".to_string())
    }

    fn file_changes_from_fact_pack(fact_pack: &str) -> Vec<String> {
        serde_json::from_str::<Value>(fact_pack)
            .ok()
            .and_then(|facts| facts.get("file_changes").and_then(Value::as_array).cloned())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|path| Self::normalize_handoff_path(&path))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
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
            .map(|tool_name| tool_name == TOOL_COMPLETE_WORKFLOW)
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

    fn normalize_handoff_file_changes(result: &str, canonical_file_changes: &[String]) -> String {
        let Ok(mut handoff) = serde_json::from_str::<Value>(result) else {
            return result.to_string();
        };
        // Tool observations are scoped to this boundary; a worktree diff can include unrelated work.
        let canonical_paths = canonical_file_changes
            .iter()
            .filter_map(|path| Self::normalize_handoff_path(&Value::String(path.clone())))
            .collect::<std::collections::BTreeSet<_>>();
        handoff["file_changes"] = json!(canonical_paths);
        if handoff.get("kind").and_then(Value::as_str) == Some("pressure_handoff") {
            // Normalize legacy constraint fields at this compatibility boundary so new handoffs
            // have one unambiguous field without invalidating old persisted summaries.
            let mut constraints = std::collections::BTreeSet::new();
            for field in [
                "constraints_and_guards",
                "technical_invariants",
                "warnings_and_do_not_repeat",
                "environment_constraints",
            ] {
                if let Some(entries) = handoff.get(field).and_then(Value::as_array) {
                    constraints.extend(
                        entries
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::trim)
                            .filter(|entry| !entry.is_empty()),
                    );
                }
            }
            handoff["constraints_and_guards"] = json!(constraints);
            if let Some(object) = handoff.as_object_mut() {
                object.remove("technical_invariants");
                object.remove("warnings_and_do_not_repeat");
                object.remove("environment_constraints");
            }

            let user_directives = handoff
                .get("user_directives")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|directive| !directive.is_empty())
                .collect::<std::collections::BTreeSet<_>>();
            handoff["user_directives"] = json!(user_directives);

            let mut boundary_open_items = std::collections::BTreeSet::new();
            if let Some(items) = handoff.get("boundary_open_items").and_then(Value::as_array) {
                for item in items {
                    if let (Some(kind), Some(summary)) = (
                        item.get("kind").and_then(Value::as_str),
                        item.get("summary").and_then(Value::as_str),
                    ) {
                        boundary_open_items
                            .insert((kind.trim().to_string(), summary.trim().to_string()));
                    }
                }
            }
            for (field, kind) in [
                ("facts_to_verify", "verification"),
                ("open_threads_at_boundary", "remediation"),
            ] {
                if let Some(entries) = handoff.get(field).and_then(Value::as_array) {
                    for summary in entries.iter().filter_map(Value::as_str).map(str::trim) {
                        if !summary.is_empty() {
                            boundary_open_items.insert((kind.to_string(), summary.to_string()));
                        }
                    }
                }
            }
            handoff["boundary_open_items"] = json!(boundary_open_items
                .into_iter()
                .map(|(kind, summary)| json!({"kind": kind, "summary": summary}))
                .collect::<Vec<_>>());
            if let Some(object) = handoff.as_object_mut() {
                object.remove("facts_to_verify");
                object.remove("open_threads_at_boundary");
                object.remove("unresolved_items");
                object.remove("credential_references");
            }
        }
        serde_json::to_string(&handoff).unwrap_or_else(|_| result.to_string())
    }

    fn inject_runtime_handoff_fields(
        result: &str,
        review_rounds: &str,
        canonical_file_changes: &[String],
        mode: CompressionMode,
        compressed_until_message_id: i64,
    ) -> String {
        let Ok(mut handoff) = serde_json::from_str::<Value>(result) else {
            return result.to_string();
        };
        let Some(object) = handoff.as_object_mut() else {
            return result.to_string();
        };

        let canonical_paths = canonical_file_changes
            .iter()
            .filter_map(|path| Self::normalize_handoff_path(&Value::String(path.clone())))
            .collect::<std::collections::BTreeSet<_>>();
        let canonical_review_rounds = if review_rounds == "None" {
            Value::Array(Vec::new())
        } else {
            serde_json::from_str(review_rounds).unwrap_or_else(|_| Value::Array(Vec::new()))
        };

        object.insert("schema_version".to_string(), json!(2));
        object.insert(
            "kind".to_string(),
            json!(match mode {
                CompressionMode::Blocking => "pressure_handoff",
                CompressionMode::Rollup => "completed_task_rollup",
            }),
        );
        object.insert("file_changes".to_string(), json!(canonical_paths));
        object.insert("review_rounds".to_string(), canonical_review_rounds);
        if matches!(mode, CompressionMode::Blocking) {
            object.insert(
                "as_of_boundary".to_string(),
                json!({ "compressed_until_message_id": compressed_until_message_id }),
            );
        }
        serde_json::to_string(&handoff).unwrap_or_else(|_| result.to_string())
    }

    fn normalize_summary_result(result: &str) -> String {
        let trimmed = result.trim();
        let trimmed = trimmed
            .strip_prefix("```json")
            .or_else(|| trimmed.strip_prefix("```JSON"))
            .or_else(|| trimmed.strip_prefix("```"))
            .map_or(trimmed, |fenced| {
                fenced.trim().trim_end_matches("```").trim()
            });
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if let Some(content) = parsed.get("content") {
                match content {
                    serde_json::Value::String(text) => {
                        return Self::normalize_summary_result(text);
                    }
                    serde_json::Value::Object(_) => {
                        if let Ok(text) = serde_json::to_string_pretty(content) {
                            return text;
                        }
                    }
                    _ => return String::new(),
                }
            }

            if parsed.is_object() {
                return serde_json::to_string_pretty(&parsed).unwrap_or_default();
            }

            return trimmed.to_string();
        }

        trimmed.to_string()
    }

    #[cfg(test)]
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

    fn validate_compression_result(
        normalized: &str,
        _completed_tasks: &str,
        review_rounds: &str,
        fact_pack: &str,
        mode: CompressionMode,
        compressed_until_message_id: i64,
    ) -> Result<String, String> {
        let parsed = serde_json::from_str::<serde_json::Value>(normalized)
            .map_err(|_| "new compression output must be a v2 JSON object".to_string())?;
        Self::validate_v2_summary(
            &parsed,
            review_rounds,
            fact_pack,
            mode,
            compressed_until_message_id,
        )
    }

    #[cfg(test)]
    fn validate_summary_result_with_completed_tasks(
        normalized: &str,
        completed_tasks: &str,
    ) -> Result<String, String> {
        let validated = Self::validate_summary_result(normalized)?;
        if completed_tasks.trim() == "None" {
            return Ok(validated);
        }
        let parsed: serde_json::Value = serde_json::from_str(&validated).map_err(|_| {
            "legacy completed-task carryover requires JSON snapshot output".to_string()
        })?;
        let latest_task_index = completed_tasks
            .rsplit_once("<task_index>")
            .and_then(|(_, rest)| rest.split_once("</task_index>"))
            .and_then(|(index, _)| index.trim().parse::<i64>().ok())
            .ok_or_else(|| "completed-task archive is missing the latest task_index".to_string())?;
        let preserved = parsed["prev_tasks"].as_array().is_some_and(|tasks| {
            tasks.iter().any(|task| {
                task.get("task_index").and_then(serde_json::Value::as_i64)
                    == Some(latest_task_index)
            })
        });
        preserved
            .then_some(validated)
            .ok_or_else(|| "legacy prev_tasks omitted latest completed task".to_string())
    }

    fn validate_v2_summary(
        parsed: &serde_json::Value,
        review_rounds: &str,
        fact_pack: &str,
        mode: CompressionMode,
        compressed_until_message_id: i64,
    ) -> Result<String, String> {
        let object = parsed
            .as_object()
            .ok_or_else(|| "response must be a JSON object".to_string())?;
        if object
            .get("schema_version")
            .and_then(|value| value.as_i64())
            != Some(2)
        {
            return Err("schema_version must be 2".to_string());
        }

        let (kind, required_fields): (&str, Vec<&str>) = match mode {
            CompressionMode::Blocking => (
                "pressure_handoff",
                vec![
                    "as_of_boundary",
                    "user_directives",
                    "confirmed_facts",
                    "boundary_open_items",
                    "completed_work",
                    "file_changes",
                    "constraints_and_guards",
                    "review_rounds",
                ],
            ),
            CompressionMode::Rollup => (
                "completed_task_rollup",
                vec![
                    "confirmed_facts",
                    "unresolved_carryovers",
                    "completed_work",
                    "file_changes",
                    "constraints_and_guards",
                    "review_rounds",
                ],
            ),
        };
        if object.get("kind").and_then(|value| value.as_str()) != Some(kind) {
            return Err(format!("kind must be {kind}"));
        }
        if matches!(mode, CompressionMode::Blocking)
            && object
                .get("as_of_boundary")
                .and_then(|value| value.get("compressed_until_message_id"))
                .and_then(|value| value.as_i64())
                != Some(compressed_until_message_id)
        {
            return Err("as_of_boundary.compressed_until_message_id is incorrect".to_string());
        }
        for field in &required_fields {
            let field = *field;
            if field != "as_of_boundary"
                && !object.get(field).is_some_and(serde_json::Value::is_array)
            {
                return Err(format!("{field} must be an array"));
            }
        }
        if let Some(items) = object
            .get("boundary_open_items")
            .and_then(serde_json::Value::as_array)
        {
            for (index, item) in items.iter().enumerate() {
                let kind = item
                    .get("kind")
                    .and_then(serde_json::Value::as_str)
                    .filter(|kind| matches!(*kind, "verification" | "decision" | "remediation"))
                    .ok_or_else(|| format!("boundary_open_items[{index}].kind is invalid"))?;
                let _ = kind;
                Self::validate_non_empty_string_field(
                    item.get("summary").ok_or_else(|| {
                        format!("boundary_open_items[{index}].summary is required")
                    })?,
                    &format!("boundary_open_items[{index}].summary"),
                )?;
            }
        }
        match mode {
            CompressionMode::Blocking => {
                Self::validate_semantic_string_arrays(
                    object,
                    &[
                        "user_directives",
                        "confirmed_facts",
                        "completed_work",
                        "constraints_and_guards",
                    ],
                )?;
                Self::validate_semantic_entry_limits(
                    object,
                    &[
                        ("user_directives", 2),
                        ("confirmed_facts", 4),
                        ("boundary_open_items", 3),
                        ("completed_work", 2),
                        ("constraints_and_guards", 3),
                    ],
                )?;
            }
            CompressionMode::Rollup => {
                Self::validate_semantic_string_arrays(
                    object,
                    &[
                        "confirmed_facts",
                        "completed_work",
                        "unresolved_carryovers",
                        "constraints_and_guards",
                    ],
                )?;
                Self::validate_semantic_entry_limits(
                    object,
                    &[
                        ("confirmed_facts", 4),
                        ("unresolved_carryovers", 3),
                        ("completed_work", 2),
                        ("constraints_and_guards", 3),
                    ],
                )?;
                for obsolete in [
                    "completed_tasks",
                    "durable_decisions",
                    "cross_task_constraints",
                    "warnings_and_do_not_repeat",
                    "environment_constraints",
                    "credential_references",
                ] {
                    if object.contains_key(obsolete) {
                        return Err(format!(
                            "{obsolete} is not part of the semantic rollup contract"
                        ));
                    }
                }
            }
        }
        for prohibited in [
            "next_action",
            "todos",
            "task_state",
            "overall_goal",
            "approved_plan",
        ] {
            if object.contains_key(prohibited) {
                return Err(format!("{prohibited} is prohibited in v2 handoff"));
            }
        }
        Self::validate_review_rounds(object, review_rounds)?;
        Self::validate_fact_pack(object, fact_pack)?;
        serde_json::to_string_pretty(parsed)
            .map_err(|error| format!("failed to normalize v2 handoff json: {error}"))
    }

    fn validate_review_rounds(
        object: &serde_json::Map<String, serde_json::Value>,
        review_rounds: &str,
    ) -> Result<(), String> {
        if review_rounds == "None" {
            return Ok(());
        }
        let input_rounds = serde_json::from_str::<Vec<serde_json::Value>>(review_rounds)
            .map_err(|_| "review_rounds fact pack must be JSON".to_string())?;
        let output_rounds = object
            .get("review_rounds")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| "review_rounds must be an array".to_string())?;
        for (index, input) in input_rounds.iter().enumerate() {
            let input_id = input
                .get("id")
                .and_then(serde_json::Value::as_i64)
                .ok_or_else(|| "input review round is missing id".to_string())?;
            let output = output_rounds
                .iter()
                .find(|round| round.get("id").and_then(serde_json::Value::as_i64) == Some(input_id))
                .ok_or_else(|| format!("review_rounds omitted input round {input_id}"))?;
            for field in ["verdict", "findings", "required_fixes"] {
                if output.get(field) != input.get(field) {
                    return Err(format!("review_round {input_id} must preserve {field}"));
                }
            }
            let input_evidence = input
                .get("evidence_refs")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| format!("review_round {input_id} missing input evidence_refs"))?;
            let output_evidence = output
                .get("evidence_refs")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| format!("review_round {input_id} missing output evidence_refs"))?;
            if !input_evidence
                .iter()
                .all(|evidence| output_evidence.contains(evidence))
            {
                return Err(format!("review_round {input_id} must retain evidence_refs"));
            }
            let input_resolution = input
                .get("resolution_status")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| format!("review_round {input_id} missing resolution_status"))?;
            let output_resolution = output
                .get("resolution_status")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    format!("review_round {input_id} resolution_status must be a string")
                })?;
            let input_is_rejected = input
                .get("verdict")
                .and_then(|verdict| verdict.get("approved"))
                .and_then(serde_json::Value::as_bool)
                == Some(false);
            let later_approved_round_id = input_rounds
                .iter()
                .skip(index + 1)
                .find(|round| {
                    round
                        .get("verdict")
                        .and_then(|verdict| verdict.get("approved"))
                        .and_then(serde_json::Value::as_bool)
                        == Some(true)
                })
                .and_then(|round| round.get("id").and_then(serde_json::Value::as_i64));
            let evidence_is_review_window = output_evidence.iter().all(|evidence| {
                evidence
                    .as_i64()
                    .is_some_and(|id| id > input_id && Some(id) <= later_approved_round_id)
            });
            let evidence_contains_approved_review = later_approved_round_id
                .is_some_and(|approved_id| output_evidence.contains(&json!(approved_id)));
            if output_resolution == "verified_by_re_review"
                && (input_resolution != "verified_by_re_review"
                    || !input_is_rejected
                    || later_approved_round_id.is_none()
                    || output_evidence.is_empty()
                    || !evidence_is_review_window
                    || !evidence_contains_approved_review)
            {
                return Err(format!("review_round {input_id} cannot be verified_by_re_review without canonical evidence at or before a later approved review"));
            }
            if input_resolution == "verified_by_re_review" && output_resolution != input_resolution
            {
                return Err(format!(
                    "review_round {input_id} cannot regress resolution_status"
                ));
            }
        }
        Ok(())
    }

    fn validate_fact_pack(
        object: &serde_json::Map<String, serde_json::Value>,
        fact_pack: &str,
    ) -> Result<(), String> {
        let facts = serde_json::from_str::<Value>(fact_pack)
            .map_err(|_| "handoff fact pack must be JSON".to_string())?;
        let canonical_paths = facts
            .get("file_changes")
            .and_then(Value::as_array)
            .ok_or_else(|| "handoff fact pack file_changes must be an array".to_string())?;
        if !canonical_paths.iter().all(Value::is_string) {
            return Err("handoff fact pack file_changes must contain paths".to_string());
        }

        let output_paths = object
            .get("file_changes")
            .and_then(Value::as_array)
            .ok_or_else(|| "file_changes must be an array".to_string())?;
        if !output_paths.iter().all(Value::is_string) {
            return Err("file_changes must be a compact array of paths".to_string());
        }
        let canonical_paths = canonical_paths
            .iter()
            .filter_map(Value::as_str)
            .collect::<std::collections::BTreeSet<_>>();
        if !output_paths
            .iter()
            .filter_map(Value::as_str)
            .all(|path| canonical_paths.contains(path))
        {
            return Err("file_changes contains a path outside the canonical fact pack".to_string());
        }
        Ok(())
    }

    fn validate_semantic_string_arrays(
        object: &serde_json::Map<String, serde_json::Value>,
        fields: &[&str],
    ) -> Result<(), String> {
        for field in fields {
            let entries = object
                .get(*field)
                .and_then(Value::as_array)
                .ok_or_else(|| format!("{field} must be an array"))?;
            for (index, entry) in entries.iter().enumerate() {
                Self::validate_non_empty_string_field(entry, &format!("{field}[{index}]"))?;
            }
        }
        Ok(())
    }

    fn validate_semantic_entry_limits(
        object: &serde_json::Map<String, serde_json::Value>,
        limits: &[(&str, usize)],
    ) -> Result<(), String> {
        for (field, maximum) in limits {
            let count = object
                .get(*field)
                .and_then(Value::as_array)
                .map(Vec::len)
                .ok_or_else(|| format!("{field} must be an array"))?;
            if count > *maximum {
                return Err(format!("{field} must contain at most {maximum} entries"));
            }
        }
        Ok(())
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

    #[cfg(test)]
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

    #[cfg(test)]
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

    #[cfg(test)]
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

    fn build_retry_instruction(validation_error: &str, mode: CompressionMode) -> String {
        let schema = match mode {
            CompressionMode::Blocking => "user_directives, confirmed_facts, boundary_open_items, completed_work, constraints_and_guards",
            CompressionMode::Rollup => {
                "confirmed_facts, completed_work, unresolved_carryovers, constraints_and_guards"
            }
        };
        let item_shapes = match mode {
            CompressionMode::Blocking => "user_directives, confirmed_facts, completed_work, and constraints_and_guards must be compact string arrays; boundary_open_items must contain only {kind, summary}. Limits: user_directives=2, confirmed_facts=4, boundary_open_items=3, completed_work=2, constraints_and_guards=3.",
            CompressionMode::Rollup => "confirmed_facts, completed_work, unresolved_carryovers, and constraints_and_guards must be compact string arrays. Limits: confirmed_facts=4, unresolved_carryovers=3, completed_work=2, constraints_and_guards=3.",
        };
        format!(
            "\n\n<SYSTEM_REMINDER>Your previous compression reply was invalid. Reason: {}. Return exactly one semantic JSON object and nothing else. Required semantic fields: {}. Required item shapes: {} The runtime adds schema_version, kind, boundary, canonical file_changes, and review_rounds; do not emit those system-owned fields. Every listed array key must be present, but any semantic array may be []; never invent a verification, change, constraint, directive, open item, or review to fill one. Do not emit legacy state_snapshot, prev_tasks, task_state, todos, next_action, approved_plan, or overall_goal. Do NOT return XML, reasoning-only text, markdown fences, or explanations.</SYSTEM_REMINDER>",
            validation_error, schema, item_shapes
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
    use super::{CompressionMode, ContextCompressor};
    use crate::{
        db::WorkflowMessage,
        workflow::react::{
            constants::TASK_FINISHED,
            prompts::{BLOCKING_CONTEXT_COMPRESSION_PROMPT, ROLLUP_CONTEXT_COMPRESSION_PROMPT},
        },
    };
    use serde_json::{json, Value};

    #[test]
    fn normalize_handoff_file_changes_replaces_model_output_with_canonical_paths() {
        let handoff = json!({
            "schema_version": 2,
            "file_changes": [
                "src/fabricated.rs"
            ],
            "completed_work": ["Inspected and updated the implementation"]
        })
        .to_string();
        let normalized = ContextCompressor::normalize_handoff_file_changes(
            &handoff,
            &[
                "./src/feature.rs".to_string(),
                "src/feature.rs".to_string(),
                "src/tmp/../other.rs".to_string(),
            ],
        );
        let value: Value = serde_json::from_str(&normalized).expect("normalized handoff");
        assert_eq!(
            value["file_changes"],
            json!(["src/feature.rs", "src/other.rs"])
        );
        assert_eq!(
            value["completed_work"],
            json!(["Inspected and updated the implementation"])
        );
    }

    #[test]
    fn retry_instruction_uses_semantic_completed_work_contract() {
        let blocking = ContextCompressor::build_retry_instruction(
            "completed_work must contain at most 2 entries",
            CompressionMode::Blocking,
        );
        assert!(blocking
            .contains("completed_work, and constraints_and_guards must be compact string arrays"));
        assert!(blocking.contains("completed_work=2"));
        assert!(!blocking.contains("{summary:string, files:string[]}"));

        let rollup = ContextCompressor::build_retry_instruction(
            "completed_work must be an array",
            CompressionMode::Rollup,
        );
        assert!(rollup.contains("completed_work, unresolved_carryovers"));
        assert!(rollup.contains("completed_work=2"));
    }

    #[test]
    fn runtime_injects_rollup_mechanical_fields() {
        let semantic = json!({
            "confirmed_facts": ["The local guard is required"],
            "completed_work": ["Added the guard"],
            "unresolved_carryovers": [],
            "constraints_and_guards": []
        })
        .to_string();
        let injected = ContextCompressor::inject_runtime_handoff_fields(
            &semantic,
            "None",
            &["src/feature.rs".to_string()],
            CompressionMode::Rollup,
            42,
        );
        let value: Value = serde_json::from_str(&injected).expect("injected rollup");
        assert_eq!(value["schema_version"], json!(2));
        assert_eq!(value["kind"], json!("completed_task_rollup"));
        assert_eq!(value["file_changes"], json!(["src/feature.rs"]));
        assert_eq!(value["review_rounds"], json!([]));
        assert!(value.get("as_of_boundary").is_none());
    }

    #[test]
    fn runtime_injects_pressure_mechanical_fields() {
        let semantic = json!({
            "user_directives": ["Keep the current API contract"],
            "confirmed_facts": ["The local guard is required"],
            "boundary_open_items": [{"kind": "verification", "summary": "Run the runtime check"}],
            "completed_work": ["Added the guard"],
            "constraints_and_guards": []
        })
        .to_string();
        let injected = ContextCompressor::inject_runtime_handoff_fields(
            &semantic,
            "None",
            &["src/feature.rs".to_string()],
            CompressionMode::Blocking,
            42,
        );
        let value: Value = serde_json::from_str(&injected).expect("injected pressure handoff");
        assert_eq!(value["schema_version"], json!(2));
        assert_eq!(value["kind"], json!("pressure_handoff"));
        assert_eq!(
            value["as_of_boundary"],
            json!({"compressed_until_message_id": 42})
        );
        assert_eq!(value["file_changes"], json!(["src/feature.rs"]));
        assert_eq!(value["review_rounds"], json!([]));
    }

    #[test]
    fn normalize_handoff_constraints_consolidates_legacy_fields() {
        let handoff = json!({
            "schema_version": 2,
            "kind": "pressure_handoff",
            "file_changes": [],
            "user_directives": ["Keep the existing API contract"],
            "facts_to_verify": ["Verify the production migration"],
            "open_threads_at_boundary": ["Repair the historical records"],
            "technical_invariants": ["guard local identity"],
            "warnings_and_do_not_repeat": ["do not bulk delete"],
            "environment_constraints": ["php is unavailable"]
        })
        .to_string();
        let normalized = ContextCompressor::normalize_handoff_file_changes(&handoff, &[]);
        let value: Value = serde_json::from_str(&normalized).expect("normalized handoff");
        assert_eq!(
            value["constraints_and_guards"],
            json!([
                "do not bulk delete",
                "guard local identity",
                "php is unavailable"
            ])
        );
        assert!(value.get("technical_invariants").is_none());
        assert!(value.get("warnings_and_do_not_repeat").is_none());
        assert!(value.get("environment_constraints").is_none());
        assert_eq!(
            value["user_directives"],
            json!(["Keep the existing API contract"])
        );
        assert_eq!(
            value["boundary_open_items"],
            json!([
                {"kind": "remediation", "summary": "Repair the historical records"},
                {"kind": "verification", "summary": "Verify the production migration"}
            ])
        );
        assert!(value.get("facts_to_verify").is_none());
        assert!(value.get("open_threads_at_boundary").is_none());
    }

    #[test]
    fn minimal_pressure_handoff_accepts_all_empty_semantic_arrays() {
        let handoff = json!({
            "schema_version": 2,
            "kind": "pressure_handoff",
            "as_of_boundary": {"compressed_until_message_id": 42},
            "user_directives": [],
            "confirmed_facts": [],
            "boundary_open_items": [],
            "completed_work": [],
            "file_changes": [],
            "constraints_and_guards": [],
            "review_rounds": []
        })
        .to_string();
        assert!(ContextCompressor::validate_compression_result(
            &handoff,
            "None",
            "None",
            "{\"file_changes\":[]}",
            CompressionMode::Blocking,
            42,
        )
        .is_ok());
    }

    #[test]
    fn pressure_handoff_prompt_does_not_imply_task_specific_required_items() {
        assert!(BLOCKING_CONTEXT_COMPRESSION_PROMPT.contains("\"boundary_open_items\": []"));
        assert!(BLOCKING_CONTEXT_COMPRESSION_PROMPT.contains("\"completed_work\": []"));
        assert!(BLOCKING_CONTEXT_COMPRESSION_PROMPT
            .contains("The runtime, not you, adds schema version, kind, compression boundary"));
        assert!(BLOCKING_CONTEXT_COMPRESSION_PROMPT.contains(
            "user_directives`: an explicit later user correction, preference, or decision"
        ));
        assert!(BLOCKING_CONTEXT_COMPRESSION_PROMPT
            .contains("confirmed_facts`: evidence-backed behavior, root causes, or decisions"));
        assert!(BLOCKING_CONTEXT_COMPRESSION_PROMPT
            .contains("boundary_open_items`: material work unresolved at this boundary"));
        assert!(BLOCKING_CONTEXT_COMPRESSION_PROMPT
            .contains("completed_work`: compact strings for outcomes actually completed"));
        assert!(BLOCKING_CONTEXT_COMPRESSION_PROMPT.contains(
            "constraints_and_guards`: future-facing rules, prohibited actions, or static limitations"
        ));
        assert!(BLOCKING_CONTEXT_COMPRESSION_PROMPT
            .contains("scan both the completed-task summaries and conversation history"));
        assert!(BLOCKING_CONTEXT_COMPRESSION_PROMPT.contains(
            "external interface or third-party contract marked unconfirmed must be a `verification` item"
        ));
    }

    #[test]
    fn rollup_prompt_requires_compact_semantic_units() {
        assert!(ROLLUP_CONTEXT_COMPRESSION_PROMPT
            .contains("This is an AI-to-AI memory checkpoint, not a tool-event archive"));
        assert!(ROLLUP_CONTEXT_COMPRESSION_PROMPT
            .contains("The runtime, not you, adds schema version, kind"));
        assert!(ROLLUP_CONTEXT_COMPRESSION_PROMPT.contains("Return only this semantic schema"));
        assert!(ROLLUP_CONTEXT_COMPRESSION_PROMPT
            .contains("completed_work`: compact strings for completed outcomes"));
        assert!(ROLLUP_CONTEXT_COMPRESSION_PROMPT
            .contains("Do not emit or restate those system-owned fields"));
    }

    #[test]
    fn rollup_v2_requires_compact_semantic_strings_and_canonical_paths() {
        let fact_pack = json!({"file_changes": ["src/feature.rs"]}).to_string();
        let rollup = json!({
            "schema_version": 2,
            "kind": "completed_task_rollup",
            "confirmed_facts": ["Cached sessions require local user validation"],
            "unresolved_carryovers": ["Run the runtime check"],
            "completed_work": ["Added the local-user guard"],
            "file_changes": ["src/feature.rs"],
            "constraints_and_guards": [],
            "review_rounds": []
        })
        .to_string();
        assert!(ContextCompressor::validate_compression_result(
            &rollup,
            "None",
            "None",
            &fact_pack,
            CompressionMode::Rollup,
            0,
        )
        .is_ok());

        let mut transcript_archive: Value = serde_json::from_str(&rollup).expect("rollup json");
        transcript_archive["completed_tasks"] = json!([]);
        assert!(ContextCompressor::validate_compression_result(
            &transcript_archive.to_string(),
            "None",
            "None",
            &fact_pack,
            CompressionMode::Rollup,
            0,
        )
        .is_err());

        let mut non_canonical_path: Value = serde_json::from_str(&rollup).expect("rollup json");
        non_canonical_path["file_changes"] = json!(["src/invented.rs"]);
        assert!(ContextCompressor::validate_compression_result(
            &non_canonical_path.to_string(),
            "None",
            "None",
            &fact_pack,
            CompressionMode::Rollup,
            0,
        )
        .is_err());

        let mut legacy_work_unit: Value = serde_json::from_str(&rollup).expect("rollup json");
        legacy_work_unit["completed_work"] = json!([{
            "summary": "Added the local-user guard",
            "files": ["src/feature.rs"]
        }]);
        assert!(ContextCompressor::validate_compression_result(
            &legacy_work_unit.to_string(),
            "None",
            "None",
            &fact_pack,
            CompressionMode::Rollup,
            0,
        )
        .is_err());

        let mut excessive_constraints: Value = serde_json::from_str(&rollup).expect("rollup json");
        excessive_constraints["constraints_and_guards"] = json!(["one", "two", "three", "four"]);
        assert!(ContextCompressor::validate_compression_result(
            &excessive_constraints.to_string(),
            "None",
            "None",
            &fact_pack,
            CompressionMode::Rollup,
            0,
        )
        .is_err());
    }

    #[test]
    fn normalize_summary_result_extracts_content_field_from_json() {
        let raw = r#"{"content":{"overall_goal":"goal","prev_tasks":[],"key_knowledge":[],"error_log":[],"file_system_state":[],"recent_actions":[],"task_state":"active"},"reasoning":"ignored"}"#;
        let normalized = ContextCompressor::normalize_summary_result(raw);
        assert!(normalized.contains("\"overall_goal\": \"goal\""));
    }

    #[test]
    fn normalize_summary_result_unwraps_markdown_json_fence() {
        let raw = "```json\n{\"schema_version\":2,\"kind\":\"pressure_handoff\"}\n```";
        let normalized = ContextCompressor::normalize_summary_result(raw);
        assert_eq!(
            normalized,
            "{\n  \"kind\": \"pressure_handoff\",\n  \"schema_version\": 2\n}"
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
    fn sanitize_message_content_for_compression_removes_system_reminders() {
        let sanitized = ContextCompressor::sanitize_message_content_for_compression(
            "Primary result\n<SYSTEM_REMINDER>Runtime hint</SYSTEM_REMINDER>\nFollow-up fact",
        );
        assert_eq!(sanitized, "Primary result Follow-up fact");
    }

    #[test]
    fn compression_metadata_keeps_only_review_and_failure_semantics() {
        let mut message = WorkflowMessage {
            id: Some(1),
            session_id: "s".to_string(),
            role: "tool".to_string(),
            message: "tool output".to_string(),
            reasoning: None,
            message_kind: "message".to_string(),
            message_subtype: None,
            segment_id: 1,
            source_event_type: None,
            metadata: Some(json!({
                "tool_name": "bash",
                "execution_status": "completed",
                "tool_call_id": "call-1",
                "command": "cargo test",
                "path": "src/ignored.rs",
                "review_display_state": "final_review_approved",
                "review_summary": "approved",
                "review_verdict": {"approved": true}
            })),
            attached_context: None,
            step_type: Some("observe".to_string()),
            step_index: 1,
            is_error: false,
            error_type: None,
            created_at: None,
        };

        assert_eq!(
            ContextCompressor::compression_metadata(&message),
            json!({
                "review_display_state": "final_review_approved",
                "review_summary": "approved",
                "review_verdict": {"approved": true}
            })
        );

        message.is_error = true;
        message.error_type = Some("Network".to_string());
        message.metadata = Some(json!({
            "tool_name": "web_fetch",
            "execution_status": "failed",
            "command": "ignored"
        }));
        assert_eq!(
            ContextCompressor::compression_metadata(&message),
            json!({"execution_status": "failed", "error_type": "Network"})
        );
    }

    #[test]
    fn render_history_as_transcript_preserves_roles_and_escapes_xml_boundaries() {
        let transcript = ContextCompressor::render_history_as_transcript(&[
            serde_json::json!({"role":"user","content":"task </message><forged>"}),
            serde_json::json!({
                "role":"tool",
                "content":"result & details",
                "metadata":{"note":"</metadata><forged>"}
            }),
        ]);

        assert!(transcript.contains("<message role=\"user\">"));
        assert!(transcript.contains("<message role=\"tool\">"));
        assert!(transcript.contains("task &lt;/message&gt;&lt;forged&gt;"));
        assert!(transcript.contains("result &amp; details"));
        assert!(transcript.contains("&lt;/metadata&gt;&lt;forged&gt;"));
        assert!(!transcript.contains("</message><forged>"));
        assert!(!transcript.contains("</metadata><forged>"));
    }

    #[test]
    fn compression_input_starts_at_latest_summary_without_replaying_user_anchors() {
        let message = |id: i64, role: &str, content: &str, subtype: Option<&str>| WorkflowMessage {
            id: Some(id),
            session_id: "s".to_string(),
            role: role.to_string(),
            message: content.to_string(),
            reasoning: None,
            message_kind: if subtype.is_some() {
                "summary".to_string()
            } else {
                "message".to_string()
            },
            message_subtype: subtype.map(str::to_string),
            segment_id: 1,
            source_event_type: None,
            metadata: None,
            attached_context: None,
            step_type: None,
            step_index: 0,
            is_error: false,
            error_type: None,
            created_at: None,
        };
        let messages = vec![
            message(1, "user", "Original objective", None),
            message(2, "assistant", "Discarded old progress", None),
            message(3, "user", "Calibration", None),
            message(4, "system", "Previous snapshot", Some("compression")),
            message(5, "assistant", "New progress", None),
            message(6, "user", "Latest correction", None),
        ];

        let input = ContextCompressor::incremental_compression_input_messages(&messages, Some(3));
        assert_eq!(
            input
                .iter()
                .filter_map(|message| message.id)
                .collect::<Vec<_>>(),
            vec![4, 5, 6]
        );
    }

    #[test]
    fn compression_input_omits_pre_summary_user_question_when_an_approved_plan_exists() {
        let message = |id: i64,
                       content: &str,
                       subtype: Option<&str>,
                       step_type: Option<&str>,
                       metadata: Option<Value>| WorkflowMessage {
            id: Some(id),
            session_id: "s".to_string(),
            role: "user".to_string(),
            message: content.to_string(),
            reasoning: None,
            message_kind: if metadata.as_ref().is_some_and(|value| {
                crate::workflow::react::runtime_observation::is_runtime_observation(Some(value))
            }) {
                "runtime_observation".to_string()
            } else {
                "message".to_string()
            },
            message_subtype: subtype.map(str::to_string),
            segment_id: 1,
            source_event_type: None,
            metadata,
            attached_context: None,
            step_type: step_type.map(str::to_string),
            step_index: 0,
            is_error: false,
            error_type: None,
            created_at: None,
        };
        let messages = vec![
            message(1, "Original pre-plan question", None, None, None),
            message(
                2,
                "Runtime reminder projected as user",
                None,
                Some("observe"),
                Some(crate::workflow::react::runtime_observation::runtime_observation_metadata(
                    crate::workflow::react::runtime_observation::RuntimeObservationType::GenericReminder,
                    json!({"llm_content": "runtime"}),
                )),
            ),
            message(3, "Approved plan", Some("approved_plan"), None, None),
            message(4, "Previous checkpoint", Some("compression"), None, None),
            message(5, "Execution correction", None, None, None),
        ];
        let input = ContextCompressor::incremental_compression_input_messages(&messages, Some(3));
        assert_eq!(
            input
                .iter()
                .filter_map(|message| message.id)
                .collect::<Vec<_>>(),
            vec![4, 5]
        );
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
    fn validate_summary_result_requires_latest_completed_task_carryover() {
        let completed_tasks = "<task><task_index>1</task_index><user_query>Latest finished request</user_query><result_summary>Latest finished result</result_summary></task>";
        let missing = r#"{"prev_tasks":[{"task_index":0,"brief":"older task"}],"key_knowledge":[],"error_log":[],"file_system_state":[],"recent_actions":[],"task_state":{"status":"completed_archive","current_focus":"No active task in compressed segment","next_steps":[],"open_questions":[],"blockers":[],"todos":[]}}"#;
        assert!(
            ContextCompressor::validate_summary_result_with_completed_tasks(
                missing,
                completed_tasks
            )
            .is_err()
        );

        let preserved = r#"{"prev_tasks":[{"task_index":1,"user_query":"Latest finished request","result_summary":"Latest finished result"}],"key_knowledge":[],"error_log":[],"file_system_state":[],"recent_actions":[],"task_state":{"status":"completed_archive","current_focus":"No active task in compressed segment","next_steps":[],"open_questions":[],"blockers":[],"todos":[]}}"#;
        assert!(
            ContextCompressor::validate_summary_result_with_completed_tasks(
                preserved,
                completed_tasks
            )
            .is_ok()
        );
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
                metadata: Some(serde_json::json!({ "tool_name": "complete_workflow" })),
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
                metadata: Some(serde_json::json!({ "tool_name": "complete_workflow" })),
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
    fn new_compression_rejects_legacy_snapshot_output() {
        let legacy = r#"{"prev_tasks":[],"key_knowledge":[],"error_log":[],"file_system_state":[],"recent_actions":[],"task_state":{"status":"in_progress","current_focus":"x","next_steps":[],"open_questions":[],"blockers":[],"todos":[]}}"#;
        assert!(ContextCompressor::validate_compression_result(
            legacy,
            "None",
            "None",
            "None",
            CompressionMode::Blocking,
            1,
        )
        .is_err());
        assert!(ContextCompressor::validate_summary_result(legacy).is_ok());
    }

    #[test]
    fn task_specific_handoff_arrays_may_be_empty_without_retry_failure() {
        let pressure = json!({
            "schema_version": 2,
            "kind": "pressure_handoff",
            "as_of_boundary": {"compressed_until_message_id": 7},
            "user_directives": [], "confirmed_facts": [], "boundary_open_items": [],
            "completed_work": [], "file_changes": [], "constraints_and_guards": [],
            "review_rounds": []
        })
        .to_string();
        assert!(ContextCompressor::validate_compression_result(
            &pressure,
            "None",
            "None",
            "{\"file_changes\":[]}",
            CompressionMode::Blocking,
            7,
        )
        .is_ok());

        let rollup = json!({
            "schema_version": 2,
            "kind": "completed_task_rollup",
            "confirmed_facts": [], "unresolved_carryovers": [], "completed_work": [],
            "file_changes": [], "constraints_and_guards": [], "review_rounds": []
        })
        .to_string();
        assert!(ContextCompressor::validate_compression_result(
            &rollup,
            "None",
            "None",
            "{\"file_changes\":[]}",
            CompressionMode::Rollup,
            7,
        )
        .is_ok());
    }

    #[test]
    fn v2_pressure_handoff_preserves_canonical_review_content_and_fact_evidence() {
        let input_rounds = json!([{
            "id": 17,
            "verdict": {"approved": false},
            "findings": [{"detail": "missing test"}],
            "required_fixes": ["add test"],
            "evidence_refs": [44],
            "resolution_status": "open"
        }])
        .to_string();
        let handoff = json!({
            "schema_version": 2,
            "kind": "pressure_handoff",
            "as_of_boundary": {"compressed_until_message_id": 42},
            "user_directives": [], "confirmed_facts": [], "boundary_open_items": [],
            "completed_work": [],
            "file_changes": ["src/example.rs"],
            "constraints_and_guards": [],
            "review_rounds": serde_json::from_str::<serde_json::Value>(&input_rounds).expect("round json")
        })
        .to_string();
        let fact_pack = json!({"file_changes": ["src/example.rs"]}).to_string();
        assert!(ContextCompressor::validate_compression_result(
            &handoff,
            "None",
            &input_rounds,
            &fact_pack,
            CompressionMode::Blocking,
            42,
        )
        .is_ok());
        let erased_fix = handoff.replace("add test", "");
        assert!(ContextCompressor::validate_compression_result(
            &erased_fix,
            "None",
            &input_rounds,
            &fact_pack,
            CompressionMode::Blocking,
            42,
        )
        .is_err());

        let fabricated = handoff.replace(
            "\"resolution_status\":\"open\"",
            "\"resolution_status\":\"verified_by_re_review\"",
        );
        assert!(ContextCompressor::validate_compression_result(
            &fabricated,
            "None",
            &input_rounds,
            &fact_pack,
            CompressionMode::Blocking,
            42,
        )
        .is_err());

        let invalid = handoff.replace(
            "\"compressed_until_message_id\":42",
            "\"compressed_until_message_id\":41",
        );
        assert!(ContextCompressor::validate_compression_result(
            &invalid,
            "None",
            &input_rounds,
            &fact_pack,
            CompressionMode::Blocking,
            42,
        )
        .is_err());
    }

    #[test]
    fn second_checkpoint_carries_prior_review_rounds_forward() {
        let prior_handoff = json!({
            "schema_version": 2,
            "kind": "pressure_handoff",
            "review_rounds": [{
                "id": 17,
                "verdict": {"approved": false},
                "findings": [{"detail": "missing test"}],
                "required_fixes": ["add test"],
                "evidence_refs": [44],
                "resolution_status": "open"
            }]
        });
        let summary = WorkflowMessage {
            id: Some(50),
            session_id: "s".to_string(),
            role: "system".to_string(),
            message: format!("## Previous Context Snapshot\n{prior_handoff}"),
            reasoning: None,
            message_kind: "summary".to_string(),
            message_subtype: Some("compression".to_string()),
            segment_id: 1,
            source_event_type: None,
            metadata: Some(json!({"compressed_until_message_id": 49})),
            attached_context: None,
            step_type: None,
            step_index: 0,
            is_error: false,
            error_type: None,
            created_at: None,
        };
        let later_message = WorkflowMessage {
            id: Some(51),
            session_id: "s".to_string(),
            role: "assistant".to_string(),
            message: "applied repair".to_string(),
            reasoning: None,
            message_kind: "message".to_string(),
            message_subtype: None,
            segment_id: 1,
            source_event_type: None,
            metadata: None,
            attached_context: None,
            step_type: None,
            step_index: 1,
            is_error: false,
            error_type: None,
            created_at: None,
        };
        let messages = vec![summary, later_message];
        let canonical =
            ContextCompressor::render_canonical_review_rounds(&messages, &messages[1..]);
        assert!(canonical.contains("missing test"));
        assert!(canonical.contains("add test"));

        let second_handoff = json!({
            "schema_version": 2, "kind": "pressure_handoff",
            "as_of_boundary": {"compressed_until_message_id": 51},
            "user_directives": [], "confirmed_facts": [], "boundary_open_items": [],
            "completed_work": [],
            "file_changes": ["src/example.rs"],
            "constraints_and_guards": [],
            "review_rounds": prior_handoff["review_rounds"].clone()
        })
        .to_string();
        let fact_pack = json!({"file_changes": ["src/example.rs"]}).to_string();
        assert!(ContextCompressor::validate_compression_result(
            &second_handoff,
            "None",
            &canonical,
            &fact_pack,
            CompressionMode::Blocking,
            51,
        )
        .is_ok());
    }

    #[test]
    fn v2_handoff_accepts_string_completed_work_and_rejects_legacy_objects() {
        let handoff = json!({
            "schema_version": 2, "kind": "pressure_handoff",
            "as_of_boundary": {"compressed_until_message_id": 42},
            "user_directives": [], "confirmed_facts": [],
            "boundary_open_items": [],
            "completed_work": ["Validated the workflow compression change"],
            "file_changes": ["src/lib.rs"],
            "constraints_and_guards": [],
            "review_rounds": []
        })
        .to_string();
        let facts = json!({"file_changes": ["src/lib.rs"]}).to_string();
        assert!(ContextCompressor::validate_compression_result(
            &handoff,
            "None",
            "None",
            &facts,
            CompressionMode::Blocking,
            42,
        )
        .is_ok());
        let mut legacy_work_unit: Value = serde_json::from_str(&handoff).expect("handoff json");
        legacy_work_unit["completed_work"] = json!([{
            "summary": "Validated the workflow compression change",
            "files": ["src/lib.rs"]
        }]);
        assert!(ContextCompressor::validate_compression_result(
            &legacy_work_unit.to_string(),
            "None",
            "None",
            &facts,
            CompressionMode::Blocking,
            42,
        )
        .is_err());
        let mut malformed_files: serde_json::Value =
            serde_json::from_str(&handoff).expect("handoff json");
        malformed_files["file_changes"] = json!([{"path": "src/lib.rs"}]);
        assert!(ContextCompressor::validate_compression_result(
            &malformed_files.to_string(),
            "None",
            "None",
            &facts,
            CompressionMode::Blocking,
            42,
        )
        .is_err());
    }

    #[test]
    fn fact_pack_uses_only_successful_file_mutations_in_compressed_task_segment() {
        let message = |id: i64,
                       tool_name: &str,
                       display_path: Option<&str>,
                       execution_status: &str,
                       is_error: bool| WorkflowMessage {
            id: Some(id),
            session_id: "s".to_string(),
            role: "tool".to_string(),
            message: format!("{tool_name} result"),
            reasoning: None,
            message_kind: "message".to_string(),
            message_subtype: None,
            segment_id: 1,
            source_event_type: None,
            metadata: Some(json!({
                "tool_name": tool_name,
                "details": display_path.map(|path| json!({"display_path": path})),
                "execution_status": execution_status,
                "llm_content": "focused result"
            })),
            attached_context: None,
            step_type: None,
            step_index: 0,
            is_error,
            error_type: is_error.then(|| "tool_failed".to_string()),
            created_at: None,
        };
        let messages = vec![
            message(41, "bash", Some("src/read-only.rs"), "completed", false),
            message(
                42,
                "read_file",
                Some("src/read-only.rs"),
                "completed",
                false,
            ),
            message(
                43,
                "plan_edit_note",
                Some("work/plan.md"),
                "completed",
                false,
            ),
            message(
                44,
                "edit_file",
                Some("./src/feature.rs"),
                "completed",
                false,
            ),
            message(
                45,
                "write_file",
                Some("src/tmp/../feature.rs"),
                "completed",
                false,
            ),
            message(46, "edit_file", Some("src/failed.rs"), "failed", true),
            // This successful mutation belongs to a later retained task. Passing the full
            // history must not leak it into the rollup/pressure candidate's fact pack.
            message(
                47,
                "write_file",
                Some("src/retained-task.rs"),
                "completed",
                false,
            ),
        ];
        let facts: Value = serde_json::from_str(&ContextCompressor::render_fact_pack(
            &messages,
            &messages[..6],
        ))
        .expect("fact pack json");
        assert_eq!(facts["file_changes"], json!(["src/feature.rs"]));
    }

    #[test]
    fn fact_pack_carries_forward_prior_handoff_file_changes() {
        let summary = WorkflowMessage {
            id: Some(41),
            session_id: "s".to_string(),
            role: "system".to_string(),
            message: "## Previous Context Snapshot\n{\"schema_version\":2,\"kind\":\"pressure_handoff\",\"file_changes\":[\"src/archived.rs\"]}".to_string(),
            reasoning: None,
            message_kind: "summary".to_string(),
            message_subtype: Some("compression".to_string()),
            segment_id: 1,
            source_event_type: None,
            metadata: Some(json!({"compressed_until_message_id": 40})),
            attached_context: None,
            step_type: None,
            step_index: 0,
            is_error: false,
            error_type: None,
            created_at: None,
        };
        let new_mutation = WorkflowMessage {
            id: Some(42),
            session_id: "s".to_string(),
            role: "tool".to_string(),
            message: "write_file result".to_string(),
            reasoning: None,
            message_kind: "message".to_string(),
            message_subtype: None,
            segment_id: 1,
            source_event_type: None,
            metadata: Some(json!({
                "tool_name": "write_file",
                "details": {"display_path": "src/current.rs"},
                "execution_status": "completed"
            })),
            attached_context: None,
            step_type: None,
            step_index: 1,
            is_error: false,
            error_type: None,
            created_at: None,
        };
        let messages = vec![summary, new_mutation];
        let facts: Value =
            serde_json::from_str(&ContextCompressor::render_fact_pack(&messages, &messages))
                .expect("fact pack json");
        assert_eq!(
            facts["file_changes"],
            json!(["src/archived.rs", "src/current.rs"])
        );
    }

    #[test]
    fn review_round_status_uses_later_approval_as_semantic_evidence() {
        let review_message = |id: i64, approved: bool| WorkflowMessage {
            id: Some(id),
            session_id: "s".to_string(),
            role: "tool".to_string(),
            message: "review".to_string(),
            reasoning: None,
            message_kind: "message".to_string(),
            message_subtype: None,
            segment_id: 1,
            source_event_type: None,
            metadata: Some(json!({
                "review_display_state": if approved { "final_review_approved" } else { "final_review_rejected" },
                "review_verdict": {"approved": approved, "findings": [], "required_fixes": []}
            })),
            attached_context: None,
            step_type: None,
            step_index: 0,
            is_error: false,
            error_type: None,
            created_at: None,
        };
        let repair = WorkflowMessage {
            id: Some(18),
            session_id: "s".to_string(),
            role: "tool".to_string(),
            message: "tests passed".to_string(),
            reasoning: None,
            message_kind: "message".to_string(),
            message_subtype: None,
            segment_id: 1,
            source_event_type: None,
            metadata: Some(json!({
                "tool_name": "bash",
                "command": "cargo test"
            })),
            attached_context: None,
            step_type: None,
            step_index: 0,
            is_error: false,
            error_type: None,
            created_at: None,
        };
        let rejected = review_message(17, false);
        let approved = review_message(19, true);
        let messages = vec![rejected.clone(), repair.clone(), approved.clone()];
        let canonical = ContextCompressor::render_canonical_review_rounds(&messages, &messages);
        let rounds: Vec<serde_json::Value> = serde_json::from_str(&canonical).expect("rounds json");
        assert_eq!(rounds[0]["resolution_status"], "verified_by_re_review");
        assert_eq!(rounds[0]["evidence_refs"], json!([19]));
        assert_eq!(rounds[1]["resolution_status"], "approved_review");

        let second_approved = review_message(20, true);
        let repeated_approval_messages = vec![
            rejected.clone(),
            repair.clone(),
            approved.clone(),
            second_approved,
        ];
        let canonical = ContextCompressor::render_canonical_review_rounds(
            &repeated_approval_messages,
            &repeated_approval_messages,
        );
        let rounds: Vec<Value> = serde_json::from_str(&canonical).expect("rounds json");
        assert_eq!(rounds[0]["evidence_refs"], json!([19]));

        let unrelated_repair = repair.clone();
        let unlinked_messages = vec![rejected.clone(), unrelated_repair, approved.clone()];
        let canonical = ContextCompressor::render_canonical_review_rounds(
            &unlinked_messages,
            &unlinked_messages,
        );
        let rounds: Vec<serde_json::Value> = serde_json::from_str(&canonical).expect("rounds json");
        assert_eq!(rounds[0]["resolution_status"], "verified_by_re_review");
        assert_eq!(rounds[0]["evidence_refs"], json!([19]));

        let candidate_only_repair = repair.clone();
        let candidate_only_messages =
            vec![rejected.clone(), candidate_only_repair, approved.clone()];
        let canonical = ContextCompressor::render_canonical_review_rounds(
            &candidate_only_messages,
            &candidate_only_messages,
        );
        let rounds: Vec<serde_json::Value> = serde_json::from_str(&canonical).expect("rounds json");
        assert_eq!(rounds[0]["resolution_status"], "verified_by_re_review");
        assert_eq!(rounds[0]["evidence_refs"], json!([19]));

        let approval_only = vec![approved];
        let canonical =
            ContextCompressor::render_canonical_review_rounds(&approval_only, &approval_only);
        let rounds: Vec<serde_json::Value> = serde_json::from_str(&canonical).expect("rounds json");
        assert_eq!(rounds[0]["resolution_status"], "approved_review");
    }

    #[test]
    fn review_verification_rejects_pre_rejection_evidence() {
        let review_rounds = json!([
            {
                "id": 17,
                "verdict": {"approved": false, "findings": ["missing test"], "required_fixes": ["add test"]},
                "findings": ["missing test"],
                "required_fixes": ["add test"],
                "evidence_refs": [16],
                "resolution_status": "verified_by_re_review"
            },
            {
                "id": 19,
                "verdict": {"approved": true, "findings": [], "required_fixes": []},
                "findings": [],
                "required_fixes": [],
                "evidence_refs": [],
                "resolution_status": "approved_review"
            }
        ])
        .to_string();
        let handoff = json!({
            "schema_version": 2, "kind": "pressure_handoff",
            "as_of_boundary": {"compressed_until_message_id": 20},
            "user_directives": [], "confirmed_facts": [], "boundary_open_items": [],
            "completed_work": [], "file_changes": [], "constraints_and_guards": [],
            "review_rounds": serde_json::from_str::<serde_json::Value>(&review_rounds).expect("rounds json")
        })
        .to_string();
        assert!(ContextCompressor::validate_compression_result(
            &handoff,
            "None",
            &review_rounds,
            "{\"file_changes\":[]}",
            CompressionMode::Blocking,
            20,
        )
        .is_err());

        let valid_review_rounds = review_rounds.replace("[16]", "[19]");
        let valid_handoff = handoff.replace("[16]", "[19]");
        assert!(ContextCompressor::validate_compression_result(
            &valid_handoff,
            "None",
            &valid_review_rounds,
            "{\"file_changes\":[]}",
            CompressionMode::Blocking,
            20,
        )
        .is_ok());
    }

    #[test]
    fn rollup_preserves_review_rounds_and_typed_facts() {
        let review_rounds = json!([{
            "id": 17,
            "verdict": {"approved": false, "findings": ["missing test"], "required_fixes": ["add test"]},
            "findings": ["missing test"],
            "required_fixes": ["add test"],
            "evidence_refs": [],
            "resolution_status": "open"
        }])
        .to_string();
        let fact_pack = json!({"file_changes": ["src/example.rs"]}).to_string();
        let rollup = json!({
            "schema_version": 2, "kind": "completed_task_rollup",
            "confirmed_facts": [], "unresolved_carryovers": [], "completed_work": [],
            "file_changes": ["src/example.rs"],
            "constraints_and_guards": [],
            "review_rounds": serde_json::from_str::<serde_json::Value>(&review_rounds).expect("rounds json")
        })
        .to_string();
        assert!(ContextCompressor::validate_compression_result(
            &rollup,
            "None",
            &review_rounds,
            &fact_pack,
            CompressionMode::Rollup,
            18,
        )
        .is_ok());

        let missing_review = rollup.replace(&review_rounds, "[]");
        assert!(ContextCompressor::validate_compression_result(
            &missing_review,
            "None",
            &review_rounds,
            &fact_pack,
            CompressionMode::Rollup,
            18,
        )
        .is_err());
    }

    #[test]
    fn compression_skips_approved_plan_and_todo_messages() {
        let approved_plan = WorkflowMessage {
            id: Some(1),
            session_id: "s".to_string(),
            role: "user".to_string(),
            message: "plan".to_string(),
            reasoning: None,
            message_kind: "message".to_string(),
            message_subtype: Some("approved_plan".to_string()),
            segment_id: 1,
            source_event_type: None,
            metadata: None,
            attached_context: None,
            step_type: None,
            step_index: 0,
            is_error: false,
            error_type: None,
            created_at: None,
        };
        let todo = WorkflowMessage {
            id: Some(2),
            session_id: "s".to_string(),
            role: "tool".to_string(),
            message: "todo".to_string(),
            reasoning: None,
            message_kind: "message".to_string(),
            message_subtype: None,
            segment_id: 1,
            source_event_type: None,
            metadata: Some(json!({"tool_name":"todo_update"})),
            attached_context: None,
            step_type: Some("observe".to_string()),
            step_index: 1,
            is_error: false,
            error_type: None,
            created_at: None,
        };
        assert!(ContextCompressor::should_skip_message_for_compression(
            &approved_plan
        ));
        assert!(ContextCompressor::should_skip_message_for_compression(
            &todo
        ));
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
                "tool_name": "complete_workflow",
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
