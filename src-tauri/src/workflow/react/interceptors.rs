use serde_json::json;

use crate::tools::{
    READ_ONLY_BASH_CMDS_EXACT, READ_ONLY_BASH_PREFIXES, TOOL_BASH, TOOL_EDIT_FILE, TOOL_GLOB,
    TOOL_GREP, TOOL_LIST_DIR, TOOL_READ_FILE, TOOL_WEB_FETCH, TOOL_WEB_SEARCH, TOOL_WRITE_FILE,
};
use crate::workflow::react::engine::WorkflowExecutor;
use crate::workflow::react::error::WorkflowEngineError;
use crate::workflow::react::observation::{ObservationReinforcer, ReinforcedResult};
use crate::workflow::react::policy::{ApprovalLevel, ExecutionPhase};
use crate::workflow::react::types::{GatewayPayload, WorkflowState};

impl WorkflowExecutor {
    /// Determines if a tool call should be intercepted for user approval based on the current ApprovalLevel.
    pub(crate) fn should_intercept_for_approval(
        &self,
        name: &str,
        args: &serde_json::Value,
    ) -> bool {
        // Core workflow tools are internal and safe, never intercept them
        if name.starts_with("todo_")
            || name.starts_with("task_")
            || name.starts_with("skill_")
            || name == crate::tools::TOOL_ASK_USER
            || name == crate::tools::TOOL_TASK
            || name == crate::tools::TOOL_SKILL
            || name == crate::tools::TOOL_FINISH_TASK
        {
            return false;
        }

        // Full mode never intercepts
        if self.policy.approval_level == ApprovalLevel::Full {
            return false;
        }

        // If already in auto_approve list, don't intercept
        if self.auto_approve.contains(name) {
            return false;
        }

        // Default mode: intercept everything not in auto_approve
        if self.policy.approval_level == ApprovalLevel::Default {
            return true;
        }

        // Smart mode: allow read/write tools, intercept risky bash or unknown mutations
        if self.policy.approval_level == ApprovalLevel::Smart {
            let is_safe_tool = name.starts_with(TOOL_READ_FILE)
                || name == TOOL_EDIT_FILE
                || name == TOOL_WRITE_FILE
                || name == TOOL_LIST_DIR
                || name == TOOL_GLOB
                || name == TOOL_GREP
                || name == TOOL_WEB_SEARCH
                || name == TOOL_WEB_FETCH;

            if is_safe_tool {
                return false;
            }

            // Special handling for bash in Smart mode:
            // Auto-approve common read-only commands ONLY IF they don't contain operators
            if name == TOOL_BASH {
                let command_str = args["command"].as_str().unwrap_or("").trim().to_lowercase();

                // Security Guard: Any redirection, piping, or background execution MUST be reviewed
                // to prevent attacks like 'cat secret.txt > malicious.sh'
                let has_operators = command_str
                    .chars()
                    .any(|c| matches!(c, '>' | '<' | '|' | '&' | ';'));
                if has_operators {
                    log::info!(
                        "WorkflowExecutor {}: Intercepting bash due to shell operators: {}",
                        self.session_id,
                        command_str
                    );
                    return true;
                }

                // Check exact match first (O(1) with perfect hash)
                if READ_ONLY_BASH_CMDS_EXACT.contains(command_str.as_str()) {
                    return false; // Don't intercept - it's read-only
                }

                // Check prefix match for commands with arguments (O(n) but small n)
                let is_read_only_bash = READ_ONLY_BASH_PREFIXES
                    .iter()
                    .any(|&p| command_str.starts_with(p));
                return !is_read_only_bash; // Intercept if NOT explicitly in the read-only whitelist
            }

            // All other tools (write_file, edit_file, delete_*, etc.) should be intercepted
            return true;
        }

        true
    }

