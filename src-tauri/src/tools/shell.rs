use crate::ai::traits::chat::MCPToolDeclaration;
use crate::libs::ai_temp::ToolOutputWriter;
use crate::tools::helper::is_node_build_command;
use crate::tools::helper::{
    classify_shell_stage, parse_safe_compound_command, shell_tokens, split_shell_command_segments,
    SafeCompoundCommand, SafeCompoundStage, ShellStage,
};
use crate::tools::shell_output::{
    build_compound_shell_tool_result, build_shell_tool_result, prepare_shell_output,
    should_collect_stderr_line_as_stdout, AnsiOutputSanitizer, CompoundShellStageResult,
};
use crate::tools::{NativeToolResult, ToolCategory, ToolDefinition, ToolError};
use crate::workflow::react::error::WorkflowEngineError;
use crate::workflow::react::gateway::Gateway;
use crate::workflow::react::security::PathGuard;
use crate::workflow::react::types::GatewayPayload;
use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::time::{timeout, Duration, Instant};

/// Decision levels for shell auditing
#[derive(Debug, PartialEq, Clone, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ShellDecision {
    Allow,
    Review(String),
    Deny(String),
}

impl<'de> serde::Deserialize<'de> for ShellDecision {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?.to_lowercase();
        match s.as_str() {
            "allow" => Ok(ShellDecision::Allow),
            s if s.starts_with("review") => {
                // Handle "review" or "review:reason" format
                let reason = if s.len() > 6 {
                    s[6..].trim_start_matches(':').to_string()
                } else {
                    "Requires review".to_string()
                };
                Ok(ShellDecision::Review(reason))
            }
            s if s.starts_with("deny") => {
                let reason = if s.len() > 4 {
                    s[4..].trim_start_matches(':').to_string()
                } else {
                    "Command denied".to_string()
                };
                Ok(ShellDecision::Deny(reason))
            }
            _ => Ok(ShellDecision::Review(
                "Unknown decision, requires review".to_string(),
            )),
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct ShellPolicyRule {
    pub pattern: String,
    pub decision: ShellDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Industrial-grade Shell Policy Engine with graded auditing.
pub struct ShellPolicyEngine {
    path_guard: Arc<RwLock<PathGuard>>,
    custom_rules: Vec<ShellPolicyRule>,
}

impl ShellPolicyEngine {
    pub fn new(path_guard: Arc<RwLock<PathGuard>>, custom_rules: Vec<ShellPolicyRule>) -> Self {
        Self {
            path_guard,
            custom_rules,
        }
    }

    pub fn check(&self, command_str: &str, restrict_to_planning: bool) -> ShellDecision {
        // 1. Initial Sanity Check: Block dangerous invisible characters
        for c in command_str.chars() {
            if (c.is_control() && c != '\n' && c != '\r' && c != '\t')
                || ('\u{2000}'..='\u{200F}').contains(&c)
                || ('\u{202A}'..='\u{202F}').contains(&c)
                || c == '\u{FEFF}'
            {
                return ShellDecision::Deny(format!(
                    "Dangerous hidden character detected (U+{:04X}). Obfuscation is forbidden.",
                    c as u32
                ));
            }
        }

        if command_str.trim().is_empty() {
            return ShellDecision::Deny("Command is empty".into());
        }

        // 2. Custom Regex/Pattern matching for the full command line or normalized stages.
        if !self.custom_rules.is_empty() {
            return self.evaluate_custom_rules(command_str, restrict_to_planning);
        }

        // 3. Recursive Check: Audit nested structure contents
        let nested_patterns = [
            (
                Regex::new(r"\$\((?P<inner>.*?)\)").unwrap(),
                "Command substitution $(...)",
            ),
            (
                Regex::new(r"`(?P<inner>.*?)`").unwrap(),
                "Command substitution `...`",
            ),
            (
                Regex::new(r"<\s*\((?P<inner>.*?)\)").unwrap(),
                "Process substitution <(...)",
            ),
            (
                Regex::new(r">\s*\((?P<inner>.*?)\)").unwrap(),
                "Process substitution >(...)",
            ),
        ];

        for (re, desc) in &nested_patterns {
            for cap in re.captures_iter(command_str) {
                if let Some(inner) = cap.name("inner") {
                    match self.check(inner.as_str(), restrict_to_planning) {
                        ShellDecision::Deny(reason) => {
                            return ShellDecision::Deny(format!(
                                "Dangerous command in {}: {}",
                                desc, reason
                            ))
                        }
                        ShellDecision::Review(reason) => {
                            return ShellDecision::Review(format!(
                                "Review required for {}: {}",
                                desc, reason
                            ))
                        }
                        ShellDecision::Allow => {}
                    }
                }
            }
        }

        // 4. Pre-process for Tokenization (Quote-aware operator spacing)
        let mut processed_cmd = String::new();
        let mut in_single_quote = false;
        let mut in_double_quote = false;
        let mut escaped = false;

        for c in command_str.chars() {
            if escaped {
                processed_cmd.push(c);
                escaped = false;
                continue;
            }
            if c == '\\' && !in_single_quote {
                escaped = true;
                processed_cmd.push(c);
                continue;
            }
            if c == '\'' && !in_double_quote {
                in_single_quote = !in_single_quote;
                processed_cmd.push(c);
                continue;
            }
            if c == '"' && !in_single_quote {
                in_double_quote = !in_double_quote;
                processed_cmd.push(c);
                continue;
            }

            if !in_single_quote && !in_double_quote {
                match c {
                    ';' | '|' | '&' | '>' | '<' => {
                        processed_cmd.push(' ');
                        processed_cmd.push(c);
                        processed_cmd.push(' ');
                    }
                    _ => processed_cmd.push(c),
                }
            } else {
                processed_cmd.push(c);
            }
        }

        // 5. Tokenization
        let tokens = match shlex::split(&processed_cmd) {
            Some(t) => t,
            None => return ShellDecision::Deny("Invalid shell syntax".into()),
        };

        // 6. Graded Audit Context
        let mut next_is_binary = true;
        let separators = [";", "&&", "||", "|", "&", "-exec"];
        let redirection_ops = [">", ">>", "1>", "2>", "&>", "<"];

        let hard_deny = [
            "mkfs", "dd", "format", "fdisk", "parted", "sudo", "su", "ssh", "scp",
        ];
        let needs_review = [
            "rm",
            "mv",
            "chmod",
            "chown",
            "ln",
            "kill",
            "pkill",
            "crontab",
            "alias",
            "eval",
            "python",
            "perl",
            "ruby",
            "node",
            "php",
            "sh",
            "bash",
            "zsh",
            "source",
            "nc",
            "netcat",
            "nmap",
            "curl",
            "wget",
            "apt",
            "apt-get",
            "yum",
            "dnf",
            "brew",
            "docker",
            "podman",
            "systemctl",
            "service",
        ];

        let destructive_commands = ["rm", "mv", "chmod", "chown"];

        let mut final_decision = ShellDecision::Allow;
        let mut current_binary = String::new();
        let mut current_binary_arg_index = 0usize;

        for (i, token) in tokens.iter().enumerate() {
            let token_str = token.as_str();

            if separators.contains(&token_str) {
                next_is_binary = true;
                current_binary.clear();
                current_binary_arg_index = 0;
                continue;
            }

            if redirection_ops.contains(&token_str) {
                if let Some(next_token) = tokens.get(i + 1) {
                    if !next_token.starts_with('-') {
                        match self.validate_path_token(
                            next_token,
                            restrict_to_planning,
                            false,
                            true,
                        ) {
                            ShellDecision::Deny(reason) => return ShellDecision::Deny(reason),
                            ShellDecision::Review(reason) => {
                                if final_decision == ShellDecision::Allow {
                                    final_decision = ShellDecision::Review(reason);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                if final_decision == ShellDecision::Allow {
                    final_decision = ShellDecision::Review("File redirection detected".into());
                }
                continue;
            }

            let clean_token = token.replace('"', "").replace('\'', "").to_lowercase();

            if next_is_binary {
                current_binary = clean_token.clone();
                if hard_deny.contains(&clean_token.as_str()) {
                    return ShellDecision::Deny(format!(
                        "System-critical command '{}' is forbidden.",
                        clean_token
                    ));
                }
                if needs_review.contains(&clean_token.as_str()) {
                    if final_decision == ShellDecision::Allow {
                        final_decision = ShellDecision::Review(format!(
                            "Sensitive command '{}' requires manual approval.",
                            clean_token
                        ));
                    }
                }
                next_is_binary = false;
                current_binary_arg_index = 0;
                continue;
            }

            if !token.starts_with('-') {
                let is_delete = current_binary == "rm";
                let force_path_validation =
                    Self::should_force_path_validation(&current_binary, current_binary_arg_index);
                match self.validate_path_token(
                    token,
                    restrict_to_planning,
                    is_delete,
                    force_path_validation,
                ) {
                    ShellDecision::Deny(reason) => return ShellDecision::Deny(reason),
                    ShellDecision::Review(reason) => {
                        if final_decision == ShellDecision::Allow {
                            final_decision = ShellDecision::Review(reason);
                        }
                    }
                    ShellDecision::Allow => {
                        // Root protection check:
                        // If the current command is destructive and the path is an authorized root, DENY.
                        if destructive_commands.contains(&current_binary.as_str()) {
                            if let Ok(expanded) = shellexpand::full(token) {
                                let path_str: &str = expanded.as_ref();
                                let is_root = if let Ok(guard) = self.path_guard.read() {
                                    guard.is_authorized_root(Path::new(path_str))
                                } else {
                                    false
                                };

                                if is_root {
                                    return ShellDecision::Deny(format!(
                                        "Operation Denied: '{}' cannot be performed on the authorized root directory itself ({:?}).",
                                        current_binary, path_str
                                    ));
                                }
                            }
                        }
                    }
                }

                current_binary_arg_index += 1;
            }
        }

        final_decision
    }

    fn should_force_path_validation(command: &str, arg_index: usize) -> bool {
        match command {
            "cat" | "head" | "tail" | "less" | "more" | "bat" | "nl" | "wc" | "sort" | "uniq"
            | "ls" | "stat" | "file" | "du" | "diff" | "cmp" | "comm" => true,
            "grep" | "egrep" | "fgrep" | "rg" => arg_index >= 1,
            "sed" | "awk" => arg_index >= 1,
            "find" => arg_index == 0,
            _ => false,
        }
    }

    fn evaluate_custom_rules(
        &self,
        command_str: &str,
        restrict_to_planning: bool,
    ) -> ShellDecision {
        if let Some(decision) = self.match_custom_rule(command_str) {
            return decision;
        }

        let normalized_segments =
            match self.extract_policy_match_segments(command_str, restrict_to_planning) {
                Ok(segments) => segments,
                Err(decision) => return decision,
            };

        if normalized_segments.is_empty() {
            return ShellDecision::Allow;
        }

        let mut final_decision = ShellDecision::Allow;
        for segment in normalized_segments {
            let Some(decision) = self.match_custom_rule(&segment) else {
                return ShellDecision::Review("Requires review (not in allowed list)".to_string());
            };

            match decision {
                ShellDecision::Deny(reason) => return ShellDecision::Deny(reason),
                ShellDecision::Review(reason) => {
                    if final_decision == ShellDecision::Allow {
                        final_decision = ShellDecision::Review(reason);
                    }
                }
                ShellDecision::Allow => {}
            }
        }

        final_decision
    }

    fn match_custom_rule(&self, command_str: &str) -> Option<ShellDecision> {
        for rule in &self.custom_rules {
            if let Ok(re) = Regex::new(&rule.pattern) {
                if re.is_match(command_str) {
                    return Some(rule.decision.clone());
                }
            } else if command_str.contains(&rule.pattern) {
                return Some(rule.decision.clone());
            }
        }

        None
    }

    fn extract_policy_match_segments(
        &self,
        command_str: &str,
        restrict_to_planning: bool,
    ) -> Result<Vec<String>, ShellDecision> {
        let mut segments = Vec::new();

        for segment in split_shell_command_segments(command_str) {
            let Some(tokens) = shell_tokens(&segment) else {
                return Err(ShellDecision::Deny("Invalid shell syntax".into()));
            };
            if tokens.is_empty() {
                continue;
            }

            match classify_shell_stage(&segment) {
                Some(ShellStage::Navigation { command, target }) => {
                    match self.validate_navigation_segment(
                        command.as_str(),
                        target.as_deref(),
                        restrict_to_planning,
                    ) {
                        ShellDecision::Allow => continue,
                        decision => return Err(decision),
                    }
                }
                Some(ShellStage::Command { normalized, .. }) => segments.push(normalized),
                None => segments.push(segment),
            }
        }

        Ok(segments)
    }

    fn validate_navigation_segment(
        &self,
        command: &str,
        target: Option<&str>,
        restrict_to_planning: bool,
    ) -> ShellDecision {
        if command == "popd" {
            return ShellDecision::Allow;
        }

        let Some(target) = target else {
            return ShellDecision::Deny(
                "Directory navigation requires an explicit target within the authorized roots."
                    .into(),
            );
        };

        if target == "-" {
            return ShellDecision::Deny(
                "Directory navigation via shell history is not allowed.".into(),
            );
        }

        if target.starts_with('~') {
            return ShellDecision::Deny(
                "Tilde (~) expansion is blocked. Use absolute paths within the workspace.".into(),
            );
        }

        let expanded = match shellexpand::full(target) {
            Ok(expanded) => expanded,
            Err(err) => {
                return ShellDecision::Deny(format!(
                    "Expansion failed for token '{}': {}",
                    target, err
                ))
            }
        };

        let validated = if let Ok(guard) = self.path_guard.read() {
            guard.validate(
                Path::new(expanded.as_ref()),
                restrict_to_planning,
                true,
                false,
            )
        } else {
            Err(WorkflowEngineError::Security("Lock failed".into()))
        };

        match validated {
            Ok(path) => {
                if !path.exists() {
                    return ShellDecision::Deny(format!(
                        "Directory navigation target does not exist: {:?}",
                        path
                    ));
                }

                if !path.is_dir() {
                    return ShellDecision::Deny(format!(
                        "Directory navigation target is not a directory: {:?}",
                        path
                    ));
                }

                ShellDecision::Allow
            }
            Err(err) => ShellDecision::Deny(format!("Boundary Violation: {}", err)),
        }
    }

    fn validate_path_token(
        &self,
        token: &str,
        restrict_to_planning: bool,
        is_delete: bool,
        force_path_validation: bool,
    ) -> ShellDecision {
        if token.starts_with('~') {
            return ShellDecision::Deny(
                "Tilde (~) expansion is blocked. Use absolute paths within the workspace.".into(),
            );
        }

        let is_path_like = force_path_validation
            || token.contains('$')
            || token.starts_with('/')
            || token.starts_with('.')
            || token.contains('/')
            || token == ".."
            || token == ".";
        if is_path_like {
            match shellexpand::full(token) {
                Ok(expanded) => {
                    let expanded_str: &str = expanded.as_ref();
                    if force_path_validation
                        || expanded_str.contains('/')
                        || expanded_str.starts_with('.')
                    {
                        let valid = if let Ok(guard) = self.path_guard.read() {
                            guard.validate(
                                Path::new(expanded_str),
                                restrict_to_planning,
                                true,
                                is_delete,
                            )
                        } else {
                            Err(WorkflowEngineError::Security("Lock failed".into()))
                        };
                        match valid {
                            Ok(path) => {
                                // Precise Skill Check: Check if path starts with an authorized skill root
                                let is_skill = if let Ok(guard) = self.path_guard.read() {
                                    guard.is_within_skill_root(&path)
                                } else {
                                    false
                                };

                                if is_skill {
                                    return ShellDecision::Review(format!(
                                        "Executing script within authorized skills directory: {:?}",
                                        path
                                    ));
                                }
                            }
                            Err(e) => {
                                return ShellDecision::Deny(format!("Boundary Violation: {}", e));
                            }
                        }
                    }
                }
                Err(e) => {
                    return ShellDecision::Deny(format!(
                        "Expansion failed for token '{}': {}",
                        token, e
                    ))
                }
            }
        }
        ShellDecision::Allow
    }
}

pub struct ShellExecute {
    policy_engine: ShellPolicyEngine,
    tsid_generator: Arc<crate::libs::tsid::TsidGenerator>,
    planning_mode: bool,
    gateway: Option<Arc<dyn Gateway>>,
    session_id: Option<String>,
}

impl ShellExecute {
    pub fn new(
        path_guard: Arc<RwLock<PathGuard>>,
        tsid_generator: Arc<crate::libs::tsid::TsidGenerator>,
        custom_rules: Vec<ShellPolicyRule>,
        planning_mode: bool,
    ) -> Self {
        Self {
            policy_engine: ShellPolicyEngine::new(path_guard, custom_rules),
            tsid_generator,
            planning_mode,
            gateway: None,
            session_id: None,
        }
    }

    /// Sets the gateway for real-time output streaming
    pub fn with_gateway(mut self, gateway: Arc<dyn Gateway>, session_id: String) -> Self {
        self.gateway = Some(gateway);
        self.session_id = Some(session_id);
        self
    }
}

#[async_trait]
impl ToolDefinition for ShellExecute {
    fn name(&self) -> &str {
        crate::tools::TOOL_BASH
    }

    fn description(&self) -> &str {
        "Executes a shell command with an optional timeout.\n\n\
        IMPORTANT: This tool is for terminal operations like git, npm, docker, etc. DO NOT use it for file operations (reading, writing, editing, searching, finding files) - use the specialized tools for this instead.\n\n\
        Before executing the command, please follow these steps:\n\n\
        1. Directory Verification:\n\
           - If the command will create new directories or files, first verify the parent directory exists and is the correct location using the appropriate file-system tool or a safe terminal command\n\n\
        2. Command Execution:\n\
           - Always quote file paths that contain spaces with double quotes (e.g., cd \"path with spaces/file.txt\")\n\
           - Capture the output of the command.\n\n\
        Usage notes:\n\
          - The command argument is required.\n\
          - Commands run in the workflow's primary allowed root when available; shell state such as `cd` does not persist between tool calls.\n\
          - If you need a different working directory, include it in the command itself (for example: `cd \"path\" && npm test`).\n\
          - You can specify an optional timeout in milliseconds. Defaults to 120000ms and is capped at 600000ms.\n\
          - Large output is returned as a preview and saved to a temporary file that can be inspected with read_file or grep. Non-zero exits include stderr in the result."
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::System
    }

    fn scope(&self) -> crate::tools::ToolScope {
        crate::tools::ToolScope::Workflow
    }

    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The command to execute" },
                    "timeout": { "type": "number", "description": "Optional timeout in milliseconds. Defaults to 120000 and is capped at 600000." },
                    "description": { "type": "string", "description": "Clear description of what this command does." }
                },
                "required": ["command"]
            }),
            output_schema: None,
            disabled: false,
            scope: Some(self.scope()),
        }
    }

    async fn call(&self, params: Value) -> NativeToolResult {
        let command_str = params["command"]
            .as_str()
            .ok_or(ToolError::InvalidParams("command required".into()))?;

        // Defense-in-depth security check: Only enforce hard denials (system-critical commands).
        // This is a safety net to prevent catastrophic operations even if the workflow engine's
        // approval checks fail or are bypassed. Review-level checks are handled upstream by
        // the workflow engine's approval flow.
        match self.policy_engine.check(command_str, self.planning_mode) {
            ShellDecision::Deny(reason) => return Err(ToolError::Security(reason)),
            _ => {} // Allow and Review both proceed to execution
        }

        let timeout_ms = params["timeout"].as_u64().unwrap_or(120_000).min(600_000);
        let working_dir = self.default_working_dir();

        // Use streaming execution if gateway is configured
        if self.gateway.is_some() && self.session_id.is_some() {
            return self
                .call_with_streaming(command_str, timeout_ms, params.clone())
                .await;
        }

        if let Some(plan) = (!cfg!(target_os = "windows"))
            .then(|| parse_safe_compound_command(command_str))
            .flatten()
        {
            return self.call_safe_compound(plan, timeout_ms, working_dir).await;
        }

        // Fallback to standard execution
        let cmd_future = if cfg!(target_os = "windows") {
            let mut command = Command::new("cmd");
            command.args(["/C", command_str]);
            if let Some(dir) = &working_dir {
                command.current_dir(dir);
            }
            command.output()
        } else {
            let mut command = Command::new("sh");
            command.args(["-c", command_str]);
            if let Some(dir) = &working_dir {
                command.current_dir(dir);
            }
            command.output()
        };

        match timeout(Duration::from_millis(timeout_ms), cmd_future).await {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr);
                let exit_code = output.status.code().unwrap_or(-1);
                Ok(build_shell_tool_result(
                    command_str,
                    exit_code,
                    &stdout,
                    &stderr,
                ))
            }
            Ok(Err(e)) => Err(ToolError::ExecutionFailed(format!("Spawn failed: {}", e))),
            Err(_) => Err(ToolError::ExecutionFailed(format!(
                "Command timed out after {}ms",
                timeout_ms
            ))),
        }
    }
}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) {
    command.process_group(0);
}

