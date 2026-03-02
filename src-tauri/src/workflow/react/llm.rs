use crate::ai::interaction::chat_completion::AiChatEnum;
use crate::ai::traits::chat::{MCPToolDeclaration, MessageType};
use crate::db::{Agent, WorkflowMessage};
use crate::workflow::react::context::ContextManager;
use crate::workflow::react::error::WorkflowEngineError;
use crate::workflow::react::gateway::Gateway;
use crate::workflow::react::security::PathGuard;
use crate::workflow::react::skills::SkillManifest;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;

pub const CORE_SYSTEM_PROMPT: &str = r#"You are an autonomous AI Agent operating in a tool-use environment.
Your core philosophy is: **Everything is a tool call.**

## OPERATIONAL GUIDELINES:
1. **Tool-First Thinking**: For every response, you MUST call at least one tool. If you are thinking, use 'think' internally but ensure the final output of the turn includes a tool call.
2. **Persistence**: Do not stop until the task is fully complete. Use `todo_*` tools to track your own progress.
3. **Structured Snapshot**: You will receive a <state_snapshot> in the context. Always respect the decisions and facts recorded there.
4. **Communication**: To talk to the user, you MUST use `answer_user` or `ask_user`. To finish, use `finish_task`.
5. **No Conversational Filler**: Do not provide conversational responses without tools. If you have nothing to do, call `finish_task` with a summary."#;

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
    ) -> Result<(String, String, Option<serde_json::Value>), WorkflowEngineError> {
        let raw_history = context.get_messages_for_llm();

        // 1. Context Normalization
        let history = self.normalize_history(raw_history);

        // 2. Build System Prompt & Environment Reminders
        let final_history = self.inject_prompts(history, current_step);

        // 3. Perform the LLM Call
        let (tx, mut rx) = mpsc::channel(100);
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
                    let _ = tx.try_send(chunk);
                },
            )
            .await
            .map_err(WorkflowEngineError::Ai)?;

        let mut full_content = String::new();
        let mut full_reasoning = String::new();

        while let Some(chunk) = rx.recv().await {
            match chunk.r#type {
                MessageType::Text => {
                    gateway
                        .send(
                            &self.session_id,
                            crate::workflow::react::types::GatewayPayload::Chunk {
                                content: chunk.chunk.clone(),
                            },
                        )
                        .await?;
                    full_content.push_str(&chunk.chunk);
                }
                MessageType::Reasoning => {
                    gateway
                        .send(
                            &self.session_id,
                            crate::workflow::react::types::GatewayPayload::Chunk {
                                content: chunk.chunk.clone(),
                            },
                        )
                        .await?;
                    full_reasoning.push_str(&chunk.chunk);
                }
                MessageType::ToolCalls => {
                    full_content.push_str(&chunk.chunk);
                }
                MessageType::Finished => break,
                MessageType::Error => {
                    return Err(WorkflowEngineError::General(chunk.chunk.clone()))
                }
                _ => {}
            }
        }

        Ok((full_content, full_reasoning, None))
    }

    /// Normalizes history to handle irregular message sequences.
    fn normalize_history(&self, raw_history: Vec<WorkflowMessage>) -> Vec<serde_json::Value> {
        let mut history: Vec<serde_json::Value> = Vec::new();

        for m in raw_history {
            let role = m.role.clone();
            let mut content = m.message.clone();
            let mut tool_calls = m
                .metadata
                .as_ref()
                .and_then(|meta| meta.get("tool_calls").cloned());

            // Normalize Assistant JSON
            if role == "assistant" && content.trim().starts_with('{') {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(c) = parsed.get("content").and_then(|v| v.as_str()) {
                        content = c.to_string();
                    }
                    if let Some(tc) = parsed.get("tool_calls").cloned() {
                        tool_calls = Some(tc);
                    }
                }
            }

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

            // Logic: Merge consecutive roles
            if let Some(last) = history.last_mut() {
                if last["role"] == role && role != "tool" {
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

        // --- Post-normalization: Fix dangling assistant tool calls ---
        let mut fixed_history = Vec::new();
        for i in 0..history.len() {
            let msg = &history[i];
            if msg["role"] == "assistant" && msg.get("tool_calls").is_some() {
                let has_results = history
                    .get(i + 1)
                    .map_or(false, |next| next["role"] == "tool");
                if !has_results {
                    let mut cleaned_msg = msg.clone();
                    if let Some(obj) = cleaned_msg.as_object_mut() {
                        obj.remove("tool_calls");
                    }
                    fixed_history.push(cleaned_msg);
                    continue;
                }
            }
            fixed_history.push(msg.clone());
        }

        fixed_history
    }

    /// Injects system prompt, environmental info, and reminders into history.
    fn inject_prompts(
        &self,
        mut history: Vec<serde_json::Value>,
        _current_step: usize,
    ) -> Vec<serde_json::Value> {
        let mut full_system_prompt = String::from(CORE_SYSTEM_PROMPT);
        full_system_prompt.push_str("\n\n## AGENT SPECIFIC INSTRUCTIONS:\n");
        full_system_prompt.push_str(&self.agent_config.system_prompt);

        if let Some(sys_msg) = history.iter_mut().find(|m| m["role"] == "system") {
            let original = sys_msg["content"].as_str().unwrap_or("");
            sys_msg["content"] =
                serde_json::json!(format!("{}\n\n{}", full_system_prompt, original));
        } else {
            history.insert(
                0,
                serde_json::json!({ "role": "system", "content": full_system_prompt }),
            );
        }

        let reminders = self.build_reminders();

        if let Some(first_user_msg) = history.iter_mut().find(|m| m["role"] == "user") {
            if let Some(content) = first_user_msg["content"].as_str() {
                first_user_msg["content"] = serde_json::json!(format!("{}{}", reminders, content));
            }
        } else {
            history.insert(
                0,
                serde_json::json!({ "role": "system", "content": reminders }),
            );
        }

        history
    }

    fn build_reminders(&self) -> String {
        let mut reminders = String::new();
        if !self.available_skills.is_empty() {
            reminders.push_str("<system-reminder>\nThe following skills are available for use with the Skill tool:\n\n");
            for skill in self.available_skills.values() {
                reminders.push_str(&format!("- {}: {}\n", skill.name, skill.description));
            }
            reminders.push_str("</system-reminder>\n");
        }

        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let cwd = std::env::current_dir().unwrap_or_default();
        let is_git = std::process::Command::new("git")
            .args(["rev-parse", "--is-inside-work-tree"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "true")
            .unwrap_or(false);
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

        let mut env_info = format!("Environment\nYou have been invoked in the following environment:\n - Primary working directory: {:?}\n  - Is a git repository: {}\n", cwd, is_git);
        let allowed_roots = if let Ok(guard) = self.path_guard.read() {
            guard.allowed_roots()
        } else {
            vec![]
        };
        if allowed_roots.len() > 1 || (allowed_roots.len() == 1 && allowed_roots[0] != cwd) {
            env_info.push_str(" - Additional working directories:\n");
            for root in allowed_roots {
                if root != cwd {
                    env_info.push_str(&format!("  - {:?}\n", root));
                }
            }
        }
        env_info.push_str(&format!(
            " - Platform: {}\n - Shell: {}\n - OS Version: {}\n",
            platform, shell, os_version
        ));

        format!("<system-reminder>\n{}\n\nAs you answer the user's questions, you can use the following context:\n# currentDate\nToday's date is {}.\n\nIMPORTANT: this context may or may not be relevant to your tasks. You should not respond to this context unless it is highly relevant to your task.\n</system-reminder>\n", env_info, today)
    }
}

/// Generates context-aware system reminders for tool errors.
/// This helps the AI agent recover from failures with actionable guidance.
pub fn generate_error_reminder(error_type: &str, tool_name: &str, content: &str) -> String {
    match error_type {
        "Security" => {
            "<system-reminder>The path is outside your authorized workspace. Please use 'list_dir' to verify valid paths or ask the user to add the directory to 'allowed_paths' in settings.</system-reminder>".to_string()
        }
        "Io" => {
            let is_permission = content.contains("Permission denied");
            let is_not_found = content.contains("No such file")
                || content.contains("Not found")
                || content.contains("does not exist");
            match tool_name {
                "read_file" | "list_dir" | "edit_file" | "write_file" => {
                    if is_permission {
                        "<system-reminder>Permission denied. Check file/directory permissions or ask the user to grant access.</system-reminder>".to_string()
                    } else if is_not_found {
                        "<system-reminder>Path does not exist. Use 'list_dir' to explore valid paths, or create parent directories first if writing.</system-reminder>".to_string()
                    } else {
                        "<system-reminder>I/O error. Verify the path exists and is correct. Use 'list_dir' to explore if unsure.</system-reminder>".to_string()
                    }
                }
                _ => "<system-reminder>Analyze the error and try a different approach if necessary.</system-reminder>".to_string()
            }
        }
        "InvalidParams" => {
            "<system-reminder>Check the tool's input schema and ensure all required fields are provided with correct types.</system-reminder>".to_string()
        }
        "NoToolCall" => {
            "<system-reminder>CRITICAL: You MUST call a tool in every response. Review the available tools and select the appropriate one for your current step. If you need to communicate with the user, use 'answer_user'. If the task is complete, use 'finish_task'.</system-reminder>".to_string()
        }
        "Timeout" => {
            "<system-reminder>Operation timed out. Consider breaking the task into smaller steps or using a less resource-intensive approach.</system-reminder>".to_string()
        }
        "NetworkError" => {
            "<system-reminder>Network error occurred. Check your connection and retry. If the issue persists, the remote service may be unavailable.</system-reminder>".to_string()
        }
        "AuthError" => {
            "<system-reminder>Authentication failed. Check API keys or credentials configuration in settings.</system-reminder>".to_string()
        }
        "McpServerNotFound" => {
            "<system-reminder>The requested MCP server is not found. Verify the server name or use 'list_available_tools' to see available servers.</system-reminder>".to_string()
        }
        "Fatal" => {
            "<system-reminder>A fatal error occurred. Stop and ask the user for guidance before proceeding.</system-reminder>".to_string()
        }
        _ => {
            match tool_name {
                "bash" => {
                    let is_cmd_not_found = content.contains("command not found");
                    let is_syntax_error = content.contains("syntax error");
                    if is_cmd_not_found {
                        "<system-reminder>Command not found. Verify the tool is installed or use an alternative command.</system-reminder>".to_string()
                    } else if is_syntax_error {
                        "<system-reminder>Shell syntax error. Check quoting, parentheses, and special character escaping.</system-reminder>".to_string()
                    } else {
                        "<system-reminder>Command failed. Check syntax and ensure required dependencies are installed.</system-reminder>".to_string()
                    }
                }
                _ => "<system-reminder>Analyze this error to decide your next step.</system-reminder>".to_string()
            }
        }
    }
}