    pub(crate) async fn handle_submit_plan_intercept(
        &mut self,
        text_part: &str,
    ) -> Result<Option<ReinforcedResult>, WorkflowEngineError> {
        if self.policy.phase != ExecutionPhase::Planning {
            return Err(WorkflowEngineError::Security(
                "Tool 'submit_plan' is only allowed in Planning phase.".into(),
            ));
        }

        if text_part.trim().is_empty() {
            return Ok(Some(ReinforcedResult {
                content: "<SYSTEM_REMINDER>Error: You called 'submit_plan' but your plain text response was empty. You MUST provide a summary of your findings and why this plan is recommended in plain text BEFORE the tool call block.</SYSTEM_REMINDER>".into(),
                title: "SubmitPlan Error".to_string(),
                summary: "Missing summary".to_string(),
                is_error: true,
                error_type: Some("NoSummary".into()),
                display_type: "text".to_string(),
            }));
        }

        // In Full approval mode, set AwaitingAutoApproval state.
        // This acts as a signal for the main loop to perform auto-transition.
        if self.policy.approval_level == ApprovalLevel::Full {
            log::info!(
                "WorkflowExecutor {}: Setting AwaitingAutoApproval for auto-transition in Full mode",
                self.session_id
            );
            self.update_state(WorkflowState::AwaitingAutoApproval)
                .await?;
            return Ok(None);
        }

        self.update_state(WorkflowState::AwaitingApproval).await?;

        let plan_str = text_part.to_string();

        Ok(Some(ReinforcedResult {
            content: format!("Proposed Plan:\n\n{}", plan_str),
            title: "Submit Plan".to_string(),
            summary: "Awaiting approval".to_string(),
            is_error: false,
            error_type: None,
            display_type: "text".to_string(),
        }))
    }

    pub(crate) async fn handle_ask_user_intercept(
        &mut self,
        args: &serde_json::Value,
    ) -> Result<Option<ReinforcedResult>, WorkflowEngineError> {
        let question = args["question"]
            .as_str()
            .unwrap_or("Waiting for your response...");
        let options = args["options"].as_array();

        // We no longer send a Confirm signal here to avoid redundant popups.
        // The message is already displayed in the chat stream.
        // We just pause the engine and wait for user input in the main text area.
        self.update_state(WorkflowState::Paused).await?;

        // Format content as JSON if we have options, otherwise just the question
        let content = if let Some(opts) = options {
            json!({
                "question": question,
                "options": opts
            })
            .to_string()
        } else {
            question.to_string()
        };

        Ok(Some(ReinforcedResult {
            content,
            title: "Ask User".to_string(),
            summary: "Waiting for user".to_string(),
            is_error: false,
            error_type: None,
            display_type: if options.is_some() { "choice" } else { "text" }.to_string(),
        }))
    }

    pub(crate) async fn handle_finish_task_intercept(
        &mut self,
        text_part: &str,
    ) -> Result<Option<ReinforcedResult>, WorkflowEngineError> {
        if text_part.trim().is_empty() {
            return Ok(Some(ReinforcedResult {
                content: "<SYSTEM_REMINDER>Error: You called 'finish_task' but your plain text response was empty. You MUST provide a comprehensive summary or report in plain text BEFORE the tool call block to inform the user of your results.</SYSTEM_REMINDER>".into(),
                title: "FinishTask Error".to_string(),
                summary: "Missing summary".to_string(),
                is_error: true,
                error_type: Some("NoSummary".into()),
                display_type: "text".to_string(),
            }));
        }

        if let Ok(store) = self.context.main_store.read() {
            if let Ok(todos) = store.get_todo_list_for_workflow(&self.session_id) {
                let active_tasks: Vec<String> = todos
                    .iter()
                    .filter(|t| {
                        let s = t["status"].as_str().unwrap_or("");
                        s == "pending" || s == "in_progress"
                    })
                    .map(|t| t["subject"].as_str().unwrap_or("Untitled").to_string())
                    .collect();

                if !active_tasks.is_empty() {
                    return Ok(Some(ReinforcedResult {
                        content: format!("<SYSTEM_REMINDER>Block: You still have active tasks: {}. You MUST either complete them, or mark them as 'failed' or 'data_missing' if they cannot be fulfilled, before calling finish_task.</SYSTEM_REMINDER>", active_tasks.join(", ")),
                        title: "Tasks Pending".to_string(),
                        summary: "Incomplete todos".to_string(),
                        is_error: true,
                        error_type: Some("PendingTodos".into()),
                        display_type: "text".to_string(),
                    }));
                }
            }
        }

        // 3. Optional Hidden AI quality audit
        if self.agent_config.final_audit.unwrap_or(false) {
            log::info!(
                "WorkflowExecutor {}: Performing final quality audit...",
                self.session_id
            );
            self.update_state(WorkflowState::Auditing).await?;
            if let Some(audit_feedback) = self
                .intelligence_manager
                .run_final_audit(&self.context)
                .await?
            {
                return Ok(Some(ReinforcedResult {
                    content: format!("<SYSTEM_REMINDER>Audit Rejected: Your conclusion was deemed incomplete. Feedback: {}\n\nYou MUST address these points before you can call finish_task.</SYSTEM_REMINDER>", audit_feedback),
                    title: "Audit Rejected".to_string(),
                    summary: "Audit failed".to_string(),
                    is_error: true,
                    error_type: Some("AuditRejected".into()),
                    display_type: "text".to_string(),
                }));
            }
        }

        self.update_state(WorkflowState::Completed).await?;

        Ok(Some(ReinforcedResult {
            content: "Finished".into(),
            title: "Finish Task".to_string(),
            summary: rust_i18n::t!("workflow.task_finished").to_string(),
            is_error: false,
            error_type: None,
            display_type: "text".to_string(),
        }))
    }

