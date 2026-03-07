use crate::ai::interaction::chat_completion::{AiChatEnum, ChatState};
use crate::ai::traits::chat::{MCPToolDeclaration, MessageType};
use crate::db::{Agent, WorkflowMessage};
use crate::workflow::react::context::ContextManager;
use crate::workflow::react::error::WorkflowEngineError;
use crate::workflow::react::gateway::Gateway;
use crate::workflow::react::policy::{ExecutionPhase, ExecutionPolicy};
use crate::workflow::react::security::PathGuard;
use crate::workflow::react::skills::SkillManifest;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;

use crate::workflow::react::prompts::{
    CORE_SYSTEM_PROMPT, EXECUTION_MODE_PROMPT, PLANNING_MODE_PROMPT,
};

pub struct LlmProcessor {
    pub session_id: String,
    pub agent_config: Agent,
    pub available_skills: HashMap<String, SkillManifest>,
    pub path_guard: Arc<RwLock<PathGuard>>,
    pub active_provider_id: i64,
    pub active_model_name: String,
    pub reasoning: bool,
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
    ) -> Self {
        Self {
            session_id,
            agent_config,
            available_skills,
            path_guard,
            active_provider_id,
            active_model_name,
            reasoning,
        }
    }

    /// Prepares and calls the LLM with the current context.
    pub async fn call(
        &self,
        context: &mut ContextManager,
        current_step: usize,
        chat_interface: AiChatEnum,
        gateway: Arc<dyn Gateway>,
        tools: Vec<MCPToolDeclaration>,
        max_steps: usize,
        policy: &ExecutionPolicy,
    ) -> Result<(String, String, String, Option<serde_json::Value>), WorkflowEngineError> {
        let raw_history = context.get_messages_for_llm();

        // 1. Extract the latest state_snapshot from compression history (if any).
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

        // 2. Extract the next pending todo item for progress display.
        let next_pending_task = {
            // Look for the most recent todo_list / todo_update result content in history
            // We parse the session todo from stored messages via the reinforced snapshots.
            // Fallback: none.
            None::<String>
        };

        // 3. Context Normalization
        let history = self.normalize_history(raw_history);

        // 4. Build System Prompt & Environment Reminders
        let final_history = self.inject_prompts(
            history,
            current_step,
            max_steps,
            state_snapshot,
            next_pending_task,
            policy,
        );

        // 5. Perform the LLM Call
        let (tx, mut rx) = mpsc::channel::<Arc<crate::ai::traits::chat::ChatResponse>>(100);

        let session_id_for_rx = self.session_id.clone();
        let gateway_for_rx = gateway.clone();

        // Drain the channel in a separate task to avoid blocking the chat loop.
        // This ensures chunks are forwarded to the gateway in real-time.
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
                                crate::workflow::react::types::GatewayPayload::Chunk {
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
                                crate::workflow::react::types::GatewayPayload::ReasoningChunk {
                                    content: chunk.chunk.clone(),
                                },
                            )
                            .await?;
                        full_reasoning.push_str(&chunk.chunk);
                    }
                    MessageType::ToolCalls => {
                        // Standardize tool call IDs using our own unique generator
                        let mut tool_calls_val: serde_json::Value =
                            serde_json::from_str(&chunk.chunk).unwrap_or(serde_json::json!([]));

                        // Case 1: OpenAI style array [ {id, function: {name, arguments}}, ... ]
                        if let Some(tool_calls_array) = tool_calls_val.as_array_mut() {
                            for tool_call in tool_calls_array {
                                if let Some(tool_call_obj) = tool_call.as_object_mut() {
                                    tool_call_obj.insert(
                                        "id".to_string(),
                                        serde_json::json!(crate::ccproxy::get_tool_id()),
                                    );
                                }
                            }
                        }
                        // Case 2: ReAct style object { "tool": { "name", "arguments", ... } }
                        else if let Some(tool_wrapper) = tool_calls_val.get_mut("tool") {
                            if let Some(tool_obj) = tool_wrapper.as_object_mut() {
                                tool_obj.insert(
                                    "id".to_string(),
                                    serde_json::json!(crate::ccproxy::get_tool_id()),
                                );
                            }
                        }
                        // Case 3: Single tool object { "name", "arguments", ... }
                        else if tool_calls_val.is_object() && tool_calls_val.get("name").is_some()
                        {
                            if let Some(tool_obj) = tool_calls_val.as_object_mut() {
                                tool_obj.insert(
                                    "id".to_string(),
                                    serde_json::json!(crate::ccproxy::get_tool_id()),
                                );
                            }
                        }

                        tool_calls_json =
                            serde_json::to_string(&tool_calls_val).unwrap_or(chunk.chunk.clone());
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
        chat_interface
            .chat(
                self.active_provider_id,
                &self.active_model_name,
                self.session_id.clone(),
                final_history,
                Some(tools),
                Some(crate::ai::traits::chat::ChatMetadata {
                    reasoning: Some(self.reasoning),
                    ..Default::default()
                }),
                move |chunk| {
                    let _ = tx_for_chat.try_send(chunk);
                },
            )
            .await
            .map_err(WorkflowEngineError::Ai)?;

        // Close the local channel sender to ensure the receiver task finishes
        drop(tx);
        let (mut plain_text, tool_calls_json, mut full_reasoning, final_metadata) = rx_processor
            .await
            .map_err(|e| WorkflowEngineError::General(format!("RX task failed: {}", e)))??;

        // --- Post-processing: Extract model-native <think> blocks if present ---
        // We keep <thought> in the plain_text so it stays in the conversation history
        // and can be styled by the frontend without triggering API field conflicts.
        if plain_text.contains("<think>") || plain_text.contains("</think>") {
            let mut extracted_reasoning = String::new();
            let mut cleaned_content = String::new();
            let mut current_pos = 0;

            while let Some(start_idx) = plain_text[current_pos..].find("<think>") {
                let absolute_start = current_pos + start_idx;
                cleaned_content.push_str(&plain_text[current_pos..absolute_start]);

                let remainder = &plain_text[absolute_start + 7..];
                if let Some(end_idx) = remainder.find("</think>") {
                    let absolute_end = absolute_start + 7 + end_idx;
                    let reasoning = &plain_text[absolute_start + 7..absolute_end];
                    if !extracted_reasoning.is_empty() {
                        extracted_reasoning.push_str("\n\n");
                    }
                    extracted_reasoning.push_str(reasoning.trim());
                    current_pos = absolute_end + 8;
                } else {
                    if !extracted_reasoning.is_empty() {
                        extracted_reasoning.push_str("\n\n");
                    }
                    extracted_reasoning.push_str(remainder.trim());
                    current_pos = plain_text.len();
                    break;
                }
            }

            if current_pos < plain_text.len() {
                cleaned_content.push_str(&plain_text[current_pos..]);
            } else if current_pos == 0 {
                cleaned_content.push_str(&plain_text);
            }

            if !extracted_reasoning.is_empty() {
                if !full_reasoning.is_empty() {
                    full_reasoning.push_str("\n\n");
                }
                full_reasoning.push_str(&extracted_reasoning);
            }

            plain_text = cleaned_content.replace("</think>", "").trim().to_string();
        }

        Ok((plain_text, tool_calls_json, full_reasoning, final_metadata))
    }

    /// Normalizes history to handle irregular message sequences.
    fn normalize_history(&self, raw_history: Vec<WorkflowMessage>) -> Vec<serde_json::Value> {
        let mut history: Vec<serde_json::Value> = Vec::new();

        for m in raw_history {
            let role = m.role.clone();
            let mut content = m.message.clone();
            let tool_calls = m
                .metadata
                .as_ref()
                .and_then(|meta| meta.get("tool_calls").cloned());

            // Standardize Tool Result messages
            let tool_call_id = if role == "tool" {
                m.metadata
                    .as_ref()
                    .and_then(|meta| meta.get("tool_call_id").and_then(|v| v.as_str()))
                    .map(|s| s.to_string())
            } else {
                None
            };

            // Dynamic System Reminder for Errors
            if role == "tool" && m.is_error {
                let meta = m.metadata.as_ref();
                let tool_name = meta
                    .and_then(|meta| {
                        meta.get("tool_call").and_then(|tc| {
                            tc.get("function")
                                .and_then(|f| f.get("name").and_then(|n| n.as_str()))
                        })
                    })
                    .or_else(|| {
                        meta.and_then(|meta| {
                            meta.get("tool_call")
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

            // Logic: Merge consecutive roles (except tool results and system prompts)
            if let Some(last) = history.last_mut() {
                if last["role"] == role && role != "tool" && role != "system" {
                    let last_content = last["content"].as_str().unwrap_or("");
                    if !content.is_empty() {
                        last["content"] =
                            serde_json::json!(format!("{}\n\n{}", last_content, content));
                    }
                    if let Some(tc) = tool_calls {
                        last["tool_calls"] = tc;
                    }
                    continue;
                }
            }

            // Skip empty user messages unless they have tool_calls (though user doesn't usually)
            if role == "user" && content.trim().is_empty() {
                continue;
            }

            let mut msg = serde_json::json!({
                "role": role,
                "content": content,
            });

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

    /// Injects system prompt, environmental info, and reminders into history.
    fn inject_prompts(
        &self,
        mut history: Vec<serde_json::Value>,
        current_step: usize,
        max_steps: usize,
        state_snapshot: Option<String>,
        _next_pending_task: Option<String>,
        policy: &ExecutionPolicy,
    ) -> Vec<serde_json::Value> {
        // --- 1. Build System Prompt (Identity & Environment) ---
        let mut system_parts = Vec::new();

        // 1.1 Core Rulebook
        system_parts.push(CORE_SYSTEM_PROMPT.to_string());

        // 1.2 Agent Specific Instructions (Identity)
        if !self.agent_config.system_prompt.is_empty() {
            system_parts.push(format!(
                "<AGENT_SPECIFIC_INSTRUCTIONS>\n{}\n</AGENT_SPECIFIC_INSTRUCTIONS>",
                self.agent_config.system_prompt
            ));
        }

        // 1.3 Environmental Context (Date, OS, Skills)
        let reminders = self.build_reminders(current_step, max_steps);
        system_parts.push(format!(
            "<ENVIRONMENT_CONTEXT>\n{}\n</ENVIRONMENT_CONTEXT>",
            reminders
        ));

        // 1.4 Context Continuity (Last Snapshot)
        if let Some(snapshot) = state_snapshot {
            system_parts.push(format!(
                "<PREVIOUS_CONTEXT_SNAPSHOT>\n{}\n</PREVIOUS_CONTEXT_SNAPSHOT>",
                snapshot
            ));
        }

        let combined_system_prompt = system_parts.join("\n\n---\n\n");

        // Apply System Prompt
        if let Some(sys_msg) = history.iter_mut().find(|m| m["role"] == "system") {
            sys_msg["content"] = serde_json::json!(combined_system_prompt);
        } else {
            history.insert(
                0,
                serde_json::json!({ "role": "system", "content": combined_system_prompt }),
            );
        }

        // --- 2. Inject Phase Instructions into the FIRST User Message ---
        let phase_instruction = match policy.phase {
            ExecutionPhase::Planning => {
                // Priority: 1. Agent custom planning prompt, 2. Global default
                self.agent_config.planning_prompt.as_deref().filter(|s| !s.trim().is_empty())
                    .unwrap_or(PLANNING_MODE_PROMPT)
            }
            ExecutionPhase::Implementation => EXECUTION_MODE_PROMPT,
            ExecutionPhase::Standard => "",
        };

        if !phase_instruction.is_empty() {
            if let Some(first_user_msg) = history.iter_mut().find(|m| m["role"] == "user") {
                let original_content = first_user_msg["content"].as_str().unwrap_or("");
                
                // Wrap with clear boundaries
                first_user_msg["content"] = serde_json::json!(format!(
                    "<PHASE_INSTRUCTIONS>\n{}\n</PHASE_INSTRUCTIONS>\n\n{}",
                    phase_instruction,
                    original_content
                ));
            }
        }

        history
    }

    fn build_reminders(&self, current_step: usize, max_steps: usize) -> String {
        let mut reminders = String::new();

        // 1. Available Skills
        if !self.available_skills.is_empty() {
            reminders.push_str("## AVAILABLE SKILLS:\n");
            for skill in self.available_skills.values() {
                reminders.push_str(&format!("- {}: {}\n", skill.name, skill.description));
            }
            reminders.push_str("\n<SYSTEM_REMINDER>You can use the above specialized skills via the `skill` tool.</SYSTEM_REMINDER>\n");
        }

        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let allowed_roots = if let Ok(guard) = self.path_guard.read() {
            guard.allowed_roots()
        } else {
            vec![]
        };

        let cwd = allowed_roots.first().cloned().unwrap_or_default();
        let is_git = if !cwd.as_os_str().is_empty() {
            std::process::Command::new("git")
                .args(["-C", &cwd.to_string_lossy(), "rev-parse", "--is-inside-work-tree"])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "true")
                .unwrap_or(false)
        } else {
            false
        };

        let mut env_info = String::from("Environment\nYou have been invoked in the following environment:\n");
        if !cwd.as_os_str().is_empty() {
            env_info.push_str(&format!(" - Primary working directory: {:?}\n", cwd));
            
            if is_git {
                let branch = std::process::Command::new("git")
                    .args(["-C", &cwd.to_string_lossy(), "rev-parse", "--abbrev-ref", "HEAD"])
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

        env_info.push_str(&format!(
            " - Platform: {}\n - Shell: {}\n - OS Version: {}\n",
            platform, shell, os_version
        ));

        // Step progress block
        let progress_info = if max_steps > 0 {
            let remaining = max_steps.saturating_sub(current_step);
            format!("\n - Step: {current_step} / {max_steps} (remaining: {remaining})")
        } else {
            String::new()
        };

        format!(
            "<system-reminder>\n{}{}\n\nAs you answer the user's questions, you can use the following context:\n# currentDate\nToday's date is {}.\n\nIMPORTANT: this context may or may not be relevant to your tasks. You should not respond to this context unless it is highly relevant to your task.\n</system-reminder>\n",
            env_info, progress_info, today
        )
    }
}

/// Generates context-aware system reminders for tool errors.
/// This helps the AI agent recover from failures with actionable guidance.
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
                "web_search" | "web_fetch" => {
                    "<SYSTEM_REMINDER>Network error on web operation. Recovery steps:\n\
                    1. Retry ONCE with the same URL/query. \n\
                    2. If retry fails: try an alternative search query or a different data source URL.\n\
                    3. If no alternatives exist: mark this task as 'data_missing' and proceed to the next task.\n\
                    Do NOT retry more than once with identical parameters.</SYSTEM_REMINDER>"
                        .to_string()
                }
                _ => {
                    "<SYSTEM_REMINDER>Network error occurred. Check your connection and retry. If the issue persists, the remote service may be unavailable.</SYSTEM_REMINDER>"
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