#[cfg(not(unix))]
fn configure_process_group(_command: &mut Command) {}

struct StageProcessGuard {
    #[cfg(unix)]
    process_group_id: i32,
    active: bool,
}

impl StageProcessGuard {
    fn new(child: &Child) -> Self {
        Self {
            #[cfg(unix)]
            process_group_id: child.id().map_or(0, |id| id as i32),
            active: true,
        }
    }

    fn disarm(&mut self) {
        self.active = false;
    }

    fn kill_group(&mut self) {
        if !self.active {
            return;
        }
        #[cfg(unix)]
        if self.process_group_id > 0 {
            unsafe extern "C" {
                fn kill(pid: i32, signal: i32) -> i32;
            }
            const SIGKILL: i32 = 9;
            let _ = unsafe { kill(-self.process_group_id, SIGKILL) };
        }
        self.active = false;
    }
}

impl Drop for StageProcessGuard {
    fn drop(&mut self) {
        self.kill_group();
    }
}

async fn terminate_stage_process(child: &mut Child, process_guard: &mut StageProcessGuard) {
    process_guard.kill_group();
    #[cfg(not(unix))]
    {
        let _ = child.kill().await;
    }
    let _ = child.wait().await;
}

async fn run_stage_with_deadline(
    command_str: &str,
    cwd: &Path,
    deadline: Instant,
    timeout_ms: u64,
) -> Result<(i32, Vec<u8>, Vec<u8>), ToolError> {
    use std::process::Stdio;

    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(ToolError::ExecutionFailed(format!(
            "Command timed out after {timeout_ms}ms"
        )));
    }

    let mut command = Command::new("sh");
    command
        .args(["-c", command_str])
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    configure_process_group(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| ToolError::ExecutionFailed(format!("Failed to spawn stage: {error}")))?;
    let mut process_guard = StageProcessGuard::new(&child);
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| ToolError::ExecutionFailed("Failed to capture stage stdout".to_string()))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| ToolError::ExecutionFailed("Failed to capture stage stderr".to_string()))?;
    let stdout_task = tokio::spawn(async move {
        let mut output = Vec::new();
        stdout.read_to_end(&mut output).await.map(|_| output)
    });
    let stderr_task = tokio::spawn(async move {
        let mut output = Vec::new();
        stderr.read_to_end(&mut output).await.map(|_| output)
    });

    let status = match timeout(remaining, child.wait()).await {
        Ok(result) => result.map_err(|error| {
            ToolError::ExecutionFailed(format!("Failed to wait for stage process: {error}"))
        })?,
        Err(_) => {
            terminate_stage_process(&mut child, &mut process_guard).await;
            stdout_task.abort();
            stderr_task.abort();
            return Err(ToolError::ExecutionFailed(format!(
                "Command timed out after {timeout_ms}ms"
            )));
        }
    };
    let remaining = deadline.saturating_duration_since(Instant::now());
    let readers = async {
        let stdout = stdout_task
            .await
            .map_err(|error| ToolError::ExecutionFailed(format!("stdout reader failed: {error}")))?
            .map_err(|error| {
                ToolError::ExecutionFailed(format!("Failed to read stdout: {error}"))
            })?;
        let stderr = stderr_task
            .await
            .map_err(|error| ToolError::ExecutionFailed(format!("stderr reader failed: {error}")))?
            .map_err(|error| {
                ToolError::ExecutionFailed(format!("Failed to read stderr: {error}"))
            })?;
        Ok::<_, ToolError>((stdout, stderr))
    };
    let (stdout, stderr) = match timeout(remaining, readers).await {
        Ok(result) => result?,
        Err(_) => {
            process_guard.kill_group();
            return Err(ToolError::ExecutionFailed(format!(
                "Command timed out after {timeout_ms}ms"
            )));
        }
    };
    process_guard.disarm();
    Ok((status.code().unwrap_or(-1), stdout, stderr))
}

