use serde_json::json;

use crate::tools::{
    READ_ONLY_BASH_CMDS_EXACT, READ_ONLY_BASH_PREFIXES, TOOL_BASH, TOOL_EDIT_FILE,
    TOOL_PLAN_EDIT_NOTE, TOOL_PLAN_WRITE_NOTE, TOOL_SUBMIT_PLAN, TOOL_WRITE_FILE,
};
use crate::workflow::react::engine::WorkflowExecutor;
use crate::workflow::react::error::WorkflowEngineError;
use crate::workflow::react::events::WorkflowEvent;
use crate::workflow::react::file_preview::attach_display_context;
use crate::workflow::react::intelligence::ToolApprovalReview;
use crate::workflow::react::observation::{ObservationReinforcer, ReinforcedResult};
use crate::workflow::react::policy::{ApprovalLevel, ExecutionPhase};
use crate::workflow::react::types::{GatewayPayload, StepType, WorkflowState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SmartApprovalDecision {
    AutoApprove,
    ReviewWithAi,
    ReviewByUser,
}

impl WorkflowExecutor {
    pub(crate) fn is_smart_mode_read_only_tool(name: &str) -> bool {
        matches!(
            name,
            crate::tools::TOOL_READ_FILE
                | crate::tools::TOOL_LIST_DIR
                | crate::tools::TOOL_GLOB
                | crate::tools::TOOL_GREP
                | crate::tools::TOOL_WEB_SEARCH
                | crate::tools::TOOL_WEB_FETCH
        )
    }

    pub(crate) fn is_smart_mode_auto_approved_tool(name: &str) -> bool {
        Self::is_smart_mode_read_only_tool(name) || matches!(name, TOOL_EDIT_FILE | TOOL_WRITE_FILE)
    }

    pub(crate) fn smart_mode_approval_decision(
        name: &str,
        args: &serde_json::Value,
    ) -> SmartApprovalDecision {
        if Self::is_smart_mode_auto_approved_tool(name) {
            return SmartApprovalDecision::AutoApprove;
        }

        if name == TOOL_BASH {
            let command_str = args["command"].as_str().unwrap_or("").trim();
            if Self::is_smart_mode_read_only_shell_command(command_str)
                || Self::is_smart_mode_safe_build_shell_command(command_str)
            {
                return SmartApprovalDecision::AutoApprove;
            }

            return SmartApprovalDecision::ReviewByUser;
        }

        SmartApprovalDecision::ReviewWithAi
    }

    fn normalized_finish_task_summary(text_part: &str) -> String {
        text_part
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .filter(|line| {
                let lower = line.to_ascii_lowercase();
                !matches!(
                    lower.as_str(),
                    "done" | "finished" | "complete" | "completed" | "task complete"
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn is_valid_finish_task_summary(text_part: &str) -> bool {
        let normalized = Self::normalized_finish_task_summary(text_part);
        if normalized.is_empty() {
            return false;
        }

        let non_whitespace_len = normalized.chars().filter(|c| !c.is_whitespace()).count();
        if non_whitespace_len < 32 {
            return false;
        }

        let meaningful_lines = normalized
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count();
        let has_sentence_shape = normalized.contains('\n')
            || normalized.contains('。')
            || normalized.contains('.')
            || normalized.contains(':');

        meaningful_lines >= 2 || has_sentence_shape
    }

    fn finish_task_summary_from_args(args: &serde_json::Value) -> Option<String> {
        let summary = args
            .get("summary")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .unwrap_or_default();

        Self::is_valid_finish_task_summary(summary).then(|| summary.to_string())
    }

    fn is_read_only_shell_stage(stage: &str) -> bool {
        let stage = stage.trim();
        if stage.is_empty() {
            return true;
        }

        let stage = stage
            .replace("2>&1", " ")
            .replace("1>&2", " ")
            .replace("2>/dev/null", " ")
            .replace("2> /dev/null", " ")
            .replace("1>/dev/null", " ")
            .replace("1> /dev/null", " ")
            .replace("&>/dev/null", " ");
        let stage = stage.trim().to_lowercase();
        if stage.is_empty() {
            return true;
        }

        READ_ONLY_BASH_CMDS_EXACT.contains(stage.as_str())
            || READ_ONLY_BASH_PREFIXES
                .iter()
                .any(|&prefix| stage.starts_with(prefix))
    }

    fn has_shell_redirection(stage: &str) -> bool {
        let stage = stage
            .replace("2>&1", " ")
            .replace("1>&2", " ")
            .replace("2>/dev/null", " ")
            .replace("2> /dev/null", " ")
            .replace("1>/dev/null", " ")
            .replace("1> /dev/null", " ")
            .replace("&>/dev/null", " ");

        stage.contains('>') || stage.contains('<')
    }

    fn is_safe_shell_filter(stage: &str) -> bool {
        let stage = stage.trim().to_lowercase();
        if stage.is_empty() {
            return true;
        }

        const SAFE_FILTER_PREFIXES: &[&str] = &[
            "tail ", "head ", "grep ", "egrep ", "fgrep ", "less", "more", "sed ", "awk ", "cut ",
            "sort ", "uniq ", "wc ", "tr ", "jq ",
        ];

        SAFE_FILTER_PREFIXES
            .iter()
            .any(|&prefix| stage.starts_with(prefix))
    }

    fn strip_workspace_navigation_prefix(command_str: &str) -> String {
        let mut candidate = command_str.trim().to_lowercase();
        if let Some(rest) = candidate
            .strip_prefix("cd ")
            .or_else(|| candidate.strip_prefix("pushd "))
        {
            for delimiter in ["&&", "||", ";"] {
                if let Some((_, rest)) = rest.split_once(delimiter) {
                    candidate = rest.trim().to_string();
                    break;
                }
            }
        }
        candidate
    }

    fn is_safe_package_build_stage(stage: &str) -> bool {
        let tokens = match shlex::split(stage) {
            Some(tokens) => tokens,
            None => return false,
        };
        if tokens.is_empty() {
            return false;
        }

        match tokens[0].as_str() {
            "npm" | "pnpm" | "yarn" => tokens.get(1).map(String::as_str) == Some("build"),
            _ => false,
        }
    }

    fn is_smart_mode_safe_build_shell_command(command_str: &str) -> bool {
        let candidate = Self::strip_workspace_navigation_prefix(command_str);
        if candidate.is_empty() {
            return false;
        }

        for segment in candidate.split("&&") {
            for segment in segment.split("||") {
                for segment in segment.split(';') {
                    let segment = segment.trim();
                    if segment.is_empty() {
                        continue;
                    }

                    let mut stage_iter = segment.split('|');
                    let first_stage = stage_iter.next().unwrap_or("").trim();
                    if Self::has_shell_redirection(first_stage) {
                        return false;
                    }
                    if !Self::is_safe_package_build_stage(first_stage) {
                        return false;
                    }

                    for stage in stage_iter {
                        if Self::has_shell_redirection(stage) {
                            return false;
                        }
                        if !Self::is_safe_shell_filter(stage) {
                            return false;
                        }
                    }
                }
            }
        }

        true
    }

    fn is_smart_mode_read_only_shell_command(command_str: &str) -> bool {
        let candidate = Self::strip_workspace_navigation_prefix(command_str);
        if candidate.is_empty() {
            return false;
        }

        for segment in candidate.split("&&") {
            for segment in segment.split("||") {
                for segment in segment.split(';') {
                    let segment = segment.trim();
                    if segment.is_empty() {
                        continue;
                    }

                    let mut stage_iter = segment.split('|');
                    let first_stage = stage_iter.next().unwrap_or("").trim();
                    if Self::has_shell_redirection(first_stage) {
                        return false;
                    }
                    if !Self::is_read_only_shell_stage(first_stage) {
                        return false;
                    }

                    for stage in stage_iter {
                        if Self::has_shell_redirection(stage) {
                            return false;
                        }
                        if !Self::is_safe_shell_filter(stage) {
                            return false;
                        }
                    }
                }
            }
        }

        true
    }

    pub(crate) async fn review_tool_call_for_smart_mode(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
        assistant_text: &str,
    ) -> Result<Option<ToolApprovalReview>, WorkflowEngineError> {
        let tool = match self.tool_manager.get_tool(tool_name).await {
            Ok(tool) => tool,
            Err(error) => {
                log::warn!(
                    "WorkflowExecutor {}: Skipping AI approval review for unknown tool '{}': {}",
                    self.session_id,
                    tool_name,
                    error
                );
                return Ok(None);
            }
        };

        match self
            .intelligence_manager
            .review_tool_approval(
                &self.context,
                &self.llm_processor.build_workspace_context(),
                tool_name,
                &tool.category().to_string(),
                tool.scope().as_str(),
                tool.description(),
                args,
                assistant_text,
            )
            .await
        {
            Ok(review) => {
                log::info!(
                    "WorkflowExecutor {}: Smart approval AI review for '{}' -> approved={}, risk_level={}, reason={}",
                    self.session_id,
                    tool_name,
                    review.approved,
                    review.risk_level,
                    review.reason
                );
                Ok(Some(review))
            }
            Err(error) => {
                log::warn!(
                    "WorkflowExecutor {}: AI approval review failed for '{}': {}",
                    self.session_id,
                    tool_name,
                    error
                );
                Ok(None)
            }
        }
    }

    /// Determines if a tool call should be intercepted for user approval based on the current ApprovalLevel.
    pub(crate) fn should_intercept_for_approval(
        &mut self,
        name: &str,
        args: &serde_json::Value,
    ) -> bool {
        if crate::tools::is_auto_execute_workflow_tool(name) {
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

        if name == TOOL_BASH {
            let command_str = args["command"].as_str().unwrap_or("").trim();
            if !command_str.is_empty() && self.smart_approved_bash_commands.remove(command_str) {
                log::info!(
                    "WorkflowExecutor {}: Skipping approval for Smart-AI-approved bash command: {}",
                    self.session_id,
                    command_str
                );
                return false;
            }
        }

        // Special handling for bash: Check shell policy first
        // If a command is explicitly allowed in shell_policy (e.g. via Approve All), don't intercept
        if name == TOOL_BASH {
            let command_str = args["command"].as_str().unwrap_or("").trim();
            if !command_str.is_empty() {
                let custom_rules: Vec<crate::tools::ShellPolicyRule> = self
                    .agent_config
                    .shell_policy
                    .as_deref()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or_default();
                let policy_engine =
                    crate::tools::ShellPolicyEngine::new(self.path_guard.clone(), custom_rules);

                if let crate::tools::ShellDecision::Allow =
                    policy_engine.check(command_str, self.policy.phase == ExecutionPhase::Planning)
                {
                    log::info!(
                        "WorkflowExecutor {}: Auto-approving bash command allowed by policy: {}",
                        self.session_id,
                        command_str
                    );
                    return false;
                }
            }
        }

        // Smart mode: allow read/write tools, intercept risky bash or unknown mutations
        if self.policy.approval_level == ApprovalLevel::Smart {
            match Self::smart_mode_approval_decision(name, args) {
                SmartApprovalDecision::AutoApprove => {
                    if name == TOOL_BASH {
                        let command_str = args["command"].as_str().unwrap_or("").trim();
                        log::info!(
                            "WorkflowExecutor {}: Skipping approval for Smart-mode read-only bash command: {}",
                            self.session_id,
                            command_str
                        );
                    }
                    return false;
                }
                SmartApprovalDecision::ReviewWithAi | SmartApprovalDecision::ReviewByUser => {
                    if name == TOOL_BASH {
                        let command_str =
                            args["command"].as_str().unwrap_or("").trim().to_lowercase();
                        let has_operators = command_str
                            .chars()
                            .any(|c| matches!(c, '>' | '<' | '|' | '&' | ';'));
                        if has_operators {
                            log::info!(
                                "WorkflowExecutor {}: Intercepting bash due to shell operators: {}",
                                self.session_id,
                                command_str
                            );
                        }
                    }
                    return true;
                }
            }
        }

        // Default mode: intercept everything else
        true
    }

    pub(crate) async fn handle_submit_plan_intercept(
        &mut self,
        id: &str,
        args: &serde_json::Value,
        _text_part: &str,
    ) -> Result<Option<ReinforcedResult>, WorkflowEngineError> {
        let plan_from_args = args
            .get("plan")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .trim();

        if plan_from_args.is_empty() {
            return Ok(Some(ReinforcedResult {
                content: "<SYSTEM_REMINDER>Error: You called 'submit_plan' without a non-empty `plan` argument. The approved plan MUST come from the structured tool argument `submit_plan.plan`, not from free-form assistant text. Put the complete plan in `plan` and call `submit_plan` again.</SYSTEM_REMINDER>".into(),
                llm_content: None,
                title: "SubmitPlan Error".to_string(),
                summary: "Missing plan payload".to_string(),
                is_error: true,
                error_type: Some("MissingPlan".into()),
                display_type: "text".to_string(),
                approval_status: None,
                observation_kind: None,
            }));
        }

        self.handle_approval_interception(id, TOOL_SUBMIT_PLAN, args, None)
            .await
    }

    pub(crate) async fn handle_ask_user_intercept(
        &mut self,
        args: &serde_json::Value,
    ) -> Result<Option<ReinforcedResult>, WorkflowEngineError> {
        let groups = if let Some(groups) = args.as_array() {
            groups.clone()
        } else if args.get("title").is_some() || args.get("options").is_some() {
            vec![args.clone()]
        } else if let Some(groups) = args.get("items").and_then(|value| value.as_array()) {
            groups.clone()
        } else if let Some(groups) = args.get("groups").and_then(|value| value.as_array()) {
            groups.clone()
        } else if let (Some(question), Some(options)) = (
            args.get("question").and_then(|value| value.as_str()),
            args.get("options").and_then(|value| value.as_array()),
        ) {
            vec![json!({
                "title": question,
                "options": options
            })]
        } else {
            Vec::new()
        };

        let normalized_groups: Vec<serde_json::Value> = groups
            .into_iter()
            .filter_map(|group| {
                let title = group
                    .get("title")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())?;

                let options: Vec<String> = group
                    .get("options")
                    .and_then(|value| value.as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                    .collect();

                if options.is_empty() {
                    return None;
                }

                Some(json!({
                    "title": title,
                    "options": options
                }))
            })
            .collect();

        if normalized_groups.is_empty() {
            return Ok(Some(ReinforcedResult {
                content: "<SYSTEM_REMINDER>Error: 'ask_user' requires grouped choices with at least one valid item. Use {\"items\":[{\"title\":\"...\",\"options\":[\"...\"]}]} and ensure every group has a direct decision title plus at least one concise, actionable option. Use ask_user only for blocking decisions required to continue; do not use it for status updates, generic feedback, final answers, or plan approval. Do not include custom-input placeholder options because the UI already provides custom text input.</SYSTEM_REMINDER>".to_string(),
                llm_content: None,
                title: "Ask User Error".to_string(),
                summary: "Invalid ask_user payload".to_string(),
                is_error: true,
                error_type: Some("InvalidAskUserPayload".to_string()),
                display_type: "text".to_string(),
                approval_status: None,
                observation_kind: None,
            }));
        }

        self.update_state(WorkflowState::AwaitingUser).await?;
        let content =
            serde_json::to_string(&normalized_groups).unwrap_or_else(|_| "[]".to_string());

        Ok(Some(ReinforcedResult {
            content,
            llm_content: None,
            title: "Ask User".to_string(),
            summary: "Waiting for user".to_string(),
            is_error: false,
            error_type: None,
            display_type: "choice".to_string(),
            approval_status: None,
            observation_kind: None,
        }))
    }

    pub(crate) async fn handle_finish_task_intercept(
        &mut self,
        text_part: &str,
        args: &serde_json::Value,
    ) -> Result<Option<ReinforcedResult>, WorkflowEngineError> {
        let text_summary_valid = Self::is_valid_finish_task_summary(text_part);
        let argument_summary = Self::finish_task_summary_from_args(args);
        if !text_summary_valid && argument_summary.is_none() {
            return Ok(Some(ReinforcedResult {
                content: "<SYSTEM_REMINDER>Error: You called 'complete_workflow_with_summary' without a valid user-visible completion report. Reasoning/thinking text does NOT count. Provide the report either as normal assistant content before the tool call or in the `summary` argument of complete_workflow_with_summary. Do NOT call sub_agent_output to retrieve call-mode sub-agent results; call-mode results are already delivered as sub-agent completion observations. First, provide a brief user-visible summary that explicitly covers: 1) what was completed, 2) what was verified, and 3) any important remaining notes or limitations. If a sub-agent produced findings, synthesize those findings into your own response instead of copying them as the final answer. After that valid report is present, call complete_workflow_with_summary again.</SYSTEM_REMINDER>".into(),
                llm_content: None,
                title: "FinishTask Error".to_string(),
                summary: "Invalid completion report".to_string(),
                is_error: true,
                error_type: Some("InvalidFinishSummary".into()),
                display_type: "text".to_string(),
                approval_status: None,
                observation_kind: None,
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
                    .map(|t| {
                        format!(
                            "[{}] {} (ID: {})",
                            t["status"].as_str().unwrap_or("?"),
                            t["subject"].as_str().unwrap_or("Untitled"),
                            t["id"].as_str().unwrap_or("?")
                        )
                    })
                    .collect();

                if !active_tasks.is_empty() {
                    return Ok(Some(ReinforcedResult {
                        content: format!("<SYSTEM_REMINDER>Block: You still have active tasks: {}. Do NOT retry complete_workflow_with_summary yet. Do NOT call sub_agent_output for call-mode sub-agents; their results are delivered directly as observations. You MUST either complete these todos, or mark them as 'failed' or 'data_missing' if they cannot be fulfilled, before calling complete_workflow_with_summary.</SYSTEM_REMINDER>", active_tasks.join(", ")),
                        llm_content: None,
                        title: "Tasks Pending".to_string(),
                        summary: "Incomplete todos".to_string(),
                        is_error: true,
                        error_type: Some("PendingTodos".into()),
                        display_type: "text".to_string(),
                        approval_status: None,
                        observation_kind: None,
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
                    content: format!("<SYSTEM_REMINDER>Audit Rejected: Your conclusion was deemed incomplete. Feedback: {}\n\nYou MUST address these points before you can call complete_workflow_with_summary. Do NOT call sub_agent_output for call-mode sub-agent results; use the sub-agent completion observations already in context.</SYSTEM_REMINDER>", audit_feedback),
                    llm_content: None,
                    title: "Audit Rejected".to_string(),
                    summary: "Audit failed".to_string(),
                    is_error: true,
                    error_type: Some("AuditRejected".into()),
                    display_type: "text".to_string(),
                    approval_status: None,
                    observation_kind: None,
                }));
            }
        }

        let completion_summary = if text_summary_valid {
            Self::normalized_finish_task_summary(text_part)
        } else {
            let summary = argument_summary.unwrap_or_default();
            self.add_message_and_notify_internal(
                "assistant".to_string(),
                summary.clone(),
                None,
                None,
                Some(StepType::Think),
                false,
                None,
                Some(json!({
                    "message_kind": "completion_report",
                    "source": "complete_workflow_with_summary.summary"
                })),
            )
            .await?;
            summary
        };

        Ok(Some(ReinforcedResult {
            content: "Finished".into(),
            llm_content: None,
            title: "Complete Workflow with Summary".to_string(),
            summary: completion_summary,
            is_error: false,
            error_type: None,
            display_type: "text".to_string(),
            approval_status: None,
            observation_kind: None,
        }))
    }

    pub(crate) async fn handle_submit_result_intercept(
        &mut self,
        args: &serde_json::Value,
    ) -> Result<Option<ReinforcedResult>, WorkflowEngineError> {
        let result = args
            .get("result")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .unwrap_or_default();
        let summary = args
            .get("summary")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .unwrap_or_default();

        if result.is_empty() || summary.is_empty() {
            return Ok(Some(ReinforcedResult {
                content: "<SYSTEM_REMINDER>Error: `submit_result` requires both a non-empty `result` and a non-empty `summary`. Put the full delegated output in `result` and a short notification-safe summary in `summary`, then call `submit_result` again.</SYSTEM_REMINDER>".into(),
                llm_content: None,
                title: "Submit Result Error".to_string(),
                summary: "Missing result payload".to_string(),
                is_error: true,
                error_type: Some("InvalidSubmitResult".into()),
                display_type: "text".to_string(),
                approval_status: None,
                observation_kind: None,
            }));
        }

        Ok(Some(ReinforcedResult {
            content: result.to_string(),
            llm_content: None,
            title: "Submit Result".to_string(),
            summary: summary.to_string(),
            is_error: false,
            error_type: None,
            display_type: "text".to_string(),
            approval_status: None,
            observation_kind: None,
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
                        llm_content: None,
                        title: format!("Bash({})", command_str),
                        summary: "Blocked".to_string(),
                        is_error: true,
                        error_type: Some("Security".to_string()),
                        display_type: "text".to_string(),
                        approval_status: None,
                        observation_kind: None,
                    }));
                }
                crate::tools::ShellDecision::Review(reason) => {
                    if self.policy.approval_level == ApprovalLevel::Full {
                        log::info!(
                            "WorkflowExecutor {}: Auto-approving risky bash command in Full mode (Policy: {})",
                            self.session_id, reason
                        );
                    } else if self.policy.approval_level == ApprovalLevel::Smart {
                        // In Smart mode, allow read-only diagnostic commands even if they use
                        // command chaining or output shaping to trim noisy output.
                        if Self::is_smart_mode_read_only_shell_command(command_str)
                            || Self::is_smart_mode_safe_build_shell_command(command_str)
                        {
                            log::info!(
                                "WorkflowExecutor {}: Auto-approving low-risk bash command in Smart mode: {}",
                                self.session_id, command_str
                            );
                            return Ok(None);
                        }

                        // In Smart mode, check if this is a read-only command before intercepting
                        let command_str_lower = command_str.to_lowercase();
                        let is_read_only = READ_ONLY_BASH_CMDS_EXACT
                            .contains(command_str_lower.as_str())
                            || READ_ONLY_BASH_PREFIXES
                                .iter()
                                .any(|&p| command_str_lower.starts_with(p));

                        if is_read_only {
                            log::info!(
                                "WorkflowExecutor {}: Auto-approving read-only bash command in Smart mode: {}",
                                self.session_id, command_str
                            );
                            // Don't intercept - allow the read-only command
                        } else {
                            if let Some(review) = self
                                .review_tool_call_for_smart_mode(TOOL_BASH, args, command_str)
                                .await?
                            {
                                if review.approved {
                                    self.smart_approved_bash_commands
                                        .insert(command_str.trim().to_string());
                                    log::info!(
                                        "WorkflowExecutor {}: AI approved bash command in Smart mode (risk: {}, reason: {})",
                                        self.session_id,
                                        review.risk_level,
                                        review.reason
                                    );
                                    return Ok(None);
                                }

                                log::info!(
                                    "WorkflowExecutor {}: AI did not auto-approve bash command in Smart mode (risk: {}, reason: {})",
                                    self.session_id,
                                    review.risk_level,
                                    review.reason
                                );
                            }

                            log::info!(
                                "WorkflowExecutor {}: Intercepting bash command for review in Smart mode: {}",
                                self.session_id, reason
                            );
                            let display_content = command_str.to_string();
                            return self
                                .handle_approval_interception(
                                    id,
                                    TOOL_BASH,
                                    args,
                                    Some(display_content),
                                )
                                .await;
                        }
                    } else {
                        log::info!(
                            "WorkflowExecutor {}: Intercepting bash command for review: {}",
                            self.session_id,
                            reason
                        );
                        // Delegate to unified approval handler with descriptive command preview
                        let display_content = command_str.to_string();
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
        // 1. Determine what to show the user in the UI (Generate Diffs for File Ops)
        let mut display_type = "text".to_string();
        let (content_str, details_value) = if let Some(custom) = display_content {
            (custom.clone(), serde_json::json!(custom))
        } else {
            match name {
                TOOL_EDIT_FILE | TOOL_WRITE_FILE | TOOL_PLAN_EDIT_NOTE | TOOL_PLAN_WRITE_NOTE => {
                    display_type = "diff".to_string();
                    let mut preview_args = args.clone();
                    attach_display_context(&mut preview_args, false);
                    let preview_limit = 100_000;

                    // Truncate large fields for UI preview to prevent IPC/Rendering lag
                    if let Some(content) = preview_args.get_mut("content") {
                        if let Some(s) = content.as_str() {
                            if s.chars().count() > preview_limit {
                                let truncated: String = s.chars().take(preview_limit).collect();
                                *content = serde_json::json!(format!(
                                    "{}\n... (truncated for preview)",
                                    truncated
                                ));
                            }
                        }
                    }
                    if let Some(old_s) = preview_args.get_mut("old_string") {
                        if let Some(s) = old_s.as_str() {
                            if s.chars().count() > preview_limit {
                                let truncated: String = s.chars().take(preview_limit).collect();
                                *old_s = serde_json::json!(format!(
                                    "{}\n... (truncated for preview)",
                                    truncated
                                ));
                            }
                        }
                    }
                    if let Some(new_s) = preview_args.get_mut("new_string") {
                        if let Some(s) = new_s.as_str() {
                            if s.chars().count() > preview_limit {
                                let truncated: String = s.chars().take(preview_limit).collect();
                                *new_s = serde_json::json!(format!(
                                    "{}\n... (truncated for preview)",
                                    truncated
                                ));
                            }
                        }
                    }

                    // content_str for storing in messages, details_value for UI payload
                    (
                        serde_json::to_string(&preview_args).unwrap_or_default(),
                        preview_args,
                    )
                }
                TOOL_BASH => {
                    let command = args
                        .get("command")
                        .and_then(|value| value.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    let preview = if command.is_empty() {
                        serde_json::to_string_pretty(args).unwrap_or_default()
                    } else {
                        command.clone()
                    };
                    (
                        preview.clone(),
                        serde_json::json!({
                            "command": command,
                        }),
                    )
                }
                TOOL_SUBMIT_PLAN => {
                    display_type = "markdown".to_string();
                    let plan = args
                        .get("plan")
                        .and_then(|value| value.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    let preview = if plan.is_empty() {
                        serde_json::to_string_pretty(args).unwrap_or_default()
                    } else {
                        plan
                    };
                    (preview.clone(), serde_json::json!(preview))
                }
                _ => {
                    let msg = format!(
                        "Tool: {}\nArguments: {}",
                        name,
                        serde_json::to_string_pretty(args).unwrap_or_default()
                    );
                    (msg.clone(), serde_json::json!(msg))
                }
            }
        };

        // 2. Stash the full tool name, arguments, and details for later request_confirm_broadcast
        let stash_obj = json!({
            "name": name,
            "arguments": args,
            "details": content_str.clone(),
            "display_type": display_type.clone()
        });
        self.pending_approvals.insert(id.to_string(), stash_obj);
        self.enqueue_pending_approval(id);

        self.update_state(WorkflowState::AwaitingApproval).await?;

        let event = WorkflowEvent::approval_requested(
            self.session_id.clone(),
            id.to_string(),
            name.to_string(),
            args.clone(),
            Some(content_str.clone()),
            Some(display_type.clone()),
        );
        if let Err(e) = self.append_event(&event) {
            log::error!(
                "[Workflow][session={}] workflow.event.append_failed - error={}",
                self.session_id,
                e
            );
        }

        self.gateway
            .send(
                &self.session_id,
                GatewayPayload::Confirm {
                    id: id.to_string(),
                    action: name.to_string(),
                    details: details_value,
                    display_type: Some(display_type.clone()),
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
            content: content_str,
            llm_content: None,
            title: pretty_title,
            summary: rust_i18n::t!("workflow.awaiting_approval").to_string(),
            is_error: false,
            error_type: None,
            display_type,
            approval_status: Some("pending".to_string()),
            observation_kind: None,
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
                    llm_content: None,
                    title: format!("Security Check: {}", name),
                    summary: "Access Denied".to_string(),
                    is_error: true,
                    error_type: Some("Security".to_string()),
                    display_type: "text".to_string(),
                    approval_status: None,
                    observation_kind: None,
                }));
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::{SmartApprovalDecision, WorkflowExecutor};
    use crate::tools::{TOOL_BASH, TOOL_EDIT_FILE, TOOL_READ_FILE, TOOL_WRITE_FILE};
    use serde_json::json;

    #[test]
    fn finish_task_summary_rejects_placeholder_text() {
        assert!(!WorkflowExecutor::is_valid_finish_task_summary("done"));
        assert!(!WorkflowExecutor::is_valid_finish_task_summary(
            "<think>looks good</think>"
        ));
        assert!(!WorkflowExecutor::is_valid_finish_task_summary("Completed"));
    }

    #[test]
    fn finish_task_summary_accepts_meaningful_user_visible_report() {
        assert!(WorkflowExecutor::is_valid_finish_task_summary(
            "Implemented the workflow fix and verified it with cargo check.\nRemaining note: UI behavior still needs manual confirmation."
        ));
    }

    #[test]
    fn finish_task_summary_accepts_meaningful_tool_argument() {
        let args = json!({
            "summary": "Committed the requested code and pushed it to origin/main.\nVerified the push completed without errors."
        });

        assert_eq!(
            WorkflowExecutor::finish_task_summary_from_args(&args).as_deref(),
            Some(
                "Committed the requested code and pushed it to origin/main.\nVerified the push completed without errors."
            )
        );
    }

    #[test]
    fn finish_task_summary_rejects_placeholder_tool_argument() {
        let args = json!({ "summary": "done" });

        assert!(WorkflowExecutor::finish_task_summary_from_args(&args).is_none());
    }

    #[test]
    fn smart_mode_auto_approves_read_only_tools_and_file_writes() {
        assert_eq!(
            WorkflowExecutor::smart_mode_approval_decision(TOOL_READ_FILE, &json!({})),
            SmartApprovalDecision::AutoApprove
        );
        assert_eq!(
            WorkflowExecutor::smart_mode_approval_decision(
                TOOL_EDIT_FILE,
                &json!({"file_path":"/tmp/test.rs"})
            ),
            SmartApprovalDecision::AutoApprove
        );
        assert_eq!(
            WorkflowExecutor::smart_mode_approval_decision(
                TOOL_WRITE_FILE,
                &json!({"file_path":"/tmp/test.rs","content":"x"})
            ),
            SmartApprovalDecision::AutoApprove
        );
    }

    #[test]
    fn smart_mode_auto_approves_read_only_bash_diagnostics_with_chaining() {
        assert_eq!(
            WorkflowExecutor::smart_mode_approval_decision(
                TOOL_BASH,
                &json!({"command":"cd /repo && git status"})
            ),
            SmartApprovalDecision::AutoApprove
        );
        assert_eq!(
            WorkflowExecutor::smart_mode_approval_decision(
                TOOL_BASH,
                &json!({"command":"git log --oneline | head -20"})
            ),
            SmartApprovalDecision::AutoApprove
        );
    }

    #[test]
    fn smart_mode_auto_approves_common_package_build_commands() {
        for command in [
            "npm build",
            "pnpm build",
            "yarn build",
            "cd /repo && pnpm build | tail -20",
        ] {
            assert_eq!(
                WorkflowExecutor::smart_mode_approval_decision(
                    TOOL_BASH,
                    &json!({ "command": command })
                ),
                SmartApprovalDecision::AutoApprove,
                "command should be auto-approved: {}",
                command
            );
        }

        for command in [
            "npm run build",
            "npm run -s build",
            "npm run-script build",
            "pnpm run build",
            "yarn run build",
            "pnpm tauri dev",
            "pnpm dev",
        ] {
            assert_eq!(
                WorkflowExecutor::smart_mode_approval_decision(
                    TOOL_BASH,
                    &json!({ "command": command })
                ),
                SmartApprovalDecision::ReviewByUser,
                "command should require user approval: {}",
                command
            );
        }
    }

    #[test]
    fn smart_mode_sends_unknown_mutations_to_ai_review() {
        assert_eq!(
            WorkflowExecutor::smart_mode_approval_decision(
                "delete_file",
                &json!({"file_path":"/tmp/test.rs"})
            ),
            SmartApprovalDecision::ReviewWithAi
        );
    }

    #[test]
    fn smart_mode_routes_non_read_only_bash_to_user_review() {
        assert_eq!(
            WorkflowExecutor::smart_mode_approval_decision(
                TOOL_BASH,
                &json!({"command":"cargo test"})
            ),
            SmartApprovalDecision::ReviewByUser
        );
        assert_eq!(
            WorkflowExecutor::smart_mode_approval_decision(
                TOOL_BASH,
                &json!({"command":"cat secret.txt > out.txt"})
            ),
            SmartApprovalDecision::ReviewByUser
        );
    }
}