    pub(crate) async fn handle_bash_security_intercept(
        &mut self,
        id: &str,
        args: &serde_json::Value,
    ) -> Result<Option<ReinforcedResult>, WorkflowEngineError> {
        let command_str = args["command"].as_str().unwrap_or("");
        if !self.auto_approve.contains(TOOL_BASH) {
            let custom_rules: Vec<crate::tools::ShellPolicyRule> = self
                .agent_config
                .shell_policy
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_default();
            let policy_engine =
                crate::tools::ShellPolicyEngine::new(self.path_guard.clone(), custom_rules);

            match policy_engine.check(command_str, self.policy.phase == ExecutionPhase::Planning) {
                crate::tools::ShellDecision::Allow => {}
                crate::tools::ShellDecision::Deny(reason) => {
                    return Ok(Some(ReinforcedResult {
                        content: format!(
                            "Command blocked by security policy: {}. You may try an alternative command, modify the approach, or ask the user to adjust the policy if this restriction is blocking a legitimate task.",
                            reason
                        ),
                        title: format!("Bash({})", command_str),
                        summary: "Blocked".to_string(),
                        is_error: true,
                        error_type: Some("Security".to_string()),
                        display_type: "text".to_string(),
                    }));
                }
                crate::tools::ShellDecision::Review(reason) => {
                    if self.policy.approval_level == ApprovalLevel::Full {
                        log::info!(
                            "WorkflowExecutor {}: Auto-approving risky bash command in Full mode (Policy: {})",
                            self.session_id, reason
                        );
                    } else {
                        log::info!(
                            "WorkflowExecutor {}: Intercepting bash command for review: {}",
                            self.session_id,
                            reason
                        );
                        // Delegate to unified approval handler with descriptive command preview
                        let display_content =
                            format!("Command: {}\nReason: {}", command_str, reason);
                        return self
                            .handle_approval_interception(
                                id,
                                TOOL_BASH,
                                args,
                                Some(display_content),
                            )
                            .await;
                    }
                }
            }
        }
        Ok(None)
    }