async fn send_tool_stream(gateway: &dyn Gateway, session_id: &str, tool_id: &str, output: &str) {
    let _ = gateway
        .send(
            session_id,
            GatewayPayload::ToolStream {
                tool_id: tool_id.to_string(),
                output: output.to_string(),
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
            },
        )
        .await;
}

fn format_tool_stream_output(
    last_stream_name: Option<&str>,
    stream_name: &'static str,
    line: &str,
) -> String {
    if last_stream_name == Some(stream_name)
        || (last_stream_name.is_none() && stream_name == "stdout")
    {
        return line.to_string();
    }

    format!("\n{stream_name}:\n{line}")
}

fn node_build_stderr_stream_name(exit_code: i32) -> &'static str {
    if exit_code == 0 {
        "stdout"
    } else {
        "stderr"
    }
}

fn format_compound_raw_section(
    index: usize,
    command: &str,
    cwd: &str,
    exit_code: i32,
    stdout: &str,
    stderr: &str,
) -> String {
    format!(
        "===== Stage {index} =====\ncommand: {command}\ncwd: {cwd}\nexit_code: {exit_code}\n\nstdout:\n{stdout}\n\nstderr:\n{stderr}\n\n"
    )
}

impl ShellExecute {
    async fn call_safe_compound(
        &self,
        plan: SafeCompoundCommand,
        timeout_ms: u64,
        working_dir: Option<PathBuf>,
    ) -> NativeToolResult {
        let mut cwd = match working_dir {
            Some(path) => path,
            None => std::env::current_dir().map_err(|error| {
                ToolError::ExecutionFailed(format!("Failed to resolve working directory: {error}"))
            })?,
        };

        let validated_navigation = match plan.stages.first() {
            Some(SafeCompoundStage::Navigation { target, .. }) => {
                let requested = PathBuf::from(target);
                let requested = if requested.is_absolute() {
                    requested
                } else {
                    cwd.join(requested)
                };
                let validated = self
                    .policy_engine
                    .path_guard
                    .read()
                    .map_err(|_| {
                        ToolError::ExecutionFailed("Path guard lock is poisoned".to_string())
                    })?
                    .validate(&requested, self.planning_mode, false, false)
                    .map_err(|error| ToolError::Security(error.to_string()))?;
                Some(validated)
            }
            _ => None,
        };

        let mut writer = ToolOutputWriter::create().map_err(|error| {
            ToolError::ExecutionFailed(format!(
                "Failed to create compound command output file: {error}"
            ))
        })?;
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let mut records = Vec::with_capacity(plan.stages.len());
        let mut final_exit_code = 0;
        let mut short_circuited = false;

        for (offset, stage) in plan.stages.into_iter().enumerate() {
            let index = offset + 1;
            let stage_cwd = cwd.to_string_lossy().to_string();
            let original = match &stage {
                SafeCompoundStage::Navigation { original, .. }
                | SafeCompoundStage::Command { original, .. } => original.clone(),
            };

            if short_circuited {
                records.push(CompoundShellStageResult {
                    index,
                    command: original,
                    cwd: stage_cwd,
                    executed: false,
                    exit_code: None,
                    skip_reason: Some("previous stage failed".to_string()),
                    output: None,
                });
                continue;
            }

            match stage {
                SafeCompoundStage::Navigation { .. } => {
                    let target = validated_navigation
                        .as_ref()
                        .ok_or_else(|| {
                            ToolError::ExecutionFailed(
                                "Validated navigation target is missing".to_string(),
                            )
                        })?
                        .clone();
                    let (exit_code, stderr) = if target.is_dir() {
                        cwd = target;
                        (0, String::new())
                    } else {
                        (
                            1,
                            format!("sh: cd: {}: Not a directory\n", target.display()),
                        )
                    };
                    let output = prepare_shell_output(&original, exit_code, "", &stderr);
                    writer
                        .append(&format_compound_raw_section(
                            index,
                            &original,
                            &stage_cwd,
                            exit_code,
                            "",
                            &stderr,
                        ))
                        .map_err(|error| {
                            ToolError::ExecutionFailed(format!(
                                "Failed to persist compound command output after execution started: {error}"
                            ))
                        })?;
                    records.push(CompoundShellStageResult {
                        index,
                        command: original,
                        cwd: stage_cwd,
                        executed: true,
                        exit_code: Some(exit_code),
                        skip_reason: None,
                        output: Some(output),
                    });
                    final_exit_code = exit_code;
                    short_circuited = exit_code != 0;
                }
                SafeCompoundStage::Command { original, .. } => {
                    let (exit_code, stdout, stderr) =
                        run_stage_with_deadline(&original, &cwd, deadline, timeout_ms).await?;
                    let stdout = String::from_utf8_lossy(&stdout).to_string();
                    let stderr = String::from_utf8_lossy(&stderr).to_string();
                    let mut prepared = prepare_shell_output(&original, exit_code, &stdout, &stderr);
                    writer
                        .append(&format_compound_raw_section(
                            index,
                            &original,
                            &stage_cwd,
                            exit_code,
                            &stdout,
                            &stderr,
                        ))
                        .map_err(|error| {
                            ToolError::ExecutionFailed(format!(
                                "Failed to persist compound command output after execution started: {error}"
                            ))
                        })?;
                    prepared.raw_content.clear();
                    records.push(CompoundShellStageResult {
                        index,
                        command: original,
                        cwd: stage_cwd,
                        executed: true,
                        exit_code: Some(exit_code),
                        skip_reason: None,
                        output: Some(prepared),
                    });
                    final_exit_code = exit_code;
                    short_circuited = exit_code != 0;
                }
            }
        }

        let persisted = writer.finalize().map_err(|error| {
            ToolError::ExecutionFailed(format!(
                "Failed to finalize compound command output after execution started: {error}"
            ))
        })?;
        Ok(build_compound_shell_tool_result(
            final_exit_code,
            &records,
            persisted,
        ))
    }

