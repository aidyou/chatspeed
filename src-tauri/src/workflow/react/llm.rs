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
use crate::workflow::react::security::PathGuard;
use crate::workflow::react::skills::SkillManifest;
use crate::workflow::react::types::GatewayPayload;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};

use crate::workflow::react::prompts::{
    CORE_SYSTEM_PROMPT, DRAFTING_PROMPT, EXECUTION_MODE_PROMPT, PLANNING_MODE_PROMPT,
};

pub struct LlmProcessor {
    pub session_id: String,
    pub agent_config: Agent,
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
    pub fn new(
        session_id: String,
        agent_config: Agent,
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
        let state_snapshot = raw_history.iter().rev().find_map(|m| {
            if m.role == "system"
                && m.metadata
                    .as_ref()
                    .map_or(false, |meta| meta["type"] == "summary")
            {
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

        // 2. Retry Loop for 429 (Rate Limiting) with Exponential Backoff
        let mut retry_count = 0;
        let max_retries = 10;
        let mut last_error = None;

        while retry_count <= max_retries {
            // Check for immediate stop signal before each call
            self.check_signal(signal_rx).await?;

            let (tx, mut rx) = mpsc::channel::<Arc<crate::ai::traits::chat::ChatResponse>>(100);
            let session_id_for_rx = self.session_id.clone();
            let gateway_for_rx = gateway.clone();

            // Task to process streaming chunks
            let rx_processor = tokio::spawn(async move {
                let mut plain_text = String::new();
                let mut tool_calls_json = String::new();
                let mut full_reasoning = String::new();
                let mut final_metadata = None;

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
                            let mut tool_calls_val: serde_json::Value =
                                serde_json::from_str(&chunk.chunk).unwrap_or(serde_json::json!([]));

                            if let Some(tool_calls_array) = tool_calls_val.as_array_mut() {
                                for tool_call in tool_calls_array {
                                    if let Some(tool_call_obj) = tool_call.as_object_mut() {
                                        // Only overwrite if ID is missing or not our own generated ID
                                        let existing_id =
                                            tool_call_obj.get("id").and_then(|v| v.as_str());
                                        if existing_id.map_or(true, |id| !id.starts_with("tool_")) {
                                            tool_call_obj.insert(
                                                "id".to_string(),
                                                serde_json::json!(crate::ccproxy::get_tool_id()),
                                            );
                                        }
                                    }
                                }
                            } else if let Some(tool_wrapper) = tool_calls_val.get_mut("tool") {
                                if let Some(tool_obj) = tool_wrapper.as_object_mut() {
                                    let existing_id = tool_obj.get("id").and_then(|v| v.as_str());
                                    if existing_id.map_or(true, |id| !id.starts_with("tool_")) {
                                        tool_obj.insert(
                                            "id".to_string(),
                                            serde_json::json!(crate::ccproxy::get_tool_id()),
                                        );
                                    }
                                }
                            } else if tool_calls_val.is_object()
                                && tool_calls_val.get("name").is_some()
                            {
                                if let Some(tool_obj) = tool_calls_val.as_object_mut() {
                                    let existing_id = tool_obj.get("id").and_then(|v| v.as_str());
                                    if existing_id.map_or(true, |id| !id.starts_with("tool_")) {
                                        tool_obj.insert(
                                            "id".to_string(),
                                            serde_json::json!(crate::ccproxy::get_tool_id()),
                                        );
                                    }
                                }
                            }
                            tool_calls_json = serde_json::to_string(&tool_calls_val)
                                .unwrap_or(chunk.chunk.clone());
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
                Ok::<(String, String, String, Option<serde_json::Value>), WorkflowEngineError>((
                    plain_text,
                    tool_calls_json,
                    full_reasoning,
                    final_metadata,
                ))
            });

            let tx_for_chat = tx.clone();

            // Construct custom headers to disable silent proxy-level retries
            let custom_headers = vec![CustomHeader {
                key: "x-cs-retry-max-count".to_string(),
                value: "0".to_string(),
            }];

            // Extract model parameters (temperature, max_tokens) from agent config
            let mut temperature = None;
            let mut max_tokens = None;

            // Search through all model roles to find the one matching active_model_name
            if let Some(ref models) = self.agent_config.models {
                for model_config in [
                    models.plan.as_ref(),
                    models.act.as_ref(),
                    models.vision.as_ref(),
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

                    // --- Post-processing: Extract model-native <think> or <thought> blocks ---
                    let has_think = plain_text.to_lowercase().contains("<think>");
                    let has_thought = plain_text.to_lowercase().contains("<thought>");

                    if has_think || has_thought {
                        let mut extracted_reasoning = String::new();
                        let mut cleaned_content = String::new();
                        let mut current_pos = 0;
                        let text_lower = plain_text.to_lowercase();

                        let tag_pairs = [("<think>", "</think>"), ("<thought>", "</thought>")];

                        // Find the first occurrence of any start tag
                        while let Some((pos, s, e)) = tag_pairs.iter().find_map(|(s, e)| {
                            text_lower[current_pos..]
                                .find(s)
                                .map(|idx| (current_pos + idx, *s, *e))
                        }) {
                            cleaned_content.push_str(&plain_text[current_pos..pos]);

                            let tag_len = s.len();
                            let remainder_lower = &text_lower[pos + tag_len..];
                            if let Some(end_idx) = remainder_lower.find(e) {
                                let absolute_end = pos + tag_len + end_idx;
                                let reasoning = &plain_text[pos + tag_len..absolute_end];
                                if !extracted_reasoning.is_empty() {
                                    extracted_reasoning.push_str("\n\n");
                                }
                                extracted_reasoning.push_str(reasoning.trim());
                                current_pos = absolute_end + e.len();
                            } else {
                                // Unclosed tag: take everything till the end
                                let reasoning = &plain_text[pos + tag_len..];
                                if !extracted_reasoning.is_empty() {
                                    extracted_reasoning.push_str("\n\n");
                                }
                                extracted_reasoning.push_str(reasoning.trim());
                                current_pos = plain_text.len();
                                break;
                            }
                        }

                        if current_pos < plain_text.len() {
                            cleaned_content.push_str(&plain_text[current_pos..]);
                        }

                        if !extracted_reasoning.is_empty() {
                            if !full_reasoning.is_empty() {
                                full_reasoning.push_str("\n\n");
                            }
                            full_reasoning.push_str(&extracted_reasoning);
                        }

                        // Final cleanup: remove all variants of thinking tags from plain_text
                        plain_text = cleaned_content
                            .replace("<think>", "")
                            .replace("</think>", "")
                            .replace("<THINK>", "")
                            .replace("</THINK>", "")
                            .replace("<thought>", "")
                            .replace("</thought>", "")
                            .replace("<THOUGHT>", "")
                            .replace("</THOUGHT>", "")
                            .trim()
                            .to_string();
                    }

                    return Ok((plain_text, tool_calls_json, full_reasoning, final_metadata));
                }
                Err(e) => {
                    let should_retry = match &e {
                        AiError::ApiRequestFailed { status_code, .. } => {
                            // Do NOT retry on auth errors or permanent client errors
                            !matches!(*status_code, 400 | 401 | 403 | 404)
                        }
                        // Retry on stream errors, network timeouts, etc.
                        _ => true,
                    };

                    if should_retry && retry_count < max_retries {
                        retry_count += 1;
                        let wait_secs = 2u32.pow(retry_count - 1);

                        log::warn!("WorkflowExecutor {}: LLM error encountered. Retrying in {}s (attempt {}/{}) - Error: {}",
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
                                    let sig_json: serde_json::Value = serde_json::from_str(&sig_str).unwrap_or_default();
                                    if sig_json["type"] == "stop" || sig_str.to_lowercase().contains("stop") {
                                        log::info!(
                                            "WorkflowExecutor {}: Stop signal received during retry backoff",
                                            self.session_id
                                        );
                                        return Err(WorkflowEngineError::General("Stopped during retry backoff".into()));
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

    async fn check_signal(
        &self,
        signal_rx: &mut tokio::sync::mpsc::Receiver<String>,
    ) -> Result<(), WorkflowEngineError> {
        while let Ok(sig_str) = signal_rx.try_recv() {
            let sig_json: serde_json::Value = serde_json::from_str(&sig_str).unwrap_or_default();
            if sig_json["type"] == "stop" || sig_str.to_lowercase().contains("stop") {
                log::info!(
                    "WorkflowExecutor {}: Stop signal received in LLM processor",
                    self.session_id
                );
                return Err(WorkflowEngineError::General("STOP_SIGNAL".into()));
            }
        }
        Ok(())
    }

    fn normalize_history(&self, raw_history: Vec<WorkflowMessage>) -> Vec<serde_json::Value> {
        let mut history: Vec<serde_json::Value> = Vec::new();

        for m in raw_history {
            // As part of defensive programming, filter out system messages
            if m.role == "system" {
                continue;
            }

            let role = m.role.clone();
            let mut content = m.message.clone();
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

            let mut msg =
                if role == "assistant" && tool_calls.is_some() && content.trim().is_empty() {
                    serde_json::json!({ "role": role, "content": serde_json::Value::Null })
                } else {
                    serde_json::json!({ "role": role, "content": content })
                };
            if let Some(tc) = tool_calls {
                msg["tool_calls"] = tc;
            }
            if let Some(tid) = tool_call_id {
                msg["tool_call_id"] = serde_json::json!(tid);
            }
            history.push(msg);
        }
        history
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
        system_parts.push(CORE_SYSTEM_PROMPT.to_string());

        // 2. Agent Specific Instructions
        if !self.agent_config.system_prompt.is_empty() {
            system_parts.push(format!(
                "<AGENT_SPECIFIC_INSTRUCTIONS>\n{}\n</AGENT_SPECIFIC_INSTRUCTIONS>",
                self.agent_config.system_prompt
            ));
        }

        // 3. Drafting for non-reasoning models
        if !self.reasoning {
            system_parts.push(DRAFTING_PROMPT.to_string());
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

        // === END NEW ===

        let combined_system_prompt = system_parts.join("\n\n---\n\n");

        // 1. Remove all existing system messages
        history.retain(|m| m["role"] != "system");

        // 2. Insert the single consolidated system message at the top
        history.insert(
            0,
            serde_json::json!({ "role": "system", "content": combined_system_prompt }),
        );

        // 10. Phase Instructions (injected into user's first message)
        let phase_instruction = match policy.phase {
            ExecutionPhase::Planning => self
                .agent_config
                .planning_prompt
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(PLANNING_MODE_PROMPT),
            ExecutionPhase::Implementation => EXECUTION_MODE_PROMPT,
            ExecutionPhase::Standard => "",
        };

        if !phase_instruction.is_empty() {
            if let Some(first_user_msg) = history.iter_mut().find(|m| m["role"] == "user") {
                let original_content = first_user_msg["content"].as_str().unwrap_or("");
                first_user_msg["content"] = serde_json::json!(format!(
                    "<PHASE_INSTRUCTIONS>\n{}\n</PHASE_INSTRUCTIONS>\n\n{}",
                    phase_instruction, original_content
                ));
            }
        }

        history
    }

    fn build_extend_tools_prompt(&self) -> String {
        let mut reminders = String::new();

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
        if !self.available_skills.is_empty() {
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

    fn build_reminders(&self, current_step: usize, max_steps: usize) -> String {
        let allowed_roots = if let Ok(guard) = self.path_guard.read() {
            guard.allowed_roots()
        } else {
            vec![]
        };
        let cwd = allowed_roots.first().cloned().unwrap_or_default();

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

                let status = std::process::Command::new("git")
                    .args(["-C", &cwd.to_string_lossy(), "status", "--short"])
                    .output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_default();

                let recent_commits = std::process::Command::new("git")
                    .args(["-C", &cwd.to_string_lossy(), "log", "-n", "3", "--oneline"])
                    .output()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_default();

                env_info.push_str(&format!("  - Git Repository: [Branch: {}]\n", branch));
                if !status.is_empty() {
                    env_info.push_str(&format!("  - Pending Changes:\n{}\n", status));
                }
                env_info.push_str(&format!("  - Recent Commits:\n{}\n", recent_commits));
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

        let progress_info = if max_steps > 0 {
            let remaining = max_steps.saturating_sub(current_step);
            format!("\n - Step: {current_step} / {max_steps} (remaining: {remaining})")
        } else {
            String::new()
        };

        format!(
            "{}{}\n\n<SYSTEM_REMINDER>\nIMPORTANT: this context may or may not be relevant to your tasks. You should not respond to this context unless it is highly relevant to your task.\n</SYSTEM_REMINDER>\n",
            env_info, progress_info
        )
    }
}

pub fn generate_error_reminder(error_type: &str, tool_name: &str, content: &str) -> String {
    match error_type {
        "Security" => {
            "<SYSTEM_REMINDER>The path is outside your authorized workspace. Please use 'list_dir' to verify valid paths or ask the user to add the directory to 'allowed_paths' in settings.</SYSTEM_REMINDER>".to_string()
        }
        "Io" => {
            let is_permission = content.contains("Permission denied");
            let is_not_found = content.contains("No such file")
                || content.contains("Not found")
                || content.contains("does not exist");
            match tool_name {
                "read_file" | "list_dir" | "edit_file" | "write_file" => {
                    if is_permission {
                        "<SYSTEM_REMINDER>Permission denied. Check file/directory permissions or ask the user to grant access.</SYSTEM_REMINDER>".to_string()
                    } else if is_not_found {
                        "<SYSTEM_REMINDER>Path does not exist. Use 'list_dir' to explore valid paths, or create parent directories first if writing.</SYSTEM_REMINDER>".to_string()
                    } else {
                        "<SYSTEM_REMINDER>I/O error. Verify the path exists and is correct. Use 'list_dir' to explore if unsure.</SYSTEM_REMINDER>".to_string()
                    }
                }
                _ => "<SYSTEM_REMINDER>Analyze the error and try a different approach if necessary.</SYSTEM_REMINDER>".to_string()
            }
        }
        "InvalidParams" => {
            "<SYSTEM_REMINDER>Check the tool's input schema and ensure all required fields are provided with correct types.</SYSTEM_REMINDER>".to_string()
        }
        "NoToolCall" => {
            "<SYSTEM_REMINDER>CRITICAL: You MUST call a tool in every response. Review the available tools and select the appropriate one for your current step. If you need to communicate with the user, use 'answer_user'. If the task is complete, use 'finish_task'.</SYSTEM_REMINDER>".to_string()
        }
        "Timeout" => {
            "<SYSTEM_REMINDER>Operation timed out. Consider breaking the task into smaller steps or using a less resource-intensive approach.</SYSTEM_REMINDER>".to_string()
        }
        "NetworkError" => {
            match tool_name {
                "web_search" => {
                    "<SYSTEM_REMINDER>Search failed due to network error. Recovery steps:\n\
                    1. Retry ONCE if you suspect a transient issue.\n\
                    2. If it fails again, try rephrasing your query with different keywords. \
                    If the topic is China-related, try searching in Chinese.\n\
                    3. If no results found, mark as 'data_missing'.</SYSTEM_REMINDER>"
                        .to_string()
                }
                "web_fetch" => {
                    "<SYSTEM_REMINDER>Failed to fetch this URL. Do NOT retry the same URL. \
                    Instead: (1) try an alternative URL from your search results, or \
                    (2) if no alternatives exist, mark the data as unavailable and proceed.</SYSTEM_REMINDER>"
                        .to_string()
                }
                _ => {
                    "<SYSTEM_REMINDER>Network error occurred. Check connection and retry once. If the issue persists, the service may be unavailable.</SYSTEM_REMINDER>"
                        .to_string()
                }
            }
        }
        "AuthError" => {
            "<SYSTEM_REMINDER>Authentication failed. Check API keys or credentials configuration in settings.</SYSTEM_REMINDER>".to_string()
        }
        "McpServerNotFound" => {
            "<SYSTEM_REMINDER>The requested MCP server is not found. Verify the server name or use 'list_available_tools' to see available servers.</SYSTEM_REMINDER>".to_string()
        }
        "Fatal" => {
            "<SYSTEM_REMINDER>A fatal error occurred. Stop and ask the user for guidance before proceeding.</SYSTEM_REMINDER>".to_string()
        }
        _ => {
            match tool_name {
                "bash" => {
                    let is_cmd_not_found = content.contains("command not found");
                    let is_syntax_error = content.contains("syntax error");
                    if is_cmd_not_found {
                        "<SYSTEM_REMINDER>Command not found. Verify the tool is installed or use an alternative command.</SYSTEM_REMINDER>".to_string()
                    } else if is_syntax_error {
                        "<SYSTEM_REMINDER>Shell syntax error. Check quoting, parentheses, and special character escaping.</SYSTEM_REMINDER>".to_string()
                    } else {
                        "<SYSTEM_REMINDER>Command failed. Check syntax and ensure required dependencies are installed.</SYSTEM_REMINDER>".to_string()
                    }
                }
                _ => "<SYSTEM_REMINDER>Analyze this error to decide your next step.</SYSTEM_REMINDER>".to_string()
            }
        }
    }
}