    pub(crate) async fn handle_approval_interception(
        &mut self,
        id: &str,
        name: &str,
        args: &serde_json::Value,
        display_content: Option<String>,
    ) -> Result<Option<ReinforcedResult>, WorkflowEngineError> {
        // 1. Stash the full tool name and arguments in the server-side map.
        // This ensures the frontend doesn't need to pass complex JSON back to us.
        let stash_obj = json!({
            "name": name,
            "arguments": args
        });
        self.pending_approvals.insert(id.to_string(), stash_obj);

        self.update_state(WorkflowState::AwaitingApproval).await?;

        // 2. Determine what to show the user in the UI (Generate Diffs for File Ops)
        let mut display_type = "text".to_string();
        let content = if let Some(custom) = display_content {
            custom
        } else {
            match name {
                TOOL_EDIT_FILE => {
                    display_type = "diff".to_string();
                    let path = args["file_path"].as_str().unwrap_or("unknown");
                    let old = args["old_string"].as_str().unwrap_or("");
                    let new = args["new_string"].as_str().unwrap_or("");
                    format!("--- {}\n+++ {}\n- {}\n+ {}", path, path, old, new)
                }
                TOOL_WRITE_FILE => {
                    display_type = "diff".to_string();
                    let path = args["file_path"].as_str().unwrap_or("unknown");
                    let new_content = args["content"].as_str().unwrap_or("");

                    // Cap preview size to ensure UI snappiness
                    let preview_limit = 2000;
                    let new_preview: String = new_content.chars().take(preview_limit).collect();
                    let suffix = if new_content.chars().count() > preview_limit {
                        "... (truncated)"
                    } else {
                        ""
                    };

                    match std::fs::metadata(path) {
                        Ok(meta) => {
                            let size_kb = meta.len() / 1024;
                            format!(
                                "--- {}\n+++ {}\n(Full overwrite, size: {} KB)\n- [Existing content...]\n+ {}{}",
                                path, path, size_kb, new_preview, suffix
                            )
                        }
                        Err(_) => {
                            // Represent new file as: --- path \n +++ path \n - \n + content
                            format!("--- {}\n+++ {}\n-\n+ {}{}", path, path, new_preview, suffix)
                        }
                    }
                }
                _ => format!(
                    "Tool: {}\nArguments: {}",
                    name,
                    serde_json::to_string_pretty(args).unwrap_or_default()
                ),
            }
        };

        // 3. Notify frontend to show the confirmation prompt
        self.gateway
            .send(
                &self.session_id,
                GatewayPayload::Confirm {
                    id: id.to_string(),
                    action: name.to_string(),
                    details: content.clone(),
                },
            )
            .await?;

        // 4. Return a 'waiting' result to the engine loop.
        // Use standard title generation to match the UI screenshot provided.
        let pretty_title = {
            let primary_root = self
                .path_guard
                .read()
                .unwrap()
                .get_primary_root()
                .map(|p| p.to_path_buf());
            ObservationReinforcer::generate_title(name, args, None, primary_root.as_deref())
        };

        Ok(Some(ReinforcedResult {
            content,
            title: pretty_title,
            summary: rust_i18n::t!("workflow.state.awaiting_approval").to_string(),
            is_error: false,
            error_type: None,
            display_type,
        }))
    }

    pub(crate) fn handle_fs_path_guard_intercept(
        &self,
        name: &str,
        args: &serde_json::Value,
    ) -> Result<Option<ReinforcedResult>, WorkflowEngineError> {
        if let Some(path_str) = args["file_path"].as_str().or_else(|| args["path"].as_str()) {
            let guard = self.path_guard.read().map_err(|e| {
                WorkflowEngineError::General(format!("PathGuard lock poisoned: {}", e))
            })?;

            let is_write = [TOOL_WRITE_FILE, TOOL_EDIT_FILE].contains(&name);
            let is_planning = self.policy.phase == ExecutionPhase::Planning;

            // 1. Validate security boundaries
            if let Err(e) =
                guard.validate(std::path::Path::new(path_str), is_planning, is_write, false)
            {
                return Ok(Some(ReinforcedResult {
                    content: format!("Security Error: {}\n<SYSTEM_REMINDER>Access to this path is denied. If this path is essential, please ask the user to add it to the 'Authorized Paths' in settings. Otherwise, try to use a different path or approach.</SYSTEM_REMINDER>", e),
                    title: format!("Security Check: {}", name),
                    summary: "Access Denied".to_string(),
                    is_error: true,
                    error_type: Some("Security".to_string()),
                    display_type: "text".to_string(),
                }));
            }
        }
        Ok(None)
    }
}