    async fn call_safe_compound_streaming(
        &self,
        plan: SafeCompoundCommand,
        timeout_ms: u64,
        working_dir: Option<PathBuf>,
        gateway: &dyn Gateway,
        session_id: &str,
        tool_id: &str,
    ) -> NativeToolResult {
        let mut cwd = match working_dir {
            Some(path) => path,
            None => std::env::current_dir().map_err(|error| {
                ToolError::ExecutionFailed(format!("Failed to resolve working directory: {error}"))
            })?,
        };
        let validated_navigation = match plan.stages.first() {
            Some(SafeCompoundStage::Navigation { target, .. }) => {
                let requested = PathBuf::from(target);
                let requested = if requested.is_absolute() {
                    requested
                } else {
                    cwd.join(requested)
                };
                Some(
                    self.policy_engine
                        .path_guard
                        .read()
                        .map_err(|_| {
                            ToolError::ExecutionFailed("Path guard lock is poisoned".to_string())
                        })?
                        .validate(&requested, self.planning_mode, false, false)
                        .map_err(|error| ToolError::Security(error.to_string()))?,
                )
            }
            _ => None,
        };
        let mut writer = ToolOutputWriter::create().map_err(|error| {
            ToolError::ExecutionFailed(format!(
                "Failed to create compound command output file: {error}"
            ))
        })?;
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let mut records = Vec::with_capacity(plan.stages.len());
        let mut final_exit_code = 0;
        let mut short_circuited = false;
        let mut has_streamed_output = false;

        for (offset, stage) in plan.stages.into_iter().enumerate() {
            let index = offset + 1;
            let stage_cwd = cwd.to_string_lossy().to_string();
            let original = match &stage {
                SafeCompoundStage::Navigation { original, .. }
                | SafeCompoundStage::Command { original, .. } => original.clone(),
            };
            if short_circuited {
                records.push(CompoundShellStageResult {
                    index,
                    command: original,
                    cwd: stage_cwd,
                    executed: false,
                    exit_code: None,
                    skip_reason: Some("previous stage failed".to_string()),
                    output: None,
                });
                continue;
            }

            send_tool_stream(
                gateway,
                session_id,
                tool_id,
                &format!(
                    "{}Stage {index}: {original}",
                    if has_streamed_output { "\n" } else { "" }
                ),
            )
            .await;
            has_streamed_output = true;

            match stage {
                SafeCompoundStage::Navigation { .. } => {
                    let target = validated_navigation
                        .as_ref()
                        .ok_or_else(|| {
                            ToolError::ExecutionFailed(
                                "Validated navigation target is missing".to_string(),
                            )
                        })?
                        .clone();
                    let (exit_code, stderr) = if target.is_dir() {
                        cwd = target;
                        (0, String::new())
                    } else {
                        (
                            1,
                            format!("sh: cd: {}: Not a directory\n", target.display()),
                        )
                    };
                    if !stderr.is_empty() {
                        send_tool_stream(
                            gateway,
                            session_id,
                            tool_id,
                            &format!("\nstderr:\n{stderr}"),
                        )
                        .await;
                    }
                    let mut prepared = prepare_shell_output(&original, exit_code, "", &stderr);
                    writer
                        .append(&format_compound_raw_section(
                            index,
                            &original,
                            &stage_cwd,
                            exit_code,
                            "",
                            &stderr,
                        ))
                        .map_err(|error| {
                            ToolError::ExecutionFailed(format!(
                                "Failed to persist compound command output after execution started: {error}"
                            ))
                        })?;
                    prepared.raw_content.clear();
                    records.push(CompoundShellStageResult {
                        index,
                        command: original,
                        cwd: stage_cwd,
                        executed: true,
                        exit_code: Some(exit_code),
                        skip_reason: None,
                        output: Some(prepared),
                    });
                    final_exit_code = exit_code;
                    short_circuited = exit_code != 0;
                }
                SafeCompoundStage::Command { original, .. } => {
                    let (exit_code, stdout, stderr) = self
                        .stream_safe_stage(
                            &original, &cwd, deadline, timeout_ms, gateway, session_id, tool_id,
                        )
                        .await?;
                    let mut prepared = prepare_shell_output(&original, exit_code, &stdout, &stderr);
                    writer
                        .append(&format_compound_raw_section(
                            index,
                            &original,
                            &stage_cwd,
                            exit_code,
                            &stdout,
                            &stderr,
                        ))
                        .map_err(|error| {
                            ToolError::ExecutionFailed(format!(
                                "Failed to persist compound command output after execution started: {error}"
                            ))
                        })?;
                    prepared.raw_content.clear();
                    records.push(CompoundShellStageResult {
                        index,
                        command: original,
                        cwd: stage_cwd,
                        executed: true,
                        exit_code: Some(exit_code),
                        skip_reason: None,
                        output: Some(prepared),
                    });
                    final_exit_code = exit_code;
                    short_circuited = exit_code != 0;
                }
            }
        }

        send_tool_stream(
            gateway,
            session_id,
            tool_id,
            &format!("\nExit code: {final_exit_code}"),
        )
        .await;
        let persisted = writer.finalize().map_err(|error| {
            ToolError::ExecutionFailed(format!(
                "Failed to finalize compound command output after execution started: {error}"
            ))
        })?;
        Ok(build_compound_shell_tool_result(
            final_exit_code,
            &records,
            persisted,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    async fn stream_safe_stage(
        &self,
        command_str: &str,
        cwd: &Path,
        deadline: Instant,
        timeout_ms: u64,
        gateway: &dyn Gateway,
        session_id: &str,
        tool_id: &str,
    ) -> Result<(i32, String, String), ToolError> {
        use std::process::Stdio;

        let mut command = Command::new("sh");
        command
            .args(["-c", command_str])
            .current_dir(cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        configure_process_group(&mut command);
        let mut child = command.spawn().map_err(|error| {
            ToolError::ExecutionFailed(format!("Failed to spawn stage: {error}"))
        })?;
        let mut process_guard = StageProcessGuard::new(&child);
        let stdout = child.stdout.take().ok_or_else(|| {
            ToolError::ExecutionFailed("Failed to capture stage stdout".to_string())
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            ToolError::ExecutionFailed("Failed to capture stage stderr".to_string())
        })?;
        let mut stdout_reader = BufReader::new(stdout).lines();
        let mut stderr_reader = BufReader::new(stderr).lines();
        let mut full_stdout = String::new();
        let mut full_stderr = String::new();
        let mut pending_node_build_stderr = String::new();
        let mut stdout_sanitizer = AnsiOutputSanitizer::default();
        let mut stderr_sanitizer = AnsiOutputSanitizer::default();
        let buffers_node_build_stderr =
            is_node_build_command(&crate::tools::shell_output::normalize_command(command_str));
        let mut stdout_eof = false;
        let mut stderr_eof = false;
        let mut last_stream_name: Option<&'static str> = None;

        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                terminate_stage_process(&mut child, &mut process_guard).await;
                return Err(ToolError::ExecutionFailed(format!(
                    "Command timed out after {timeout_ms}ms"
                )));
            }
            if stdout_eof && stderr_eof {
                let status = match timeout(remaining, child.wait()).await {
                    Ok(result) => result.map_err(|error| {
                        ToolError::ExecutionFailed(format!(
                            "Failed to wait for stage process: {error}"
                        ))
                    })?,
                    Err(_) => {
                        terminate_stage_process(&mut child, &mut process_guard).await;
                        return Err(ToolError::ExecutionFailed(format!(
                            "Command timed out after {timeout_ms}ms"
                        )));
                    }
                };
                let exit_code = status.code().unwrap_or(-1);
                if !pending_node_build_stderr.is_empty() {
                    let stream_name = node_build_stderr_stream_name(exit_code);
                    send_tool_stream(
                        gateway,
                        session_id,
                        tool_id,
                        &format_tool_stream_output(
                            last_stream_name,
                            stream_name,
                            pending_node_build_stderr.trim_end(),
                        ),
                    )
                    .await;
                }
                process_guard.disarm();
                return Ok((exit_code, full_stdout, full_stderr));
            }

            tokio::select! {
                line = stdout_reader.next_line(), if !stdout_eof => {
                    match line {
                        Ok(Some(line)) => {
                            let raw_line = format!("{line}\n");
                            full_stdout.push_str(&raw_line);
                            let line = stdout_sanitizer.sanitize(&raw_line);
                            if !line.is_empty() {
                                send_tool_stream(
                                    gateway,
                                    session_id,
                                    tool_id,
                                    &format_tool_stream_output(last_stream_name, "stdout", line.trim_end_matches('\n')),
                                ).await;
                                last_stream_name = Some("stdout");
                            }
                        }
                        Ok(None) => stdout_eof = true,
                        Err(error) => {
                            terminate_stage_process(&mut child, &mut process_guard).await;
                            return Err(ToolError::ExecutionFailed(format!(
                                "Failed to read stage stdout: {error}"
                            )));
                        }
                    }
                }
                line = stderr_reader.next_line(), if !stderr_eof => {
                    match line {
                        Ok(Some(line)) => {
                            let raw_line = format!("{line}\n");
                            full_stderr.push_str(&raw_line);
                            let line = stderr_sanitizer.sanitize(&raw_line);
                            if !line.is_empty() {
                                if should_collect_stderr_line_as_stdout(command_str, &line) {
                                    send_tool_stream(
                                        gateway,
                                        session_id,
                                        tool_id,
                                        &format_tool_stream_output(last_stream_name, "stdout", line.trim_end_matches('\n')),
                                    ).await;
                                    last_stream_name = Some("stdout");
                                } else if buffers_node_build_stderr {
                                    pending_node_build_stderr.push_str(&line);
                                } else {
                                    send_tool_stream(
                                        gateway,
                                        session_id,
                                        tool_id,
                                        &format_tool_stream_output(last_stream_name, "stderr", line.trim_end_matches('\n')),
                                    ).await;
                                    last_stream_name = Some("stderr");
                                }
                            }
                        }
                        Ok(None) => stderr_eof = true,
                        Err(error) => {
                            terminate_stage_process(&mut child, &mut process_guard).await;
                            return Err(ToolError::ExecutionFailed(format!(
                                "Failed to read stage stderr: {error}"
                            )));
                        }
                    }
                }
                _ = tokio::time::sleep(remaining.min(Duration::from_millis(100))) => {}
            }
        }
    }

    fn default_working_dir(&self) -> Option<std::path::PathBuf> {
        self.policy_engine
            .path_guard
            .read()
            .ok()
            .and_then(|guard| guard.get_primary_root().map(|path| path.to_path_buf()))
    }

    /// Execute command with real-time output streaming to frontend
    async fn call_with_streaming(
        &self,
        command_str: &str,
        timeout_ms: u64,
        params: Value,
    ) -> NativeToolResult {
        use std::process::Stdio;

        let gateway = self.gateway.as_ref().ok_or(ToolError::ExecutionFailed(
            "Gateway not configured for streaming".to_string(),
        ))?;
        let session_id = self.session_id.as_ref().ok_or(ToolError::ExecutionFailed(
            "Session ID not configured for streaming".to_string(),
        ))?;

        // Use tool_call_id from params (injected by workflow engine) or generate one
        let tool_id = params
            .get(crate::constants::INTERNAL_PARAM_TOOL_CALL_ID)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                format!(
                    "bash_{}",
                    self.tsid_generator.generate().unwrap_or_default()
                )
            });
        let working_dir = self.default_working_dir();

        if let Some(plan) = (!cfg!(target_os = "windows"))
            .then(|| parse_safe_compound_command(command_str))
            .flatten()
        {
            return self
                .call_safe_compound_streaming(
                    plan,
                    timeout_ms,
                    working_dir,
                    gateway.as_ref(),
                    session_id,
                    &tool_id,
                )
                .await;
        }

        let mut child = if cfg!(target_os = "windows") {
            let mut command = Command::new("cmd");
            command
                .args(["/C", command_str])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            if let Some(dir) = &working_dir {
                command.current_dir(dir);
            }
            command
                .spawn()
                .map_err(|e| ToolError::ExecutionFailed(format!("Failed to spawn: {}", e)))?
        } else {
            let mut command = Command::new("sh");
            command
                .args(["-c", command_str])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            if let Some(dir) = &working_dir {
                command.current_dir(dir);
            }
            command
                .spawn()
                .map_err(|e| ToolError::ExecutionFailed(format!("Failed to spawn: {}", e)))?
        };

        let stdout = child.stdout.take().ok_or(ToolError::ExecutionFailed(
            "Failed to capture stdout".to_string(),
        ))?;
        let stderr = child.stderr.take().ok_or(ToolError::ExecutionFailed(
            "Failed to capture stderr".to_string(),
        ))?;

        let mut stdout_reader = BufReader::new(stdout).lines();
        let mut stderr_reader = BufReader::new(stderr).lines();

        let mut full_stdout = String::new();
        let mut full_stderr = String::new();
        let mut pending_node_build_stderr = String::new();
        let mut stdout_sanitizer = AnsiOutputSanitizer::default();
        let mut stderr_sanitizer = AnsiOutputSanitizer::default();
        let buffers_node_build_stderr =
            is_node_build_command(&crate::tools::shell_output::normalize_command(command_str));
        let mut stdout_eof = false;
        let mut stderr_eof = false;
        let mut last_stream_name: Option<&'static str> = None;

        // Read stdout and stderr concurrently with timeout
        let start_time = std::time::Instant::now();

        loop {
            let timeout_remaining =
                timeout_ms.saturating_sub(start_time.elapsed().as_millis() as u64);
            if timeout_remaining == 0 {
                let _ = child.kill().await;
                return Err(ToolError::ExecutionFailed(format!(
                    "Command timed out after {}ms",
                    timeout_ms
                )));
            }

            // Check if both streams reached EOF and process has exited
            if stdout_eof && stderr_eof {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        // Process has exited
                        let exit_code = status.code().unwrap_or(-1);
                        if !pending_node_build_stderr.is_empty() {
                            let stream_name = node_build_stderr_stream_name(exit_code);
                            let _ = gateway
                                .send(
                                    session_id,
                                    GatewayPayload::ToolStream {
                                        tool_id: tool_id.clone(),
                                        output: format_tool_stream_output(
                                            last_stream_name,
                                            stream_name,
                                            pending_node_build_stderr.trim_end(),
                                        ),
                                        timestamp: std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_millis()
                                            as u64,
                                    },
                                )
                                .await;
                            last_stream_name = Some(stream_name);
                        }
                        let _ = gateway
                            .send(
                                session_id,
                                GatewayPayload::ToolStream {
                                    tool_id: tool_id.clone(),
                                    output: if last_stream_name.is_some() {
                                        format!("\nExit code: {}", exit_code)
                                    } else {
                                        format!("Exit code: {}", exit_code)
                                    },
                                    timestamp: std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_millis()
                                        as u64,
                                },
                            )
                            .await;
                        return Ok(build_shell_tool_result(
                            command_str,
                            exit_code,
                            &full_stdout,
                            &full_stderr,
                        ));
                    }
                    Ok(None) => {
                        // Both streams EOF but process still running - wait a bit
                        tokio::time::sleep(Duration::from_millis(10)).await;
                        continue;
                    }
                    Err(e) => {
                        return Err(ToolError::ExecutionFailed(format!(
                            "Failed to check process status: {}",
                            e
                        )));
                    }
                }
            }

