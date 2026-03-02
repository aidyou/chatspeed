use crate::ai::traits::chat::MCPToolDeclaration;
use crate::tools::{NativeToolResult, ToolCallResult, ToolCategory, ToolDefinition, ToolError};
use crate::workflow::react::error::WorkflowEngineError;
use crate::workflow::react::security::PathGuard;
use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, RwLock};
use tokio::sync::Mutex;

/// Decision levels for shell auditing
#[derive(Debug, PartialEq, Clone, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShellDecision {
    Allow,
    Review(String),
    Deny(String),
}

#[derive(Debug, Clone, serde::Deserialize)]
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

    pub fn check(&self, command_str: &str) -> ShellDecision {
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
                    match self.check(inner.as_str()) {
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
        let redirection_ops = [">", ">>", "1>", "2>", "<"];

        let hard_deny = [
            "mkfs", "dd", "format", "fdisk", "parted", "sudo", "su", "ssh", "scp",
        ];
        let needs_review = [
            "rm", "mv", "chmod", "chown", "ln", "kill", "pkill", "crontab", "alias", "eval",
            "python", "perl", "ruby", "node", "php", "sh", "bash", "zsh",
        ];

        let mut final_decision = ShellDecision::Allow;

        for (i, token) in tokens.iter().enumerate() {
            let token_str = token.as_str();

            if separators.contains(&token_str) {
                next_is_binary = true;
                continue;
            }

            if redirection_ops.contains(&token_str) {
                if let Some(next_token) = tokens.get(i + 1) {
                    if !next_token.starts_with('-') {
                        match self.validate_path_token(next_token) {
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
                match self.validate_path_token(token) {
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

        final_decision
    }

    fn validate_path_token(&self, token: &str) -> ShellDecision {
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
                            guard.validate(Path::new(expanded_str))
                        } else {
                            Err(WorkflowEngineError::Security("Lock failed".into()))
                        };
                        if let Err(e) = valid {
                            return ShellDecision::Deny(format!("Boundary Violation: {}", e));
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
}

impl ShellExecute {
    pub fn new(
        path_guard: Arc<RwLock<PathGuard>>,
        tsid_generator: Arc<crate::libs::tsid::TsidGenerator>,
        custom_rules: Vec<ShellPolicyRule>,
    ) -> Self {
        Self {
            policy_engine: ShellPolicyEngine::new(path_guard, custom_rules),
            tsid_generator,
        }
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
        }
    }

    async fn call(&self, params: Value) -> NativeToolResult {
        let command_str = params["command"]
            .as_str()
            .ok_or(ToolError::InvalidParams("command required".into()))?;

        match self.policy_engine.check(command_str) {
            ShellDecision::Allow => {}
            ShellDecision::Review(reason) => {
                return Err(ToolError::Security(format!("REVIEW_REQUIRED: {}", reason)))
            }
            ShellDecision::Deny(reason) => return Err(ToolError::Security(reason)),
        }

        let _timeout_ms = params["timeout"].as_u64().unwrap_or(120_000).min(600_000);
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
                let output = if cfg!(target_os = "windows") {
                    Command::new("cmd").args(["/C", &cmd_to_run]).output()
                } else {
                    Command::new("sh").args(["-c", &cmd_to_run]).output()
                };
                match output {
                    Ok(out) => {
                        *stdout_arc.lock().await = String::from_utf8_lossy(&out.stdout).to_string();
                        *stderr_arc.lock().await = String::from_utf8_lossy(&out.stderr).to_string();
                        *status_arc.lock().await = if out.status.success() {
                            "Completed".into()
                        } else {
                            "Error".into()
                        };
                    }
                    Err(e) => {
                        *stderr_arc.lock().await = format!("Failed to spawn: {}", e);
                        *status_arc.lock().await = "Error".into();
                    }
                }
            });

            return Ok(ToolCallResult::success(
                Some(json!({ "task_id": task_id, "status": "Started" }).to_string()),
                None,
            ));
        }

        let output = if cfg!(target_os = "windows") {
            Command::new("cmd").args(["/C", command_str]).output()
        } else {
            Command::new("sh").args(["-c", command_str]).output()
        }
        .map_err(|e| ToolError::ExecutionFailed(format!("Spawn failed: {}", e)))?;

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::react::security::PathGuard;
    use tempfile::tempdir;

    fn setup_test_context() -> (tempfile::TempDir, std::path::PathBuf, Arc<RwLock<PathGuard>>) {
        let root = tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();
        let guard = Arc::new(RwLock::new(PathGuard::new(vec![
            root_path.clone(),
            std::env::current_dir().unwrap(),
        ])));
        (root, root_path, guard)
    }

    #[test]
    fn test_policy_engine_basic() {
        let (_root, root_path, guard) = setup_test_context();
        let engine = ShellPolicyEngine::new(guard, vec![]);
        assert_eq!(engine.check("ls"), ShellDecision::Allow);
        assert_eq!(
            engine.check(&format!("ls {}", root_path.display())),
            ShellDecision::Allow
        );
    }

    #[test]
    fn test_policy_engine_blocked_binaries() {
        let (_root, _, guard) = setup_test_context();
        let engine = ShellPolicyEngine::new(guard, vec![]);
        assert!(matches!(
            engine.check("sudo rm -rf /"),
            ShellDecision::Deny(_)
        ));
        assert!(matches!(
            engine.check("rm -rf test"),
            ShellDecision::Review(_)
        ));
    }

    #[test]
    fn test_policy_engine_custom_rules() {
        let (_root, _, guard) = setup_test_context();
        let rules = vec![
            ShellPolicyRule {
                pattern: "^ls -la$".to_string(),
                decision: ShellDecision::Deny("Forbidden".into()),
            },
            ShellPolicyRule {
                pattern: "rm -rf /safe/.*".to_string(),
                decision: ShellDecision::Allow,
            },
        ];
        let engine = ShellPolicyEngine::new(guard, rules);
        assert!(matches!(engine.check("ls -la"), ShellDecision::Deny(_)));
        assert_eq!(engine.check("rm -rf /safe/test"), ShellDecision::Allow);
    }
}
