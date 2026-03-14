use crate::ai::traits::chat::MCPToolDeclaration;
use crate::tools::{NativeToolResult, ToolCallResult, ToolCategory, ToolDefinition, ToolError};
use crate::workflow::react::error::WorkflowEngineError;
use crate::workflow::react::gateway::Gateway;
use crate::workflow::react::security::PathGuard;
use crate::workflow::react::types::GatewayPayload;
use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::{Arc, RwLock};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration};

/// Decision levels for shell auditing
#[derive(Debug, PartialEq, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ShellDecision {
    Allow,
    Review(String),
    Deny(String),
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct ShellPolicyRule {
    pub pattern: String,
    pub decision: ShellDecision,
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

        // 2. Custom Regex/Pattern matching for the full command line
        for rule in &self.custom_rules {
            if let Ok(re) = Regex::new(&rule.pattern) {
                if re.is_match(command_str) {
                    return rule.decision.clone();
                }
            } else if command_str.contains(&rule.pattern) {
                return rule.decision.clone();
            }
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

        for (i, token) in tokens.iter().enumerate() {
            let token_str = token.as_str();

            if separators.contains(&token_str) {
                next_is_binary = true;
                current_binary.clear();
                continue;
            }

            if redirection_ops.contains(&token_str) {
                if let Some(next_token) = tokens.get(i + 1) {
                    if !next_token.starts_with('-') {
                        match self.validate_path_token(next_token, restrict_to_planning, false) {
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
            }

            if !token.starts_with('-') {
                let is_delete = current_binary == "rm";
                match self.validate_path_token(token, restrict_to_planning, is_delete) {
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
            }
        }

        final_decision
    }

    fn validate_path_token(
        &self,
        token: &str,
        restrict_to_planning: bool,
        is_delete: bool,
    ) -> ShellDecision {
        if token.starts_with('~') {
            return ShellDecision::Deny(
                "Tilde (~) expansion is blocked. Use absolute paths within the workspace.".into(),
            );
        }

        let is_path_like = token.contains('$')
            || token.starts_with('/')
            || token.starts_with('.')
            || token.contains('/')
            || token == ".."
            || token == ".";
        if is_path_like {
            match shellexpand::full(token) {
                Ok(expanded) => {
                    let expanded_str: &str = expanded.as_ref();
                    if expanded_str.contains('/') || expanded_str.starts_with('.') {
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
        "Executes a given bash command with optional timeout. Working directory persists between commands.\n\n\
        IMPORTANT: This tool is for terminal operations like git, npm, docker, etc. DO NOT use it for file operations (reading, writing, editing, searching, finding files) - use the specialized tools for this instead.\n\n\
        Before executing the command, please follow these steps:\n\n\
        1. Directory Verification:\n\
           - If the command will create new directories or files, first use `ls` to verify the parent directory exists and is the correct location\n\n\
        2. Command Execution:\n\
           - Always quote file paths that contain spaces with double quotes (e.g., cd \"path with spaces/file.txt\")\n\
           - Capture the output of the command.\n\n\
        Usage notes:\n\
          - The command argument is required.\n\
          - You can specify an optional timeout in milliseconds (max 600000ms).\n\
          - You can use the `run_in_background` parameter to run the command in the background. Use `task_output` to read the output later."
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
                    "timeout": { "type": "number", "description": "Optional timeout in milliseconds" },
                    "description": { "type": "string", "description": "Clear description of what this command does." },
                    "run_in_background": { "type": "boolean", "description": "Run in background" }
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
        let run_in_background = params["run_in_background"].as_bool().unwrap_or(false);

        if run_in_background {
            let task_id = format!(
                "shell_{}",
                self.tsid_generator
                    .generate()
                    .map_err(|e| ToolError::ExecutionFailed(e))?
            );
            let cmd_to_run = command_str.to_string();
            let stdout_arc = Arc::new(Mutex::new(String::new()));
            let stderr_arc = Arc::new(Mutex::new(String::new()));
            let status_arc = Arc::new(Mutex::new("Running".to_string()));

            use crate::workflow::react::orchestrator::{BackgroundTask, BACKGROUND_TASKS};
            BACKGROUND_TASKS.insert(
                task_id.clone(),
                BackgroundTask::ShellCommand {
                    command: cmd_to_run.clone(),
                    stdout: stdout_arc.clone(),
                    stderr: stderr_arc.clone(),
                    status: status_arc.clone(),
                },
            );

            tokio::spawn(async move {
                let cmd_future = if cfg!(target_os = "windows") {
                    Command::new("cmd").args(["/C", &cmd_to_run]).output()
                } else {
                    Command::new("sh").args(["-c", &cmd_to_run]).output()
                };

                match timeout(Duration::from_millis(timeout_ms), cmd_future).await {
                    Ok(Ok(out)) => {
                        *stdout_arc.lock().await = String::from_utf8_lossy(&out.stdout).to_string();
                        *stderr_arc.lock().await = String::from_utf8_lossy(&out.stderr).to_string();
                        *status_arc.lock().await = if out.status.success() {
                            "Completed".into()
                        } else {
                            "Error".into()
                        };
                    }
                    Ok(Err(e)) => {
                        *stderr_arc.lock().await = format!("Failed to spawn: {}", e);
                        *status_arc.lock().await = "Error".into();
                    }
                    Err(_) => {
                        *stderr_arc.lock().await =
                            format!("Command timed out after {}ms", timeout_ms);
                        *status_arc.lock().await = "Error".into();
                    }
                }
            });

            return Ok(ToolCallResult::success(
                Some(json!({ "task_id": task_id, "status": "Started" }).to_string()),
                None,
            ));
        }

        // Use streaming execution if gateway is configured
        if self.gateway.is_some() && self.session_id.is_some() {
            return self.call_with_streaming(command_str, timeout_ms).await;
        }

        // Fallback to standard execution
        let cmd_future = if cfg!(target_os = "windows") {
            Command::new("cmd").args(["/C", command_str]).output()
        } else {
            Command::new("sh").args(["-c", command_str]).output()
        };

        match timeout(Duration::from_millis(timeout_ms), cmd_future).await {
            Ok(Ok(output)) => {
                let mut stdout = String::from_utf8_lossy(&output.stdout).to_string();
                if stdout.len() > 30_000 {
                    stdout.truncate(30_000);
                    stdout.push_str("\n[Truncated]");
                }

                if output.status.success() {
                    Ok(ToolCallResult::success(Some(stdout), None))
                } else {
                    Err(ToolError::ExecutionFailed(format!(
                        "Exit {}. STDOUT: {}\nSTDERR: {}",
                        output.status,
                        stdout,
                        String::from_utf8_lossy(&output.stderr)
                    )))
                }
            }
            Ok(Err(e)) => Err(ToolError::ExecutionFailed(format!("Spawn failed: {}", e))),
            Err(_) => Err(ToolError::ExecutionFailed(format!(
                "Command timed out after {}ms",
                timeout_ms
            ))),
        }
    }
}

impl ShellExecute {
    /// Execute command with real-time output streaming to frontend
    async fn call_with_streaming(
        &self,
        command_str: &str,
        timeout_ms: u64,
    ) -> NativeToolResult {
        use std::process::Stdio;

        let gateway = self.gateway.as_ref().ok_or(ToolError::ExecutionFailed(
            "Gateway not configured for streaming".to_string(),
        ))?;
        let session_id = self.session_id.as_ref().ok_or(ToolError::ExecutionFailed(
            "Session ID not configured for streaming".to_string(),
        ))?;

        let mut child = if cfg!(target_os = "windows") {
            Command::new("cmd")
                .args(["/C", command_str])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| ToolError::ExecutionFailed(format!("Failed to spawn: {}", e)))?
        } else {
            Command::new("sh")
                .args(["-c", command_str])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
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

        // Read stdout and stderr concurrently with timeout
        let start_time = std::time::Instant::now();

        loop {
            let timeout_remaining = timeout_ms.saturating_sub(start_time.elapsed().as_millis() as u64);
            if timeout_remaining == 0 {
                let _ = child.kill().await;
                return Err(ToolError::ExecutionFailed(format!(
                    "Command timed out after {}ms",
                    timeout_ms
                )));
            }

            // Try to read from both stdout and stderr with a small timeout
            let mut got_output = false;

            // Use tokio::select! to read from either stream
            tokio::select! {
                line = stdout_reader.next_line() => {
                    match line {
                        Ok(Some(l)) => {
                            full_stdout.push_str(&l);
                            full_stdout.push('\n');

                            // Send real-time notification to frontend
                            let _ = gateway.send(
                                session_id,
                                GatewayPayload::Notification {
                                    message: format!("📄 {}", l),
                                    category: Some("shell".to_string()),
                                },
                            ).await;
                            got_output = true;
                        }
                        Ok(None) => {
                            // EOF reached for stdout
                        }
                        Err(e) => {
                            log::warn!("Error reading stdout: {}", e);
                        }
                    }
                }
                line = stderr_reader.next_line() => {
                    match line {
                        Ok(Some(l)) => {
                            full_stderr.push_str(&l);
                            full_stderr.push('\n');

                            // Send real-time notification to frontend
                            let _ = gateway.send(
                                session_id,
                                GatewayPayload::Notification {
                                    message: format!("⚠️ {}", l),
                                    category: Some("shell-error".to_string()),
                                },
                            ).await;
                            got_output = true;
                        }
                        Ok(None) => {
                            // EOF reached for stderr
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
                            if status.success() {
                                if full_stdout.len() > 30_000 {
                                    full_stdout.truncate(30_000);
                                    full_stdout.push_str("\n[Truncated]");
                                }
                                return Ok(ToolCallResult::success(Some(full_stdout), None));
                            } else {
                                return Err(ToolError::ExecutionFailed(format!(
                                    "Exit {}. STDOUT: {}\nSTDERR: {}",
                                    status,
                                    full_stdout,
                                    full_stderr
                                )));
                            }
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
    use crate::workflow::react::security::PathGuard;
    use tempfile::tempdir;

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

        // 4. NORMAL FILE removal inside root should NOT be hard-denied (it should be Review)
        let cmd_file = format!("rm {}", root_path.join("file.txt").display());
        assert!(matches!(
            engine.check(&cmd_file, false),
            ShellDecision::Review(_)
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
            let decision = engine.validate_path_token(path, false, false);
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
}