            // Try to read from both stdout and stderr with a small timeout
            let mut got_output = false;

            // Use tokio::select! to read from either stream
            tokio::select! {
                line = stdout_reader.next_line(), if !stdout_eof => {
                    match line {
                        Ok(Some(l)) => {
                            let l = stdout_sanitizer.sanitize(&format!("{l}\n"));
                            if l.is_empty() {
                                continue;
                            }
                            full_stdout.push_str(&l);

                            // Send real-time streaming output to frontend
                            let _ = gateway.send(
                                session_id,
                                GatewayPayload::ToolStream {
                                    tool_id: tool_id.clone(),
                                    output: format_tool_stream_output(
                                        last_stream_name,
                                        "stdout",
                                        l.trim_end_matches('\n'),
                                    ),
                                    timestamp: std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_millis() as u64,
                                },
                            ).await;
                            last_stream_name = Some("stdout");
                            got_output = true;
                        }
                        Ok(None) => {
                            // EOF reached for stdout
                            stdout_eof = true;
                        }
                        Err(e) => {
                            log::warn!("Error reading stdout: {}", e);
                        }
                    }
                }
                line = stderr_reader.next_line(), if !stderr_eof => {
                    match line {
                        Ok(Some(l)) => {
                            let l = stderr_sanitizer.sanitize(&format!("{l}\n"));
                            if l.is_empty() {
                                continue;
                            }
                            let timestamp = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis() as u64;

                            let collect_as_stdout =
                                should_collect_stderr_line_as_stdout(command_str, &l);

                            if collect_as_stdout {
                                full_stdout.push_str(&l);

                                let _ = gateway.send(
                                    session_id,
                                    GatewayPayload::ToolStream {
                                        tool_id: tool_id.clone(),
                                        output: format_tool_stream_output(
                                            last_stream_name,
                                            "stdout",
                                            l.trim_end_matches('\n'),
                                        ),
                                        timestamp,
                                    },
                                ).await;
                                last_stream_name = Some("stdout");
                            } else {
                                full_stderr.push_str(&l);

                                if buffers_node_build_stderr {
                                    pending_node_build_stderr.push_str(&l);
                                } else {
                                    let _ = gateway.send(
                                        session_id,
                                        GatewayPayload::ToolStream {
                                            tool_id: tool_id.clone(),
                                            output: format_tool_stream_output(
                                                last_stream_name,
                                                "stderr",
                                                l.trim_end_matches('\n'),
                                            ),
                                            timestamp,
                                        },
                                    ).await;
                                    last_stream_name = Some("stderr");
                                }
                            }
                            got_output = true;
                        }
                        Ok(None) => {
                            // EOF reached for stderr
                            stderr_eof = true;
                        }
                        Err(e) => {
                            log::warn!("Error reading stderr: {}", e);
                        }
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    // Check if process has exited
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            // Process has exited
                            let exit_code = status.code().unwrap_or(-1);
                            if !pending_node_build_stderr.is_empty() {
                                let stream_name = node_build_stderr_stream_name(exit_code);
                                let _ = gateway
                                    .send(
                                        session_id,
                                        GatewayPayload::ToolStream {
                                            tool_id: tool_id.clone(),
                                            output: format_tool_stream_output(
                                                last_stream_name,
                                                stream_name,
                                                pending_node_build_stderr.trim_end(),
                                            ),
                                            timestamp: std::time::SystemTime::now()
                                                .duration_since(std::time::UNIX_EPOCH)
                                                .unwrap_or_default()
                                                .as_millis()
                                                as u64,
                                        },
                                    )
                                    .await;
                                last_stream_name = Some(stream_name);
                            }
                            let _ = gateway
                                .send(
                                    session_id,
                                    GatewayPayload::ToolStream {
                                        tool_id: tool_id.clone(),
                                        output: if last_stream_name.is_some() {
                                            format!("\nExit code: {}", exit_code)
                                        } else {
                                            format!("Exit code: {}", exit_code)
                                        },
                                        timestamp: std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_millis()
                                            as u64,
                                    },
                                )
                                .await;
                            return Ok(build_shell_tool_result(
                                command_str,
                                exit_code,
                                &full_stdout,
                                &full_stderr,
                            ));
                        }
                        Ok(None) => {
                            // Process still running, continue
                        }
                        Err(e) => {
                            return Err(ToolError::ExecutionFailed(format!(
                                "Failed to check process status: {}",
                                e
                            )));
                        }
                    }
                }
            }

            // Small yield to prevent busy loop
            if !got_output {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::shell_output::{strip_ansi_escape_sequences, AnsiOutputSanitizer};
    use crate::workflow::react::security::PathGuard;
    use std::sync::Mutex;
    use tempfile::tempdir;

    #[derive(Default)]
    struct RecordingGateway {
        streams: Mutex<Vec<(String, String)>>,
    }

    #[async_trait]
    impl Gateway for RecordingGateway {
        async fn send(
            &self,
            _session_id: &str,
            payload: GatewayPayload,
        ) -> Result<(), WorkflowEngineError> {
            if let GatewayPayload::ToolStream {
                tool_id, output, ..
            } = payload
            {
                self.streams.lock().unwrap().push((tool_id, output));
            }
            Ok(())
        }

        async fn inject_input(
            &self,
            _session_id: &str,
            _input: String,
        ) -> Result<(), WorkflowEngineError> {
            Ok(())
        }
    }

    fn setup_test_context() -> (
        tempfile::TempDir,
        std::path::PathBuf,
        Arc<RwLock<PathGuard>>,
    ) {
        let root = tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();
        // Updated to use three-argument constructor for PathGuard
        let guard = Arc::new(RwLock::new(PathGuard::new(
            vec![root_path.clone(), std::env::current_dir().unwrap()],
            vec![],
            vec![],
        )));
        (root, root_path, guard)
    }

    #[test]
    fn streaming_sanitizers_remove_cross_line_ansi_strings() {
        let mut stdout_sanitizer = AnsiOutputSanitizer::default();
        assert_eq!(stdout_sanitizer.sanitize("\u{1b}]0;secret\n"), "");
        assert_eq!(
            stdout_sanitizer.sanitize("payload\u{1b}\\visible\n"),
            "visible\n"
        );

        let mut stderr_sanitizer = AnsiOutputSanitizer::default();
        assert_eq!(stderr_sanitizer.sanitize("\u{0090}secret\u{7}\n"), "");
        assert_eq!(
            stderr_sanitizer.sanitize("payload\u{009C}visible\n"),
            "visible\n"
        );

        let mut c1_osc_sanitizer = AnsiOutputSanitizer::default();
        assert_eq!(c1_osc_sanitizer.sanitize("\u{009D}secret\n"), "");
        assert_eq!(
            c1_osc_sanitizer.sanitize("payload\u{009C}visible\n"),
            "visible\n"
        );
    }

    #[test]
    fn streaming_payload_preserves_text_after_c1_ansi_terminator() {
        let output = strip_ansi_escape_sequences("\u{009D}title\u{009C}visible");

        assert_eq!(
            format_tool_stream_output(None, "stdout", &output),
            "visible"
        );
    }

    #[test]
    fn node_build_command_detection_is_stage_local() {
        assert!(!is_node_build_command(
            &crate::tools::shell_output::normalize_command("cd app; pnpm build")
        ));
        assert!(is_node_build_command(
            &crate::tools::shell_output::normalize_command("CI=1 pnpm build")
        ));
        assert!(is_node_build_command(
            &crate::tools::shell_output::normalize_command(
                "BUILD_LABEL=\"release candidate\" pnpm build"
            )
        ));
        assert!(!is_node_build_command(
            &crate::tools::shell_output::normalize_command(
                "cd app; BUILD_LABEL=\"release candidate\" pnpm build"
            )
        ));
    }

    #[test]
    fn node_build_stderr_stream_name_depends_on_exit_code() {
        assert_eq!(node_build_stderr_stream_name(0), "stdout");
        assert_eq!(node_build_stderr_stream_name(1), "stderr");
        assert_eq!(
            format_tool_stream_output(
                Some("stdout"),
                node_build_stderr_stream_name(1),
                "error: failed to build"
            ),
            "\nstderr:\nerror: failed to build"
        );
    }

    #[test]
    fn node_build_stderr_stream_payload_uses_stdout_label() {
        let warning = "(!) Some chunks are larger than 500 kB after minification.";

        assert_eq!(
            format_tool_stream_output(Some("stderr"), "stdout", warning),
            format!("\nstdout:\n{warning}")
        );
        assert_eq!(format_tool_stream_output(None, "stdout", warning), warning);
    }

    #[test]
    fn test_policy_engine_basic() {
        let (_root, root_path, guard) = setup_test_context();
        let engine = ShellPolicyEngine::new(guard, vec![]);
        assert_eq!(engine.check("ls", false), ShellDecision::Allow);
        assert_eq!(
            engine.check(&format!("ls {}", root_path.display()), false),
            ShellDecision::Allow
        );
    }

    #[test]
    fn test_policy_engine_blocked_binaries() {
        let (_root, _, guard) = setup_test_context();
        let engine = ShellPolicyEngine::new(guard, vec![]);
        assert!(matches!(
            engine.check("sudo rm -rf /", false),
            ShellDecision::Deny(_)
        ));
        assert!(matches!(
            engine.check("rm -rf test", false),
            ShellDecision::Review(_)
        ));
    }

    #[test]
    fn test_policy_engine_root_protection() {
        let (_root, root_path, guard) = setup_test_context();
        let engine = ShellPolicyEngine::new(guard, vec![]);

        // 1. Attempt to remove the root directory itself (Absolute path)
        let cmd_root = format!("rm -rf {}", root_path.display());
        assert!(matches!(
            engine.check(&cmd_root, false),
            ShellDecision::Deny(_)
        ));

        // 2. Attempt to remove the root via "." or "./"
        assert!(matches!(
            engine.check("rm -rf .", false),
            ShellDecision::Deny(_)
        ));
        assert!(matches!(
            engine.check("rm -rf ./", false),
            ShellDecision::Deny(_)
        ));

        // 3. Attempt to move the root
        let cmd_mv = format!("mv {} /tmp/moved_root", root_path.display());
        assert!(matches!(
            engine.check(&cmd_mv, false),
            ShellDecision::Deny(_)
        ));

        // 4. Workspace file deletion is blocked by PathGuard.
        let cmd_file = format!("rm {}", root_path.join("file.txt").display());
        assert!(matches!(
            engine.check(&cmd_file, false),
            ShellDecision::Deny(_)
        ));
    }

    #[test]
    fn test_policy_engine_git_diff_multiple_paths() {
        // Test case for git diff with multiple file path arguments
        // This simulates: git diff broadcast/src/common/account_manager.rs broadcast/src/main.rs broadcast/src/server.rs
        // with base directory /Volumes/dev/personal/dev/rust/rsctp

        // Use the actual authorized directory
        let authorized_root = std::path::PathBuf::from("/Volumes/dev/personal/dev/rust/rsctp");
        let current_dir = std::env::current_dir().unwrap();
        let guard = Arc::new(RwLock::new(PathGuard::new(
            vec![authorized_root.clone(), current_dir.clone()],
            vec![],
            vec![],
        )));
        let engine = ShellPolicyEngine::new(guard, vec![]);

        // Test git diff command with multiple RELATIVE paths (as the user would use it)
        let cmd_relative = "git diff broadcast/src/common/account_manager.rs broadcast/src/main.rs broadcast/src/server.rs";
        let result_relative = engine.check(cmd_relative, false);

        println!("Git diff relative command: {}", cmd_relative);
        println!("Authorized root: {:?}", authorized_root);
        println!("Current working dir: {:?}", current_dir);
        println!("Result: {:?}", result_relative);

        // Should NOT be Deny - git diff with relative paths should be allowed or reviewed
        assert!(!matches!(result_relative, ShellDecision::Deny(_)));

        // Test with absolute paths pointing to the authorized directory
        let file1 = authorized_root.join("broadcast/src/common/account_manager.rs");
        let file2 = authorized_root.join("broadcast/src/main.rs");
        let file3 = authorized_root.join("broadcast/src/server.rs");

        let cmd_absolute = format!(
            "git diff {} {} {}",
            file1.display(),
            file2.display(),
            file3.display()
        );
        let result_absolute = engine.check(&cmd_absolute, false);

        println!("Git diff absolute command: {}", cmd_absolute);
        println!("Result: {:?}", result_absolute);

        // Should NOT be Deny
        assert!(!matches!(result_absolute, ShellDecision::Deny(_)));
    }

    #[test]
    fn test_policy_engine_relative_path_with_different_cwd() {
        // This test simulates the actual issue:
        // - Authorized root: /Volumes/dev/personal/dev/rust/rsctp
        // - Shell CWD (process working directory): /Volumes/dev/personal/dev/rust/rsctp
        // - AI passes relative paths like "broadcast/src/common/account_manager.rs"
        // - PathGuard should validate these paths correctly

        // Create a temporary directory to simulate the rsctp project
        let temp_root = tempdir().unwrap();
        let project_root = temp_root.path().canonicalize().unwrap();

        // Create the directory structure
        let broadcast_dir = project_root.join("broadcast/src/common");
        std::fs::create_dir_all(&broadcast_dir).unwrap();
        let file1 = broadcast_dir.join("account_manager.rs");
        let file2 = project_root.join("broadcast/src/main.rs");
        let file3 = project_root.join("broadcast/src/server.rs");
        std::fs::create_dir_all(file2.parent().unwrap()).unwrap();
        std::fs::create_dir_all(file3.parent().unwrap()).unwrap();
        std::fs::write(&file1, "// test").unwrap();
        std::fs::write(&file2, "// test").unwrap();
        std::fs::write(&file3, "// test").unwrap();

        // Set up PathGuard with the project root as primary
        let guard = Arc::new(RwLock::new(PathGuard::new(
            vec![project_root.clone()],
            vec![],
            vec![],
        )));

        let engine = ShellPolicyEngine::new(guard, vec![]);

        // Simulate the command AI would send - relative paths
        let cmd = "git diff broadcast/src/common/account_manager.rs broadcast/src/main.rs broadcast/src/server.rs";
        let result = engine.check(cmd, false);

        println!("\n=== Relative Path Test ===");
        println!("Project root: {:?}", project_root);
        println!("Command: {}", cmd);
        println!("Result: {:?}", result);

        // The paths are relative and look like paths, so PathGuard should validate them
        // against the primary root. They should NOT be denied.
        match &result {
            ShellDecision::Deny(reason) => {
                panic!("Relative path was DENIED unexpectedly: {}", reason);
            }
            ShellDecision::Review(reason) => {
                println!("Review required (expected for git): {}", reason);
            }
            ShellDecision::Allow => {
                println!("Allowed");
            }
        }

        // Test with ls command on relative paths
        let cmd_ls = "ls broadcast/src/common broadcast/src";
        let result_ls = engine.check(cmd_ls, false);
        println!("\nls command: {}", cmd_ls);
        println!("Result: {:?}", result_ls);
        assert!(!matches!(result_ls, ShellDecision::Deny(_)));
    }

    #[test]
    fn test_policy_engine_relative_path_nonexistent_files() {
        // Test case: git diff with files that don't exist yet
        // This is common when reviewing changes before files are created

        let temp_root = tempdir().unwrap();
        let project_root = temp_root.path().canonicalize().unwrap();

        // DON'T create the files - they don't exist yet
        let guard = Arc::new(RwLock::new(PathGuard::new(
            vec![project_root.clone()],
            vec![],
            vec![],
        )));

        let engine = ShellPolicyEngine::new(guard, vec![]);

        // git diff on non-existent files (common scenario)
        let cmd = "git diff new_file.rs another_new_file.rs";
        let result = engine.check(cmd, false);

        println!("\n=== Non-existent Files Test ===");
        println!("Project root: {:?}", project_root);
        println!("Command: {}", cmd);
        println!("Result: {:?}", result);

        // Should NOT deny - these are valid relative paths within the workspace
        match &result {
            ShellDecision::Deny(reason) => {
                // This might be the actual issue!
                println!("ERROR: Command was DENIED: {}", reason);
            }
            ShellDecision::Review(reason) => {
                println!("Review required: {}", reason);
            }
            ShellDecision::Allow => {
                println!("Allowed");
            }
        }

        // Test git status (common command, should always work)
        let cmd_status = "git status";
        let result_status = engine.check(cmd_status, false);
        println!("\ngit status result: {:?}", result_status);
        assert!(!matches!(result_status, ShellDecision::Deny(_)));

        let cmd_status_short = "git status --short";
        let result_status_short = engine.check(cmd_status_short, false);
        println!("\ngit status --short result: {:?}", result_status_short);
        assert_eq!(result_status_short, ShellDecision::Allow);
    }

    #[test]
    fn test_policy_engine_custom_rule_allows_authorized_cd_prefix() {
        let temp_root = tempdir().unwrap();
        let project_root = temp_root.path().canonicalize().unwrap();

        let guard = Arc::new(RwLock::new(PathGuard::new(
            vec![project_root.clone()],
            vec![],
            vec![],
        )));
        let engine = ShellPolicyEngine::new(
            guard,
            vec![ShellPolicyRule {
                pattern: "^git diff($| .*)".to_string(),
                decision: ShellDecision::Allow,
                description: None,
            }],
        );

        let result = engine.check("cd . && git diff src/main.rs | head -80", false);

        assert_eq!(result, ShellDecision::Allow);
    }

    #[test]
    fn test_policy_engine_custom_rule_allows_benign_stream_merge_and_tail_filter() {
        let temp_root = tempdir().unwrap();
        let project_root = temp_root.path().canonicalize().unwrap();

        let guard = Arc::new(RwLock::new(PathGuard::new(
            vec![project_root.clone()],
            vec![],
            vec![],
        )));
        let engine = ShellPolicyEngine::new(
            guard,
            vec![ShellPolicyRule {
                pattern: "^cargo check($| .*)".to_string(),
                decision: ShellDecision::Allow,
                description: None,
            }],
        );

        let result = engine.check(
            "cd . && cargo check --manifest-path src-tauri/Cargo.toml 2>&1 | tail -10",
            false,
        );

        assert_eq!(result, ShellDecision::Allow);
    }

    #[test]
    fn test_policy_engine_custom_rule_denies_unauthorized_cd_prefix() {
        let temp_root = tempdir().unwrap();
        let project_root = temp_root.path().canonicalize().unwrap();
        let outside_root = tempdir().unwrap();

        let guard = Arc::new(RwLock::new(PathGuard::new(
            vec![project_root],
            vec![],
            vec![],
        )));
        let engine = ShellPolicyEngine::new(
            guard,
            vec![ShellPolicyRule {
                pattern: "^git diff($| .*)".to_string(),
                decision: ShellDecision::Allow,
                description: None,
            }],
        );

        let command = format!(
            "cd {} && git diff src/main.rs | head -80",
            outside_root.path().display()
        );
        let result = engine.check(&command, false);

        assert!(matches!(result, ShellDecision::Deny(_)));
    }

    #[test]
    fn test_shell_execute_uses_primary_root_as_default_working_dir() {
        let temp_root = tempdir().unwrap();
        let project_root = temp_root.path().canonicalize().unwrap();
        let nested_dir = project_root.join("nested");
        std::fs::create_dir_all(&nested_dir).unwrap();

        let guard = Arc::new(RwLock::new(PathGuard::new(
            vec![nested_dir.clone()],
            vec![],
            vec![],
        )));
        let shell = ShellExecute::new(
            guard,
            Arc::new(crate::libs::tsid::TsidGenerator::new(1).unwrap()),
            vec![],
            false,
        );

        assert_eq!(
            shell.default_working_dir().as_deref(),
            Some(nested_dir.as_path())
        );
    }

    #[test]
    fn test_policy_engine_path_token_validation() {
        // Test to understand how validate_path_token works with relative paths
        let temp_root = tempdir().unwrap();
        let project_root = temp_root.path().canonicalize().unwrap();

        // Create a subdirectory
        let subdir = project_root.join("broadcast/src/common");
        std::fs::create_dir_all(&subdir).unwrap();

        let guard = Arc::new(RwLock::new(PathGuard::new(
            vec![project_root.clone()],
            vec![],
            vec![],
        )));

        let engine = ShellPolicyEngine::new(guard.clone(), vec![]);

        // Test different path formats
        let test_cases = vec![
            ("broadcast/src/common/account_manager.rs", "relative path"),
            (
                "./broadcast/src/common/account_manager.rs",
                "relative with ./",
            ),
            ("file.txt", "simple filename"),
            ("./file.txt", "simple filename with ./"),
            ("src/../file.txt", "path with parent dir"),
        ];

        println!("\n=== Path Token Validation Test ===");
        println!("Project root: {:?}", project_root);

        for (path, desc) in test_cases {
            let decision = engine.validate_path_token(path, false, false, false);
            println!("\nPath: {} ({})", path, desc);
            println!("Decision: {:?}", decision);

            // All should be Allow or Review (for skill paths), never Deny
            if matches!(decision, ShellDecision::Deny(_)) {
                panic!(
                    "Path '{}' ({}) was unexpectedly denied: {:?}",
                    path, desc, decision
                );
            }
        }
    }

    fn test_shell_execute(project_root: PathBuf) -> ShellExecute {
        let guard = Arc::new(RwLock::new(PathGuard::new(
            vec![project_root],
            vec![],
            vec![],
        )));
        ShellExecute::new(
            guard,
            Arc::new(crate::libs::tsid::TsidGenerator::new(1).unwrap()),
            vec![],
            false,
        )
    }

    fn initialize_test_git_repository(project_root: &Path) {
        let status = std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(project_root)
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn safe_compound_execution_tracks_cwd_and_persists_all_raw_stages() {
        let temp_root = tempdir().unwrap();
        let project_root = temp_root.path().canonicalize().unwrap();
        let nested = project_root.join("nested");
        std::fs::create_dir(&nested).unwrap();
        initialize_test_git_repository(&nested);
        let shell = test_shell_execute(project_root);

        let result = shell
            .call(json!({
                "command": "cd nested && /bin/pwd && git status --short",
                "timeout": 10_000,
            }))
            .await
            .unwrap();
        let structured = result
            .structured_content
            .expect("structured content missing");

        assert_eq!(structured["exit_code"].as_i64(), Some(0));
        assert_eq!(structured["stages"].as_array().map(Vec::len), Some(3));
        assert_eq!(structured["stages"][0]["command"], "cd nested");
        assert_eq!(
            structured["stages"][1]["cwd"].as_str(),
            Some(nested.to_string_lossy().as_ref())
        );
        assert_eq!(structured["stages"][2]["executed"].as_bool(), Some(true));

        let model_path = structured["persisted_output"]["path"]
            .as_str()
            .expect("persisted path missing");
        let physical_path = crate::libs::ai_temp::resolve_ai_temp_path(Path::new(model_path));
        let raw = std::fs::read_to_string(&physical_path).unwrap();
        assert!(raw.contains("===== Stage 1 ====="));
        assert!(raw.contains("command: /bin/pwd"));
        assert!(raw.contains(nested.to_string_lossy().as_ref()));
        assert!(raw.contains("command: git status --short"));
        std::fs::remove_file(physical_path).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn safe_compound_execution_short_circuits_after_failure() {
        let temp_root = tempdir().unwrap();
        let project_root = temp_root.path().canonicalize().unwrap();
        initialize_test_git_repository(&project_root);
        let shell = test_shell_execute(project_root.clone());

        let result = shell
            .call(json!({
                "command": "/usr/bin/false && git status && /usr/bin/touch marker",
                "timeout": 10_000,
            }))
            .await
            .unwrap();
        let structured = result
            .structured_content
            .expect("structured content missing");

        assert_eq!(structured["exit_code"].as_i64(), Some(1));
        assert_eq!(structured["stages"][0]["executed"].as_bool(), Some(true));
        assert_eq!(structured["stages"][1]["executed"].as_bool(), Some(false));
        assert_eq!(structured["stages"][2]["executed"].as_bool(), Some(false));
        assert_eq!(
            structured["stages"][2]["skip_reason"].as_str(),
            Some("previous stage failed")
        );
        assert!(!project_root.join("marker").exists());

        let model_path = structured["persisted_output"]["path"]
            .as_str()
            .expect("persisted path missing");
        let physical_path = crate::libs::ai_temp::resolve_ai_temp_path(Path::new(model_path));
        let raw = std::fs::read_to_string(&physical_path).unwrap();
        assert!(raw.contains("command: /usr/bin/false"));
        assert!(!raw.contains("command: git status"));
        assert!(!raw.contains("command: /usr/bin/touch marker"));
        std::fs::remove_file(physical_path).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn safe_compound_streaming_uses_one_tool_id_and_short_circuits() {
        let temp_root = tempdir().unwrap();
        let project_root = temp_root.path().canonicalize().unwrap();
        initialize_test_git_repository(&project_root);
        let gateway = Arc::new(RecordingGateway::default());
        let shell = test_shell_execute(project_root.clone())
            .with_gateway(gateway.clone(), "test-session".to_string());

        let result = shell
            .call(json!({
                "command": "/bin/echo first && /usr/bin/false && git status --short",
                "timeout": 10_000,
                crate::constants::INTERNAL_PARAM_TOOL_CALL_ID: "tool-safe-compound",
            }))
            .await
            .unwrap();
        let structured = result
            .structured_content
            .expect("structured content missing");
        assert_eq!(structured["exit_code"].as_i64(), Some(1));
        assert_eq!(structured["stages"][2]["executed"].as_bool(), Some(false));

        let streams = gateway.streams.lock().unwrap().clone();
        assert!(!streams.is_empty());
        assert!(streams
            .iter()
            .all(|(tool_id, _)| tool_id == "tool-safe-compound"));
        let combined = streams
            .iter()
            .map(|(_, output)| output.as_str())
            .collect::<String>();
        assert!(combined.contains("Stage 1: /bin/echo first"));
        assert!(combined.contains("Stage 2: /usr/bin/false"));
        assert!(!combined.contains("Stage 3: git status --short"));
        assert_eq!(combined.matches("Exit code: 1").count(), 1);

        let model_path = structured["persisted_output"]["path"]
            .as_str()
            .expect("persisted path missing");
        let physical_path = crate::libs::ai_temp::resolve_ai_temp_path(Path::new(model_path));
        std::fs::remove_file(physical_path).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn commands_without_a_reducer_or_with_complex_syntax_use_the_legacy_path() {
        let temp_root = tempdir().unwrap();
        let project_root = temp_root.path().canonicalize().unwrap();
        initialize_test_git_repository(&project_root);
        let shell = test_shell_execute(project_root);

        for command in [
            "/bin/echo first && /bin/echo second",
            "git status --short | cat",
        ] {
            let result = shell
                .call(json!({ "command": command, "timeout": 10_000 }))
                .await
                .unwrap();
            let structured = result
                .structured_content
                .expect("structured content missing");
            assert!(
                structured.get("stages").is_none(),
                "unexpected split for {command}"
            );
            if let Some(model_path) = structured["persisted_output"]["path"].as_str() {
                let physical_path =
                    crate::libs::ai_temp::resolve_ai_temp_path(Path::new(model_path));
                std::fs::remove_file(physical_path).unwrap();
            }
        }
    }

    #[cfg(unix)]
    fn write_long_running_child_script(project_root: &Path) {
        std::fs::write(
            project_root.join("spawn-child.sh"),
            "sleep 30 &\necho $! > child.pid\nwait\n",
        )
        .unwrap();
    }

    #[cfg(unix)]
    async fn assert_recorded_child_stopped(project_root: &Path) {
        let pid_path = project_root.join("child.pid");
        for _ in 0..20 {
            if pid_path.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let pid = std::fs::read_to_string(&pid_path)
            .expect("child pid file missing")
            .trim()
            .to_string();
        for _ in 0..50 {
            let running = std::process::Command::new("kill")
                .args(["-0", pid.as_str()])
                .status()
                .is_ok_and(|status| status.success());
            if !running {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("child process {pid} survived stage timeout");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn safe_compound_timeout_kills_the_non_streaming_process_group() {
        let temp_root = tempdir().unwrap();
        let project_root = temp_root.path().canonicalize().unwrap();
        initialize_test_git_repository(&project_root);
        write_long_running_child_script(&project_root);
        let shell = test_shell_execute(project_root.clone());

        let result = shell
            .call(json!({
                "command": "sh spawn-child.sh && git status --short",
                "timeout": 150,
            }))
            .await;

        assert!(
            matches!(result, Err(ToolError::ExecutionFailed(message)) if message.contains("timed out after 150ms"))
        );
        assert_recorded_child_stopped(&project_root).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn safe_compound_timeout_kills_the_streaming_process_group() {
        let temp_root = tempdir().unwrap();
        let project_root = temp_root.path().canonicalize().unwrap();
        initialize_test_git_repository(&project_root);
        write_long_running_child_script(&project_root);
        let gateway = Arc::new(RecordingGateway::default());
        let shell = test_shell_execute(project_root.clone())
            .with_gateway(gateway, "test-session".to_string());

        let result = shell
            .call(json!({
                "command": "sh spawn-child.sh && git status --short",
                "timeout": 150,
            }))
            .await;

        assert!(
            matches!(result, Err(ToolError::ExecutionFailed(message)) if message.contains("timed out after 150ms"))
        );
        assert_recorded_child_stopped(&project_root).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn safe_compound_execution_uses_one_total_timeout_budget() {
        let temp_root = tempdir().unwrap();
        let project_root = temp_root.path().canonicalize().unwrap();
        initialize_test_git_repository(&project_root);
        let shell = test_shell_execute(project_root);
        let started = Instant::now();

        let result = shell
            .call(json!({
                "command": "/bin/sleep 0.08 && /bin/sleep 0.08 && git status --short",
                "timeout": 120,
            }))
            .await;

        assert!(
            matches!(result, Err(ToolError::ExecutionFailed(message)) if message.contains("timed out after 120ms"))
        );
        assert!(started.elapsed() < Duration::from_millis(220));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn safe_compound_execution_revalidates_cd_against_path_guard() {
        let temp_root = tempdir().unwrap();
        let project_root = temp_root.path().canonicalize().unwrap();
        let outside_root = tempdir().unwrap();
        initialize_test_git_repository(&project_root);
        let shell = test_shell_execute(project_root.clone());
        let plan = parse_safe_compound_command("cd ../outside && git status")
            .expect("command should be syntactically safe");

        let result = shell
            .call_safe_compound(plan, 10_000, Some(project_root))
            .await;

        assert!(matches!(result, Err(ToolError::Security(_))));
        drop(outside_root);
    }

    #[test]
    fn test_policy_engine_blocks_gitignored_bare_filename_for_cat() {
        let temp_root = tempdir().unwrap();
        let project_root = temp_root.path().canonicalize().unwrap();
        std::fs::write(project_root.join(".gitignore"), "ignored.txt\n").unwrap();
        std::fs::write(project_root.join("ignored.txt"), "secret").unwrap();
        std::fs::write(project_root.join("visible.txt"), "ok").unwrap();

        let guard = Arc::new(RwLock::new(PathGuard::new(
            vec![project_root.clone()],
            vec![],
            vec![],
        )));
        let engine = ShellPolicyEngine::new(guard, vec![]);

        assert!(matches!(
            engine.check("cat ignored.txt", false),
            ShellDecision::Deny(_)
        ));
        assert_eq!(engine.check("cat visible.txt", false), ShellDecision::Allow);
    }

    #[test]
    fn test_policy_engine_blocks_gitignored_bare_filename_for_grep_file_operand() {
        let temp_root = tempdir().unwrap();
        let project_root = temp_root.path().canonicalize().unwrap();
        std::fs::write(project_root.join(".gitignore"), "ignored.log\n").unwrap();
        std::fs::write(project_root.join("ignored.log"), "needle").unwrap();
        std::fs::write(project_root.join("visible.log"), "needle").unwrap();

        let guard = Arc::new(RwLock::new(PathGuard::new(
            vec![project_root.clone()],
            vec![],
            vec![],
        )));
        let engine = ShellPolicyEngine::new(guard, vec![]);

        assert!(matches!(
            engine.check("grep needle ignored.log", false),
            ShellDecision::Deny(_)
        ));
        assert_eq!(
            engine.check("grep needle visible.log", false),
            ShellDecision::Allow
        );
    }
}
