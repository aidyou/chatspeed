use crate::tools::{TOOL_BASH, TOOL_EDIT_FILE, TOOL_WRITE_FILE};
use crate::workflow::react::engine::WorkflowExecutor;
use crate::workflow::react::error::WorkflowEngineError;
use crate::workflow::react::observation::ReinforcedResult;
use crate::workflow::react::policy::{ApprovalLevel, ExecutionPhase};
use crate::workflow::react::types::{GatewayPayload, WorkflowState};

impl WorkflowExecutor {
    /// Determines if a tool call should be intercepted for user approval based on the current ApprovalLevel.
    pub(crate) fn should_intercept_for_approval(
        &self,
        name: &str,
        args: &serde_json::Value,
    ) -> bool {
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

        // Smart mode: allow read-only tools, intercept mutations and risky bash
        if self.policy.approval_level == ApprovalLevel::Smart {
            let is_read_only_tool = name.starts_with("read_")
                || name.starts_with("list_")
                || name.starts_with("get_")
                || name.starts_with("todo_list")
                || name.starts_with("todo_get")
                || name.contains("search")
                || name.contains("fetch")
                || name == "glob"
                || name == "grep";

            if is_read_only_tool {
                return false;
            }

            // Special handling for bash in Smart mode:
            // Auto-approve common read-only commands
            if name == TOOL_BASH {
                let command_str = args["command"].as_str().unwrap_or("").trim().to_lowercase();
                let read_only_bash_cmds = [
                    "ls",
                    "pwd",
                    "date",
                    "git status",
                    "git log",
                    "git diff",
                    "cat ",
                    "grep ",
                    "find ",
                    "file ",
                    "stat ",
                ];
                let is_read_only_bash = read_only_bash_cmds
                    .iter()
                    .any(|&p| command_str.starts_with(p));
                return !is_read_only_bash; // Intercept if NOT read-only
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

        self.update_state(WorkflowState::AwaitingApproval).await?;

        Ok(None)
    }

    pub(crate) async fn handle_ask_user_intercept(
        &mut self,
    ) -> Result<Option<ReinforcedResult>, WorkflowEngineError> {
        self.update_state(WorkflowState::Paused).await?;
        Ok(Some(ReinforcedResult {
            content: "Waiting for user".into(),
            title: "AskUser".to_string(),
            summary: "Asked user".to_string(),
            is_error: false,
            error_type: None,
            display_type: "text".to_string(),
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
        Ok(None)
    }

    pub(crate) async fn handle_bash_security_intercept(
        &mut self,
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
                            "Error: Command blocked by security policy. Reason: {}",
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
                    self.gateway
                        .send(
                            &self.session_id,
                            GatewayPayload::Confirm {
                                id: uuid::Uuid::new_v4().to_string(),
                                action: TOOL_BASH.to_string(),
                                details: format!("{} (Policy: {})", command_str, reason),
                            },
                        )
                        .await?;
                    self.update_state(WorkflowState::Paused).await?;
                    return Ok(Some(ReinforcedResult {
                        content: "WAITING_FOR_USER_APPROVAL".to_string(),
                        title: format!("Bash({})", command_str),
                        summary: "Waiting for approval".to_string(),
                        is_error: false,
                        error_type: None,
                        display_type: "text".to_string(),
                    }));
                }
            }
        }
        Ok(None)
    }

    pub(crate) fn handle_fs_path_guard_intercept(
        &self,
        name: &str,
        args: &serde_json::Value,
    ) -> Result<(), WorkflowEngineError> {
        if let Some(path_str) = args["file_path"].as_str().or_else(|| args["path"].as_str()) {
            let guard = self.path_guard.read().map_err(|e| {
                WorkflowEngineError::General(format!("PathGuard lock poisoned: {}", e))
            })?;

            let is_write = [TOOL_WRITE_FILE, TOOL_EDIT_FILE].contains(&name);
            let is_planning = self.policy.phase == ExecutionPhase::Planning;

            guard.validate(std::path::Path::new(path_str), is_planning, is_write, false)?;
        }
        Ok(())
    }
}
