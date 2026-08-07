use crate::ai::traits::chat::MCPToolDeclaration;
#[cfg(test)]
use crate::libs::ai_temp::ToolOutputWriter;
use crate::tools::helper::is_node_build_command;
use crate::tools::helper::{
    classify_shell_stage, leading_command_index, shell_tokens, split_shell_command_segments,
    ShellStage,
};
#[cfg(test)]
use crate::tools::helper::{SafeCompoundCommand, SafeCompoundStage};
#[cfg(test)]
use crate::tools::shell_output::{
    build_compound_shell_tool_result, prepare_shell_output, CompoundShellStageResult,
};
use crate::tools::shell_output::{
    build_shell_tool_result_with_metadata, should_collect_stderr_line_as_stdout,
    should_suppress_incidental_termination_stderr, AnsiOutputSanitizer,
};
use crate::tools::{NativeToolResult, ToolCategory, ToolDefinition, ToolError};
use crate::workflow::react::error::WorkflowEngineError;
use crate::workflow::react::gateway::Gateway;
use crate::workflow::react::security::PathGuard;
use crate::workflow::react::types::GatewayPayload;
use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
#[cfg(test)]
use tokio::io::AsyncReadExt;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
#[cfg(test)]
use tokio::time::Instant;
use tokio::time::{timeout, Duration};

/// Decision levels for shell auditing
#[derive(Debug, PartialEq, Clone)]
pub enum ShellDecision {
    Allow,
    Review(String),
    Deny(String),
}

impl serde::Serialize for ShellDecision {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Allow => serializer.serialize_str("allow"),
            Self::Review(reason) => serializer.serialize_str(&format!("review:{reason}")),
            Self::Deny(reason) => serializer.serialize_str(&format!("deny:{reason}")),
        }
    }
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
    const MAX_EXECUTION_AUDIT_DEPTH: usize = 16;

    fn merge_audit_decisions(current: ShellDecision, candidate: ShellDecision) -> ShellDecision {
        match candidate {
            ShellDecision::Deny(_) => candidate,
            ShellDecision::Review(_) if current == ShellDecision::Allow => candidate,
            _ => current,
        }
    }

    fn executable_name(token: &str) -> String {
        token
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(token)
            .trim_end_matches(".exe")
            .to_ascii_lowercase()
    }

    fn hard_denied_command(command: &str) -> bool {
        matches!(
            command,
            "mkfs"
                | "dd"
                | "format"
                | "fdisk"
                | "parted"
                | "sudo"
                | "su"
                | "doas"
                | "pkexec"
                | "runuser"
                | "chsh"
                | "newgrp"
                | "sg"
                | "ssh"
                | "scp"
                | "useradd"
                | "adduser"
                | "userdel"
                | "deluser"
                | "usermod"
                | "chage"
                | "passwd"
                | "vipw"
                | "groupadd"
                | "addgroup"
                | "groupdel"
                | "delgroup"
                | "groupmod"
                | "gpasswd"
                | "vigr"
        )
    }

    fn contains_hard_denied_command_text(text: &str) -> bool {
        text.split(|character: char| !character.is_ascii_alphanumeric())
            .filter(|token| !token.is_empty())
            .map(str::to_ascii_lowercase)
            .any(|token| Self::hard_denied_command(&token))
    }

    fn command_after_options<'a>(
        arguments: &'a [String],
        options_with_values: &[&str],
    ) -> Option<&'a [String]> {
        let mut index = 0;
        while index < arguments.len() {
            let argument = arguments[index].as_str();
            if argument == "--" {
                return arguments.get(index + 1..);
            }
            if argument == "-" || !argument.starts_with('-') {
                return Some(&arguments[index..]);
            }

            let consumes_next = options_with_values.iter().any(|option| argument == *option);
            let has_attached_value = options_with_values.iter().any(|option| {
                argument.starts_with(option)
                    && argument != *option
                    && (option.starts_with("--") || option.len() == 2)
            });
            index += if consumes_next && !has_attached_value {
                2
            } else {
                1
            };
        }
        None
    }

    pub fn execution_audit_decision(&self, command_str: &str) -> ShellDecision {
        self.audit_execution_forms(command_str, 0)
    }

    fn audit_execution_forms(&self, command_str: &str, depth: usize) -> ShellDecision {
        if depth >= Self::MAX_EXECUTION_AUDIT_DEPTH {
            return ShellDecision::Deny(
                "Nested shell execution exceeds the audit limit.".to_string(),
            );
        }

        let mut decision = ShellDecision::Allow;
        let nested_patterns = [
            Regex::new(r"\$\((?P<inner>.*?)\)").unwrap(),
            Regex::new(r"`(?P<inner>.*?)`").unwrap(),
            Regex::new(r"<\s*\((?P<inner>.*?)\)").unwrap(),
            Regex::new(r">\s*\((?P<inner>.*?)\)").unwrap(),
        ];
        for pattern in nested_patterns {
            for captures in pattern.captures_iter(command_str) {
                if let Some(inner) = captures.name("inner") {
                    decision = Self::merge_audit_decisions(
                        decision,
                        self.audit_execution_forms(inner.as_str(), depth + 1),
                    );
                    if matches!(decision, ShellDecision::Deny(_)) {
                        return decision;
                    }
                }
            }
        }

        for segment in split_shell_command_segments(command_str) {
            let Some(tokens) = shell_tokens(&segment) else {
                continue;
            };
            decision =
                Self::merge_audit_decisions(decision, self.audit_execution_tokens(&tokens, depth));
            if matches!(decision, ShellDecision::Deny(_)) {
                return decision;
            }
        }

        decision
    }

    fn audit_execution_tokens(&self, tokens: &[String], depth: usize) -> ShellDecision {
        let index = leading_command_index(tokens);
        let Some(command) = tokens.get(index) else {
            return ShellDecision::Allow;
        };
        let executable = Self::executable_name(command);
        if Self::hard_denied_command(&executable) {
            return ShellDecision::Deny(format!(
                "System-critical command '{}' is forbidden.",
                executable
            ));
        }

        let arguments = &tokens[index + 1..];
        match executable.as_str() {
            "find" => self.audit_find_actions(arguments, depth),
            "env" => self.audit_env_wrapper(arguments, depth),
            "command" => self.audit_command_wrapper(arguments, depth),
            "watch" => self.audit_watch_wrapper(arguments, depth),
            "xargs" => self.audit_xargs_wrapper(arguments, depth),
            "sh" | "bash" | "zsh" | "dash" | "ksh" => {
                let mut decision = ShellDecision::Review(format!(
                    "Shell interpreter '{}' requires execution audit.",
                    executable
                ));
                if let Some(script_index) = arguments
                    .iter()
                    .position(|argument| argument == "-c" || argument.starts_with("-c"))
                {
                    let script = arguments[script_index]
                        .strip_prefix("-c")
                        .filter(|script| !script.is_empty())
                        .map(ToString::to_string)
                        .or_else(|| arguments.get(script_index + 1).cloned());
                    if let Some(script) = script {
                        decision = Self::merge_audit_decisions(
                            decision,
                            self.audit_execution_forms(&script, depth + 1),
                        );
                    }
                }
                decision
            }
            "eval" => Self::merge_audit_decisions(
                ShellDecision::Review("Shell eval requires execution audit.".to_string()),
                self.audit_execution_forms(&arguments.join(" "), depth + 1),
            ),
            "awk"
                if arguments.iter().any(|argument| {
                    argument.contains("system(")
                        || argument.contains("| getline")
                        || argument.contains("|&")
                        || argument.contains("| \"")
                        || argument.contains("| '")
                }) =>
            {
                if arguments
                    .iter()
                    .any(|argument| Self::contains_hard_denied_command_text(argument))
                {
                    ShellDecision::Deny(
                        "Dynamic awk execution contains a forbidden system command.".to_string(),
                    )
                } else {
                    ShellDecision::Review(
                        "Dynamic awk execution requires execution audit.".to_string(),
                    )
                }
            }
            "source" | "." => {
                ShellDecision::Review("Shell source requires execution audit.".to_string())
            }
            "time" => self.audit_direct_wrapper(
                "time",
                Self::command_after_options(arguments, &["-o", "--output", "-f", "--format"]),
                depth,
            ),
            "nice" => self.audit_direct_wrapper(
                "nice",
                Self::command_after_options(arguments, &["-n", "--adjustment"]),
                depth,
            ),
            "timeout" => self.audit_direct_wrapper(
                "timeout",
                Self::command_after_options(arguments, &["-s", "--signal", "-k", "--kill-after"])
                    .and_then(|arguments| arguments.get(1..)),
                depth,
            ),
            "stdbuf" => self.audit_direct_wrapper(
                "stdbuf",
                Self::command_after_options(arguments, &["-i", "-o", "-e"]),
                depth,
            ),
            "nohup" | "setsid" | "exec" | "builtin" | "coproc" => self.audit_direct_wrapper(
                executable.as_str(),
                Self::command_after_options(arguments, &["-a"]),
                depth,
            ),
            _ => ShellDecision::Allow,
        }
    }

    fn audit_direct_wrapper(
        &self,
        wrapper: &str,
        child_tokens: Option<&[String]>,
        depth: usize,
    ) -> ShellDecision {
        let mut decision = ShellDecision::Review(format!(
            "Command wrapper '{}' requires execution audit.",
            wrapper
        ));
        if let Some(child_tokens) = child_tokens {
            decision = Self::merge_audit_decisions(
                decision,
                self.audit_execution_tokens(child_tokens, depth + 1),
            );
        }
        decision
    }

    fn audit_env_wrapper(&self, arguments: &[String], depth: usize) -> ShellDecision {
        let mut index = 0;
        while index < arguments.len() {
            let argument = arguments[index].as_str();
            if matches!(argument, "-S" | "--split-string") {
                let mut decision = ShellDecision::Review(
                    "Environment command with split-string requires execution audit.".to_string(),
                );
                if let Some(script) = arguments.get(index + 1) {
                    decision = Self::merge_audit_decisions(
                        decision,
                        self.audit_execution_forms(script, depth + 1),
                    );
                }
                return decision;
            }
            if let Some(script) = argument.strip_prefix("--split-string=").or_else(|| {
                argument
                    .strip_prefix("-S")
                    .filter(|script| !script.is_empty())
            }) {
                return Self::merge_audit_decisions(
                    ShellDecision::Review(
                        "Environment command with split-string requires execution audit."
                            .to_string(),
                    ),
                    self.audit_execution_forms(script, depth + 1),
                );
            }
            if argument == "--" {
                index += 1;
                break;
            }
            if matches!(argument, "-u" | "--unset" | "-C" | "--chdir") {
                index += 2;
                continue;
            }
            if argument.starts_with("--unset=")
                || argument.starts_with("--chdir=")
                || (argument.starts_with("-u") && argument.len() > 2)
                || (argument.starts_with("-C") && argument.len() > 2)
            {
                index += 1;
                continue;
            }
            if argument.starts_with('-') {
                index += 1;
                continue;
            }
            break;
        }

        let child_start = index + leading_command_index(&arguments[index..]);
        if child_start >= arguments.len() {
            ShellDecision::Allow
        } else {
            self.audit_direct_wrapper("env", arguments.get(child_start..), depth)
        }
    }

    fn audit_command_wrapper(&self, arguments: &[String], depth: usize) -> ShellDecision {
        let mut index = 0;
        while index < arguments.len() {
            match arguments[index].as_str() {
                "--" => {
                    return self.audit_direct_wrapper("command", arguments.get(index + 1..), depth)
                }
                "-v" | "-V" => return ShellDecision::Allow,
                "-p" => index += 1,
                argument if argument.starts_with('-') => index += 1,
                _ => return self.audit_direct_wrapper("command", arguments.get(index..), depth),
            }
        }
        ShellDecision::Allow
    }

    fn audit_watch_wrapper(&self, arguments: &[String], depth: usize) -> ShellDecision {
        let mut decision =
            ShellDecision::Review("Command wrapper 'watch' requires execution audit.".to_string());
        if let Some(command) = Self::command_after_options(arguments, &["-n", "--interval"]) {
            decision = Self::merge_audit_decisions(
                decision,
                self.audit_execution_forms(&command.join(" "), depth + 1),
            );
        }
        decision
    }

    fn audit_xargs_wrapper(&self, arguments: &[String], depth: usize) -> ShellDecision {
        let mut decision =
            ShellDecision::Review("Command wrapper 'xargs' requires execution audit.".to_string());
        if let Some(command) = Self::command_after_options(
            arguments,
            &[
                "-E",
                "--eof",
                "-I",
                "--replace",
                "-L",
                "--max-lines",
                "-n",
                "--max-args",
                "-P",
                "--max-procs",
                "-s",
                "--max-chars",
                "-a",
                "--arg-file",
                "-d",
                "--delimiter",
            ],
        ) {
            decision = Self::merge_audit_decisions(
                decision,
                self.audit_execution_tokens(command, depth + 1),
            );
        }
        decision
    }

    fn audit_find_actions(&self, arguments: &[String], depth: usize) -> ShellDecision {
        let mut decision = ShellDecision::Allow;
        let mut index = 0;
        while index < arguments.len() {
            let argument = arguments[index].as_str();
            if matches!(
                argument,
                "-delete" | "-fprint" | "-fprint0" | "-fprintf" | "-fls"
            ) {
                decision = Self::merge_audit_decisions(
                    decision,
                    ShellDecision::Review(
                        "Mutating find action requires execution audit.".to_string(),
                    ),
                );
            }
            if matches!(argument, "-exec" | "-execdir" | "-ok" | "-okdir") {
                let command_start = index + 1;
                let command_end = arguments[command_start..]
                    .iter()
                    .position(|token| matches!(token.as_str(), ";" | "+"))
                    .map(|offset| command_start + offset)
                    .unwrap_or(arguments.len());
                decision = Self::merge_audit_decisions(
                    decision,
                    ShellDecision::Review(
                        "Executable find action requires execution audit.".to_string(),
                    ),
                );
                if command_start < command_end {
                    decision = Self::merge_audit_decisions(
                        decision,
                        self.audit_execution_tokens(
                            &arguments[command_start..command_end],
                            depth + 1,
                        ),
                    );
                }
                index = command_end;
            }
            if matches!(decision, ShellDecision::Deny(_)) {
                return decision;
            }
            index += 1;
        }
        decision
    }

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

        // 2. Audit command wrappers and indirect execution before evaluating reusable allow rules.
        // A custom allow rule can grant a known command, but cannot suppress an execution audit.
        let execution_audit = self.audit_execution_forms(command_str, 0);
        if let ShellDecision::Deny(reason) = &execution_audit {
            return ShellDecision::Deny(reason.clone());
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
        let mut has_boundary_review = false;
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
                has_boundary_review = true;
                if final_decision == ShellDecision::Allow {
                    final_decision = ShellDecision::Review("File redirection detected".into());
                }
                continue;
            }

            let clean_token = Self::executable_name(token);

            if next_is_binary {
                current_binary = clean_token.clone();
                if Self::hard_denied_command(&clean_token) {
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
                        has_boundary_review = true;
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

        let baseline_decision =
            Self::merge_audit_decisions(execution_audit.clone(), final_decision);
        if self.custom_rules.is_empty() {
            return baseline_decision;
        }

        let custom_decision = self.evaluate_custom_rules(command_str, restrict_to_planning);
        match custom_decision {
            ShellDecision::Deny(reason) => ShellDecision::Deny(reason),
            ShellDecision::Review(reason) if baseline_decision == ShellDecision::Allow => {
                ShellDecision::Review(reason)
            }
            ShellDecision::Review(_) => baseline_decision,
            ShellDecision::Allow
                if !has_boundary_review && matches!(execution_audit, ShellDecision::Allow) =>
            {
                ShellDecision::Allow
            }
            ShellDecision::Allow => baseline_decision,
        }
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
        let normalized_segments =
            match self.extract_policy_match_segments(command_str, restrict_to_planning) {
                Ok(segments) => segments,
                Err(decision) => return decision,
            };

        if normalized_segments.is_empty() {
            return ShellDecision::Allow;
        }

        // A rule matching the first command must never authorize later commands
        // in a compound shell expression. Preserve full-command matching only
        // for a genuinely single command, then require every compound segment
        // to match an explicit policy rule below.
        if normalized_segments.len() == 1 {
            if let Some(decision) = self.match_custom_rule(command_str) {
                return decision;
            }
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
    sandbox_config: Option<crate::tools::AgentSandboxConfig>,
    gateway: Option<Arc<dyn Gateway>>,
    session_id: Option<String>,
    approved_execution_plans: Arc<dashmap::DashMap<String, crate::tools::ShellExecutionPlan>>,
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
            sandbox_config: None,
            gateway: None,
            session_id: None,
            approved_execution_plans: Arc::new(dashmap::DashMap::new()),
        }
    }

    pub fn with_sandbox_config(
        mut self,
        sandbox_config: Option<crate::tools::AgentSandboxConfig>,
    ) -> Self {
        self.sandbox_config = sandbox_config;
        self
    }

    pub fn with_approved_execution_plans(
        mut self,
        approved_execution_plans: Arc<dashmap::DashMap<String, crate::tools::ShellExecutionPlan>>,
    ) -> Self {
        self.approved_execution_plans = approved_execution_plans;
        self
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
          - You can specify an optional timeout in milliseconds. When omitted, sandbox execution uses the Profile timeout; otherwise it defaults to 120000ms. Values are capped at 600000ms.\n\
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
                    "timeout": { "type": "number", "description": "Optional AI-selected timeout in milliseconds. When omitted, uses the sandbox profile timeout, then defaults to 120000. Capped at 600000." },
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

        let requested_timeout_ms = params.get("timeout").and_then(Value::as_u64);
        let working_dir = self.default_working_dir();

        // Use streaming execution if gateway is configured
        if self.gateway.is_some() && self.session_id.is_some() {
            return self
                .call_with_streaming(command_str, requested_timeout_ms, params.clone())
                .await;
        }

        let tool_id = params
            .get(crate::constants::INTERNAL_PARAM_TOOL_CALL_ID)
            .and_then(|v| v.as_str())
            .unwrap_or("bash");
        let execution_plan = self.execution_plan_for_params(tool_id, command_str)?;
        if execution_plan.status != crate::tools::ShellExecutionPlanStatus::Ready {
            return Err(ToolError::ExecutionFailed(
                Self::execution_plan_denied_message(&execution_plan),
            ));
        }
        let timeout_ms = crate::tools::effective_timeout_ms(&execution_plan, requested_timeout_ms);
        Self::log_execution_backend_debug(tool_id, command_str, &execution_plan);
        let execution_plan_metadata = serde_json::to_value(&execution_plan).ok();

        if let Some(sandbox_command) =
            crate::tools::sandbox_command_for_plan(&execution_plan, command_str)?
        {
            let output =
                run_sandbox_output_with_timeout(sandbox_command, &execution_plan, timeout_ms).await;
            return match output {
                Ok(output) => {
                    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let exit_code = output.status.code().unwrap_or(-1);
                    Ok(build_shell_tool_result_with_metadata(
                        command_str,
                        exit_code,
                        &stdout,
                        &stderr,
                        execution_plan_metadata.clone(),
                    ))
                }
                Err(error) => Err(error),
            };
        }

        // Execute host compound commands through the platform shell as one command.
        // `parse_safe_compound_command` remains available for policy/output analysis,
        // but AC-9 requires backend execution to preserve original shell semantics.

        // Fallback to standard execution
        let host_command = crate::libs::ai_temp::map_ai_temp_paths_for_host_command(command_str);
        let mut command = if cfg!(target_os = "windows") {
            let mut command = Command::new("cmd");
            command.args(["/C", &host_command]);
            configure_no_window(&mut command);
            command
        } else {
            let mut command = Command::new("sh");
            command.args(["-c", &host_command]);
            command
        };
        if let Some(dir) = &working_dir {
            command.current_dir(dir);
        }

        match run_host_output_with_timeout(command, timeout_ms).await {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr);
                let exit_code = output.status.code().unwrap_or(-1);
                Ok(build_shell_tool_result_with_metadata(
                    command_str,
                    exit_code,
                    &stdout,
                    &stderr,
                    execution_plan_metadata.clone(),
                ))
            }
            Err(error) => Err(error),
        }
    }
}

#[cfg(target_os = "windows")]
fn configure_no_window(command: &mut Command) {
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
}

#[cfg(not(target_os = "windows"))]
fn configure_no_window(_command: &mut Command) {}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) {
    command.process_group(0);
}

#[cfg(not(unix))]
fn configure_process_group(_command: &mut Command) {}

struct StageProcessGuard {
    #[cfg(unix)]
    process_group_id: i32,
    #[cfg(windows)]
    process_id: u32,
    active: bool,
}

impl StageProcessGuard {
    fn new(_child: &Child) -> Self {
        Self {
            #[cfg(unix)]
            process_group_id: _child.id().map_or(0, |id| id as i32),
            #[cfg(windows)]
            process_id: _child.id().unwrap_or(0),
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
        #[cfg(windows)]
        if self.process_id > 0 {
            let mut command = std::process::Command::new("taskkill");
            command
                .args(["/PID", &self.process_id.to_string(), "/T", "/F"])
                .creation_flags(0x08000000); // CREATE_NO_WINDOW
            let _ = command
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
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

async fn run_host_output_with_timeout(
    mut command: Command,
    timeout_ms: u64,
) -> Result<std::process::Output, ToolError> {
    use std::process::Stdio;

    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    configure_process_group(&mut command);
    let child = command
        .spawn()
        .map_err(|error| ToolError::ExecutionFailed(format!("Spawn failed: {error}")))?;
    let mut process_guard = StageProcessGuard::new(&child);

    match timeout(Duration::from_millis(timeout_ms), child.wait_with_output()).await {
        Ok(Ok(output)) => {
            process_guard.disarm();
            Ok(output)
        }
        Ok(Err(error)) => Err(ToolError::ExecutionFailed(format!(
            "Failed to wait for command: {error}"
        ))),
        Err(_) => Err(ToolError::ExecutionFailed(format!(
            "Command timed out after {timeout_ms}ms"
        ))),
    }
}

struct SandboxCleanupGuard {
    plan: Option<crate::tools::ShellExecutionPlan>,
    active: bool,
}

impl SandboxCleanupGuard {
    fn new(plan: Option<crate::tools::ShellExecutionPlan>) -> Self {
        Self { plan, active: true }
    }

    fn disarm(&mut self) {
        self.active = false;
    }

    fn cleanup_required(plan: &crate::tools::ShellExecutionPlan) -> bool {
        matches!(plan.backend, crate::tools::ShellExecutionBackendKind::Msb)
    }

    async fn cleanup_after_success(&mut self) {
        let should_cleanup = self.plan.as_ref().is_some_and(Self::cleanup_required);
        if should_cleanup {
            self.cleanup_now().await;
        } else {
            self.disarm();
        }
    }

    async fn cleanup_now(&mut self) {
        if !self.active {
            return;
        }
        if let Some(plan) = self.plan.as_ref() {
            cleanup_sandbox_execution(plan).await;
        }
        self.active = false;
    }
}

impl Drop for SandboxCleanupGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let Some(plan) = self.plan.as_ref() else {
            return;
        };
        let Some(argv) = crate::tools::sandbox_cleanup_argv_for_plan(plan) else {
            return;
        };
        let mut iter = argv.into_iter();
        let Some(program) = iter.next() else {
            return;
        };
        let _ = std::process::Command::new(program)
            .args(iter)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
}

async fn cleanup_sandbox_execution(plan: &crate::tools::ShellExecutionPlan) {
    if let Some(mut cleanup) = crate::tools::sandbox_cleanup_command_for_plan(plan) {
        configure_no_window(&mut cleanup);
        let _ = timeout(Duration::from_secs(10), cleanup.status()).await;
    }
}

async fn run_sandbox_output_with_timeout(
    mut command: Command,
    plan: &crate::tools::ShellExecutionPlan,
    timeout_ms: u64,
) -> Result<std::process::Output, ToolError> {
    configure_process_group(&mut command);
    let child = command.spawn().map_err(|error| {
        sandbox_failure_error(
            plan,
            crate::tools::SandboxFailureReason::SpawnFailed,
            format!("Sandbox spawn failed: {error}"),
        )
    })?;
    let mut cleanup_guard = SandboxCleanupGuard::new(Some(plan.clone()));
    match timeout(Duration::from_millis(timeout_ms), child.wait_with_output()).await {
        Ok(result) => match result {
            Ok(output) => {
                cleanup_guard.cleanup_after_success().await;
                Ok(output)
            }
            Err(error) => Err(sandbox_failure_error(
                plan,
                crate::tools::SandboxFailureReason::RunnerFailed,
                format!("Sandbox execution failed: {error}"),
            )),
        },
        Err(_) => {
            cleanup_guard.cleanup_now().await;
            Err(sandbox_failure_error(
                plan,
                crate::tools::SandboxFailureReason::TimedOut,
                format!("Command timed out after {timeout_ms}ms"),
            ))
        }
    }
}

fn sandbox_failure_error(
    plan: &crate::tools::ShellExecutionPlan,
    reason: crate::tools::SandboxFailureReason,
    message: impl Into<String>,
) -> ToolError {
    ToolError::SandboxFailure(crate::tools::SandboxFailure::from_plan(
        plan, reason, message,
    ))
}

#[cfg(test)]
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

#[cfg(test)]
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

struct ResultOnlyShellGateway<'a> {
    inner: &'a dyn Gateway,
}

impl<'a> ResultOnlyShellGateway<'a> {
    fn new(inner: &'a dyn Gateway) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl Gateway for ResultOnlyShellGateway<'_> {
    async fn send(
        &self,
        session_id: &str,
        payload: GatewayPayload,
    ) -> Result<(), WorkflowEngineError> {
        if matches!(&payload, GatewayPayload::ToolStream { .. }) {
            return Ok(());
        }
        self.inner.send(session_id, payload).await
    }

    async fn inject_input(
        &self,
        session_id: &str,
        input: String,
    ) -> Result<(), WorkflowEngineError> {
        self.inner.inject_input(session_id, input).await
    }
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

#[cfg(test)]
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
    #[cfg(test)]
    async fn call_safe_compound(
        &self,
        plan: SafeCompoundCommand,
        timeout_ms: u64,
        working_dir: Option<PathBuf>,
        execution_plan_metadata: Option<serde_json::Value>,
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
            execution_plan_metadata,
        ))
    }

    #[cfg(test)]
    async fn call_safe_compound_streaming(
        &self,
        plan: SafeCompoundCommand,
        timeout_ms: u64,
        working_dir: Option<PathBuf>,
        gateway: &dyn Gateway,
        session_id: &str,
        tool_id: &str,
        execution_plan_metadata: Option<serde_json::Value>,
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
            execution_plan_metadata,
        ))
    }

    #[cfg(test)]
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
                            let line = stderr_sanitizer.sanitize(&raw_line);
                            if should_suppress_incidental_termination_stderr(command_str, &line) {
                                continue;
                            }
                            full_stderr.push_str(&raw_line);
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

    fn approved_shell_execution_plan(
        &self,
        tool_call_id: &str,
    ) -> Option<crate::tools::ShellExecutionPlan> {
        self.approved_execution_plans
            .remove(tool_call_id)
            .map(|(_, plan)| plan)
    }

    fn log_execution_backend_debug(
        tool_call_id: &str,
        command: &str,
        plan: &crate::tools::ShellExecutionPlan,
    ) {
        log::debug!(
            "[ShellExecute][debug_route][tool_call_id={}] execution backend={:?} origin={:?} runtime={:?} profile={:?} image={:?} scheme_id={:?} command={}",
            tool_call_id,
            plan.backend,
            plan.backend_origin,
            plan.runtime,
            plan.profile,
            plan.image,
            plan.scheme_id,
            command
        );
    }

    fn execution_plan_denied_message(plan: &crate::tools::ShellExecutionPlan) -> String {
        let status = format!("{:?}", plan.status);
        match &plan.fallback_reason {
            Some(reason) => {
                format!("Sandbox execution is not authorized before spawn: {status} ({reason:?})")
            }
            None => format!("Sandbox execution is not authorized before spawn: {status}"),
        }
    }

    fn execution_plan_for_params(
        &self,
        tool_call_id: &str,
        command_str: &str,
    ) -> Result<crate::tools::ShellExecutionPlan, ToolError> {
        if let Some(plan) = self.approved_shell_execution_plan(tool_call_id) {
            if plan.tool_call_id != tool_call_id || plan.command != command_str {
                return Err(ToolError::ExecutionFailed(
                    "Approved shell execution plan is bound to a different tool call or command; re-approval is required".to_string(),
                ));
            }
            let current = self.resolve_execution_plan(tool_call_id, command_str);
            if plan != current {
                return Err(ToolError::ExecutionFailed(
                    "Approved shell execution plan no longer matches current sandbox resolution; re-approval is required".to_string(),
                ));
            }
            return Ok(plan);
        }
        Ok(self.resolve_execution_plan(tool_call_id, command_str))
    }

    fn resolve_execution_plan(
        &self,
        tool_call_id: &str,
        command_str: &str,
    ) -> crate::tools::ShellExecutionPlan {
        let runtime_status =
            crate::tools::SandboxRuntimeDetector::new(crate::tools::SandboxDetectorOptions {
                required_images: self
                    .sandbox_config
                    .as_ref()
                    .map(crate::tools::AgentSandboxConfig::required_images)
                    .unwrap_or_default(),
                ..crate::tools::SandboxDetectorOptions::default()
            })
            .detect();
        let primary_root = self.default_working_dir();
        let mount_context = self.sandbox_mount_context();
        crate::tools::ShellExecutionResolver::complete_sandbox_mounts(
            crate::tools::ShellExecutionResolver::resolve(
                tool_call_id,
                command_str,
                self.sandbox_config.as_ref(),
                &runtime_status,
                primary_root.as_deref(),
            ),
            &mount_context,
        )
    }

    fn sandbox_mount_context(&self) -> crate::tools::ShellSandboxMountContext {
        self.policy_engine
            .path_guard
            .read()
            .map(|guard| {
                let skill_roots = guard.skill_roots();
                crate::tools::ShellSandboxMountContext {
                    authorized_roots: guard.workspace_roots(),
                    writable_skill_roots: skill_roots
                        .iter()
                        .filter(|path| crate::workflow::react::security::is_user_skill_path(path))
                        .cloned()
                        .collect(),
                    skill_roots,
                }
            })
            .unwrap_or(crate::tools::ShellSandboxMountContext {
                authorized_roots: Vec::new(),
                skill_roots: Vec::new(),
                writable_skill_roots: Vec::new(),
            })
    }

    fn default_working_dir(&self) -> Option<std::path::PathBuf> {
        self.policy_engine
            .path_guard
            .read()
            .ok()
            .and_then(|guard| guard.get_primary_root().map(|path| path.to_path_buf()))
    }

    async fn call_with_streaming_command(
        &self,
        command_str: &str,
        timeout_ms: u64,
        mut command: Command,
        gateway: &dyn Gateway,
        session_id: &str,
        tool_id: &str,
        execution_plan: Option<crate::tools::ShellExecutionPlan>,
    ) -> NativeToolResult {
        configure_process_group(&mut command);
        let mut child = command.spawn().map_err(|e| {
            if let Some(plan) = execution_plan.as_ref() {
                sandbox_failure_error(
                    plan,
                    crate::tools::SandboxFailureReason::SpawnFailed,
                    format!("Failed to spawn sandbox command: {}", e),
                )
            } else {
                ToolError::ExecutionFailed(format!("Failed to spawn command: {}", e))
            }
        })?;
        let mut process_guard = StageProcessGuard::new(&child);
        let mut sandbox_cleanup_guard = SandboxCleanupGuard::new(execution_plan.clone());

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
        let mut stdout_sanitizer = AnsiOutputSanitizer::default();
        let mut stderr_sanitizer = AnsiOutputSanitizer::default();
        let mut stdout_eof = false;
        let mut stderr_eof = false;
        let mut last_stream_name: Option<&'static str> = None;
        let start_time = std::time::Instant::now();

        loop {
            let timeout_remaining =
                timeout_ms.saturating_sub(start_time.elapsed().as_millis() as u64);
            if timeout_remaining == 0 {
                terminate_stage_process(&mut child, &mut process_guard).await;
                sandbox_cleanup_guard.cleanup_now().await;
                return Err(if let Some(plan) = execution_plan.as_ref() {
                    sandbox_failure_error(
                        plan,
                        crate::tools::SandboxFailureReason::TimedOut,
                        format!("Command timed out after {}ms", timeout_ms),
                    )
                } else {
                    ToolError::ExecutionFailed(format!("Command timed out after {}ms", timeout_ms))
                });
            }

            if stdout_eof && stderr_eof {
                if let Some(status) = child.try_wait().map_err(|e| {
                    ToolError::ExecutionFailed(format!("Failed to check process status: {}", e))
                })? {
                    let exit_code = status.code().unwrap_or(-1);
                    let _ = gateway
                        .send(
                            session_id,
                            GatewayPayload::ToolStream {
                                tool_id: tool_id.to_string(),
                                output: if last_stream_name.is_some() {
                                    format!("\nExit code: {}", exit_code)
                                } else {
                                    format!("Exit code: {}", exit_code)
                                },
                                timestamp: std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_millis() as u64,
                            },
                        )
                        .await;
                    sandbox_cleanup_guard.cleanup_after_success().await;
                    process_guard.disarm();
                    return Ok(build_shell_tool_result_with_metadata(
                        command_str,
                        exit_code,
                        &full_stdout,
                        &full_stderr,
                        execution_plan
                            .as_ref()
                            .and_then(|plan| serde_json::to_value(plan).ok()),
                    ));
                }
            }

            let mut got_output = false;
            tokio::select! {
                line = stdout_reader.next_line(), if !stdout_eof => {
                    match line {
                        Ok(Some(l)) => {
                            let l = stdout_sanitizer.sanitize(&format!("{l}\n"));
                            if !l.is_empty() {
                                full_stdout.push_str(&l);
                                let _ = gateway.send(session_id, GatewayPayload::ToolStream {
                                    tool_id: tool_id.to_string(),
                                    output: format_tool_stream_output(last_stream_name, "stdout", l.trim_end_matches('\n')),
                                    timestamp: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64,
                                }).await;
                                last_stream_name = Some("stdout");
                                got_output = true;
                            }
                        }
                        Ok(None) => stdout_eof = true,
                        Err(e) => log::warn!("Error reading stdout: {}", e),
                    }
                }
                line = stderr_reader.next_line(), if !stderr_eof => {
                    match line {
                        Ok(Some(l)) => {
                            let l = stderr_sanitizer.sanitize(&format!("{l}\n"));
                            if !l.is_empty()
                                && !should_suppress_incidental_termination_stderr(command_str, &l)
                            {
                                if should_collect_stderr_line_as_stdout(command_str, &l) {
                                    full_stdout.push_str(&l);
                                    let _ = gateway.send(session_id, GatewayPayload::ToolStream {
                                        tool_id: tool_id.to_string(),
                                        output: format_tool_stream_output(last_stream_name, "stdout", l.trim_end_matches('\n')),
                                        timestamp: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64,
                                    }).await;
                                    last_stream_name = Some("stdout");
                                } else {
                                    full_stderr.push_str(&l);
                                    let _ = gateway.send(session_id, GatewayPayload::ToolStream {
                                        tool_id: tool_id.to_string(),
                                        output: format_tool_stream_output(last_stream_name, "stderr", l.trim_end_matches('\n')),
                                        timestamp: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64,
                                    }).await;
                                    last_stream_name = Some("stderr");
                                }
                                got_output = true;
                            }
                        }
                        Ok(None) => stderr_eof = true,
                        Err(e) => log::warn!("Error reading stderr: {}", e),
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    if let Some(status) = child.try_wait().map_err(|e| ToolError::ExecutionFailed(format!("Failed to check process status: {}", e)))? {
                        let exit_code = status.code().unwrap_or(-1);
                        let _ = gateway.send(session_id, GatewayPayload::ToolStream {
                            tool_id: tool_id.to_string(),
                            output: if last_stream_name.is_some() { format!("\nExit code: {}", exit_code) } else { format!("Exit code: {}", exit_code) },
                            timestamp: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64,
                        }).await;
                        sandbox_cleanup_guard.cleanup_after_success().await;
                        process_guard.disarm();
                        return Ok(build_shell_tool_result_with_metadata(
                            command_str,
                            exit_code,
                            &full_stdout,
                            &full_stderr,
                            execution_plan
                                .as_ref()
                                .and_then(|plan| serde_json::to_value(plan).ok()),
                        ));
                    }
                }
            }
            if !got_output {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    }

    /// Execute a command while capturing stdout and stderr concurrently.
    async fn call_with_streaming(
        &self,
        command_str: &str,
        requested_timeout_ms: Option<u64>,
        params: Value,
    ) -> NativeToolResult {
        use std::process::Stdio;

        let configured_gateway = self.gateway.as_ref().ok_or(ToolError::ExecutionFailed(
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
        let execution_plan = self.execution_plan_for_params(&tool_id, command_str)?;
        if execution_plan.status != crate::tools::ShellExecutionPlanStatus::Ready {
            return Err(ToolError::ExecutionFailed(
                Self::execution_plan_denied_message(&execution_plan),
            ));
        }
        let timeout_ms = crate::tools::effective_timeout_ms(&execution_plan, requested_timeout_ms);
        Self::log_execution_backend_debug(&tool_id, command_str, &execution_plan);

        if let Some(sandbox_command) =
            crate::tools::sandbox_command_for_plan(&execution_plan, command_str)?
        {
            return self
                .call_with_streaming_command(
                    command_str,
                    timeout_ms,
                    sandbox_command,
                    configured_gateway.as_ref(),
                    session_id,
                    &tool_id,
                    Some(execution_plan.clone()),
                )
                .await;
        }

        let gateway = ResultOnlyShellGateway::new(configured_gateway.as_ref());
        let execution_plan_metadata = serde_json::to_value(&execution_plan).ok();
        // Execute host compound commands through the platform shell as one command.
        // `parse_safe_compound_command` remains available for policy/output analysis,
        // but AC-9 requires backend execution to preserve original shell semantics.

        let host_command = crate::libs::ai_temp::map_ai_temp_paths_for_host_command(command_str);
        let mut child = if cfg!(target_os = "windows") {
            let mut command = Command::new("cmd");
            command
                .args(["/C", &host_command])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            configure_no_window(&mut command);
            if let Some(dir) = &working_dir {
                command.current_dir(dir);
            }
            command
                .spawn()
                .map_err(|e| ToolError::ExecutionFailed(format!("Failed to spawn: {}", e)))?
        } else {
            let mut command = Command::new("sh");
            command
                .args(["-c", &host_command])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            configure_process_group(&mut command);
            if let Some(dir) = &working_dir {
                command.current_dir(dir);
            }
            command
                .spawn()
                .map_err(|e| ToolError::ExecutionFailed(format!("Failed to spawn: {}", e)))?
        };
        let mut process_guard = StageProcessGuard::new(&child);

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
                terminate_stage_process(&mut child, &mut process_guard).await;
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
                        process_guard.disarm();
                        return Ok(build_shell_tool_result_with_metadata(
                            command_str,
                            exit_code,
                            &full_stdout,
                            &full_stderr,
                            execution_plan_metadata.clone(),
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
                            if l.is_empty()
                                || should_suppress_incidental_termination_stderr(command_str, &l)
                            {
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
                            process_guard.disarm();
                            return Ok(build_shell_tool_result_with_metadata(
                                command_str,
                                exit_code,
                                &full_stdout,
                                &full_stderr,
                                execution_plan_metadata.clone(),
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
    use crate::tools::helper::parse_safe_compound_command;
    use crate::tools::shell_output::{strip_ansi_escape_sequences, AnsiOutputSanitizer};
    use crate::tools::{
        SandboxFailureReason, SandboxMountPlan, SandboxNetworkPolicy, SandboxResourceLimits,
        SandboxRuntime, ShellExecutionBackendKind, ShellExecutionPlan, ShellExecutionPlanStatus,
        ShellExecutionRiskFloor, WorkspaceAccess,
    };
    use crate::workflow::react::security::PathGuard;
    use std::process::Stdio;
    use std::sync::{Mutex, OnceLock};
    use tempfile::tempdir;

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    #[test]
    fn shell_decisions_round_trip_as_policy_strings() {
        let rules = vec![
            ShellPolicyRule {
                pattern: "^git status$".to_string(),
                decision: ShellDecision::Review("repository state".to_string()),
                description: None,
            },
            ShellPolicyRule {
                pattern: "^rm ".to_string(),
                decision: ShellDecision::Deny("destructive command".to_string()),
                description: None,
            },
        ];

        let json = serde_json::to_string(&rules).expect("failed to serialize shell policy");
        assert!(json.contains(r#""decision":"review:repository state""#));
        assert!(json.contains(r#""decision":"deny:destructive command""#));

        let decoded: Vec<ShellPolicyRule> =
            serde_json::from_str(&json).expect("failed to deserialize shell policy");
        assert_eq!(decoded[0].decision, rules[0].decision);
        assert_eq!(decoded[1].decision, rules[1].decision);
    }

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
        for command in [
            "sudo rm -rf /",
            "su -c 'id'",
            "chsh -s /bin/sh",
            "newgrp staff",
            "sg staff -c id",
        ] {
            assert!(
                matches!(engine.check(command, false), ShellDecision::Deny(_)),
                "expected hard denial for {command}"
            );
        }
        assert!(matches!(
            engine.check("rm -rf test", false),
            ShellDecision::Review(_)
        ));
    }

    #[test]
    fn test_policy_engine_custom_rules_preserve_path_boundaries() {
        let (_root, _, guard) = setup_test_context();
        let outside_root = tempdir().unwrap();
        let outside_file = outside_root.path().join("outside.txt");
        std::fs::write(&outside_file, "outside").unwrap();
        let engine = ShellPolicyEngine::new(
            guard,
            vec![ShellPolicyRule {
                pattern: "^cat .*".to_string(),
                decision: ShellDecision::Allow,
                description: None,
            }],
        );

        assert!(matches!(
            engine.check(&format!("cat {}", outside_file.display()), false),
            ShellDecision::Deny(_)
        ));
    }

    #[test]
    fn test_policy_engine_audits_wrapped_and_indirect_execution() {
        let (_root, _, guard) = setup_test_context();
        let engine = ShellPolicyEngine::new(
            guard,
            vec![ShellPolicyRule {
                pattern: "^(?:env|time|watch|find|xargs|command)(?:$| .*)".to_string(),
                decision: ShellDecision::Allow,
                description: None,
            }],
        );

        for command in [
            "time sudo rm -rf /",
            "timeout 5 su -c 'id'",
            "watch -n 5 'sudo id'",
            "env -S 'sudo id'",
            "env -uPATH sudo id",
            "env --unset=PATH sudo id",
            "find . -exec sudo id {} \\;",
            "printf x | xargs sudo id",
            "command sudo id",
            "awk 'BEGIN { system(\"sudo id\") }'",
            "awk 'BEGIN { \"sudo id\" | getline }'",
        ] {
            assert!(
                matches!(engine.check(command, false), ShellDecision::Deny(_)),
                "expected wrapped hard denial for {command}"
            );
        }

        for command in [
            "time pnpm tauri dev",
            "watch -n 5 'ls -l'",
            "env CI=1 cargo check",
            "find . -exec ls {} \\;",
            "printf x | xargs rm",
        ] {
            assert!(
                matches!(engine.check(command, false), ShellDecision::Review(_)),
                "expected execution audit for {command}"
            );
        }
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

        // Use a temporary authorized directory so the test is portable.
        let temp_root = tempdir().unwrap();
        let authorized_root = temp_root.path().canonicalize().unwrap();
        for relative in [
            "broadcast/src/common/account_manager.rs",
            "broadcast/src/main.rs",
            "broadcast/src/server.rs",
        ] {
            let path = authorized_root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "// test").unwrap();
        }
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
            vec![
                ShellPolicyRule {
                    pattern: "^git diff($| .*)".to_string(),
                    decision: ShellDecision::Allow,
                    description: None,
                },
                ShellPolicyRule {
                    pattern: "^head($| .*)".to_string(),
                    decision: ShellDecision::Allow,
                    description: None,
                },
            ],
        );

        let result = engine.check("cd . && git diff src/main.rs | head -80", false);

        assert_eq!(result, ShellDecision::Allow);
    }

    #[test]
    fn test_policy_engine_does_not_allow_unmatched_mutations_after_allowed_git_diff() {
        let temp_root = tempdir().unwrap();
        let project_root = temp_root.path().canonicalize().unwrap();
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

        let result = engine.check(
            "git diff --check && git add -- src/lib.rs && git commit -m 'unsafe'",
            false,
        );

        assert!(matches!(result, ShellDecision::Review(_)));
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
            vec![
                ShellPolicyRule {
                    pattern: "^cargo check($| .*)".to_string(),
                    decision: ShellDecision::Allow,
                    description: None,
                },
                ShellPolicyRule {
                    pattern: "^tail($| .*)".to_string(),
                    decision: ShellDecision::Allow,
                    description: None,
                },
            ],
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

    fn sandbox_test_plan(
        backend: ShellExecutionBackendKind,
        runtime: SandboxRuntime,
        project_root: &Path,
    ) -> ShellExecutionPlan {
        ShellExecutionPlan {
            tool_call_id: format!("{runtime:?}-stream-test"),
            command: "printf 'out\\n'; printf 'err\\n' >&2".to_string(),
            scheme_id: None,
            scheme_revision: None,
            backend,
            backend_origin: Default::default(),
            runtime: Some(runtime),
            profile: Some("busybox".to_string()),
            image: Some("busybox:latest".to_string()),
            instance_name: None,
            network: Some(SandboxNetworkPolicy::default()),
            resources: Some(SandboxResourceLimits::default()),
            workspace_access: Some(WorkspaceAccess::ReadOnly),
            mounts: vec![SandboxMountPlan {
                host_path: project_root.display().to_string(),
                guest_path: "/workspace".to_string(),
                access: WorkspaceAccess::ReadOnly,
            }],
            workdir: Some("/workspace".to_string()),
            fallback_reason: None,
            risk_floor: ShellExecutionRiskFloor::Normal,
            status: ShellExecutionPlanStatus::Ready,
        }
    }

    fn local_streaming_test_command() -> Command {
        let mut command = Command::new("sh");
        command
            .args(["-c", "printf 'out\\n'; printf 'err\\n' >&2"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        command
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
        let shell = test_shell_execute(project_root.clone());
        let plan = parse_safe_compound_command("cd nested && /bin/pwd && git status --short")
            .expect("command should be syntactically safe");
        let result = shell
            .call_safe_compound(plan, 10_000, Some(project_root), None)
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
    async fn host_shell_call_executes_compound_command_as_single_unit() {
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
        assert!(
            structured.get("stages").is_none(),
            "host ShellExecute::call must not split compound commands into stage execution"
        );
        let llm_content = structured["llm_content"]
            .as_str()
            .expect("llm content missing");
        assert!(llm_content.contains(nested.to_string_lossy().as_ref()));
        assert_eq!(
            structured["execution_plan"]["backend"].as_str(),
            Some("host")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn safe_compound_execution_short_circuits_after_failure() {
        let temp_root = tempdir().unwrap();
        let project_root = temp_root.path().canonicalize().unwrap();
        initialize_test_git_repository(&project_root);
        let shell = test_shell_execute(project_root.clone());

        let plan =
            parse_safe_compound_command("/usr/bin/false && git status && /usr/bin/touch marker")
                .expect("command should be syntactically safe");
        let result = shell
            .call_safe_compound(plan, 10_000, Some(project_root.clone()), None)
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
    async fn safe_compound_with_gateway_returns_result_without_stream_events() {
        let temp_root = tempdir().unwrap();
        let project_root = temp_root.path().canonicalize().unwrap();
        initialize_test_git_repository(&project_root);
        let gateway = Arc::new(RecordingGateway::default());
        let shell = test_shell_execute(project_root.clone())
            .with_gateway(gateway.clone(), "test-session".to_string());

        let plan =
            parse_safe_compound_command("/bin/echo first && /usr/bin/false && git status --short")
                .expect("command should be syntactically safe");
        let result = shell
            .call_safe_compound_streaming(
                plan,
                10_000,
                Some(project_root.clone()),
                gateway.as_ref(),
                "test-session",
                "tool-safe-compound",
                None,
            )
            .await
            .unwrap();
        let structured = result
            .structured_content
            .expect("structured content missing");
        assert_eq!(structured["exit_code"].as_i64(), Some(1));
        assert_eq!(structured["stages"][2]["executed"].as_bool(), Some(false));

        let streams = gateway.streams.lock().unwrap().clone();
        assert!(
            streams
                .iter()
                .any(|(_, output)| output.contains("Stage 1:")),
            "direct safe compound streaming helper should emit stage stream events"
        );

        let model_path = structured["persisted_output"]["path"]
            .as_str()
            .expect("persisted path missing");
        let physical_path = crate::libs::ai_temp::resolve_ai_temp_path(Path::new(model_path));
        std::fs::remove_file(physical_path).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn simple_command_with_gateway_returns_full_result_without_stream_events() {
        let temp_root = tempdir().unwrap();
        let project_root = temp_root.path().canonicalize().unwrap();
        let gateway = Arc::new(RecordingGateway::default());
        let shell = test_shell_execute(project_root)
            .with_gateway(gateway.clone(), "test-session".to_string());

        let result = shell
            .call(json!({
                "command": "/usr/bin/printf 'first\\nsecond\\n'",
                "timeout": 10_000,
                crate::constants::INTERNAL_PARAM_TOOL_CALL_ID: "tool-simple-command",
            }))
            .await
            .expect("simple command should complete");
        let structured = result
            .structured_content
            .expect("structured content missing");

        assert_eq!(structured["exit_code"].as_i64(), Some(0));
        let llm_content = structured["llm_content"]
            .as_str()
            .expect("llm content missing");
        assert!(llm_content.contains("first\nsecond"));
        assert!(
            gateway.streams.lock().unwrap().is_empty(),
            "bash must not emit live stream events for simple commands"
        );
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

        let plan = parse_safe_compound_command("sh spawn-child.sh && git status --short")
            .expect("command should be syntactically safe");
        let result = shell
            .call_safe_compound(plan, 150, Some(project_root.clone()), None)
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
    async fn msb_streaming_command_delivers_live_stdout_stderr_and_terminal_result() {
        let temp_root = tempdir().unwrap();
        let project_root = temp_root.path().canonicalize().unwrap();
        let gateway = RecordingGateway::default();
        let shell = test_shell_execute(project_root.clone());
        let plan = sandbox_test_plan(
            ShellExecutionBackendKind::Msb,
            SandboxRuntime::Msb,
            &project_root,
        );

        let command_str = plan.command.clone();
        let result = shell
            .call_with_streaming_command(
                &command_str,
                10_000,
                local_streaming_test_command(),
                &gateway,
                "test-session",
                "tool-msb-stream",
                Some(plan),
            )
            .await
            .unwrap();

        let streams = gateway.streams.lock().unwrap().clone();
        assert!(
            streams.iter().any(|(_, output)| output.contains("out")),
            "{streams:?}"
        );
        assert!(
            streams
                .iter()
                .any(|(_, output)| output.contains("stderr:") && output.contains("err")),
            "{streams:?}"
        );
        assert!(
            streams
                .iter()
                .any(|(_, output)| output.contains("Exit code: 0")),
            "{streams:?}"
        );
        let structured = result
            .structured_content
            .expect("structured content missing");
        assert_eq!(structured["exit_code"].as_i64(), Some(0));
        assert_eq!(
            structured["execution_plan"]["backend"].as_str(),
            Some("msb")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn docker_streaming_command_delivers_live_stdout_stderr_and_terminal_result() {
        let temp_root = tempdir().unwrap();
        let project_root = temp_root.path().canonicalize().unwrap();
        let gateway = RecordingGateway::default();
        let shell = test_shell_execute(project_root.clone());
        let plan = sandbox_test_plan(
            ShellExecutionBackendKind::Docker,
            SandboxRuntime::Docker,
            &project_root,
        );

        let command_str = plan.command.clone();
        let result = shell
            .call_with_streaming_command(
                &command_str,
                10_000,
                local_streaming_test_command(),
                &gateway,
                "test-session",
                "tool-docker-stream",
                Some(plan),
            )
            .await
            .unwrap();

        let streams = gateway.streams.lock().unwrap().clone();
        assert!(
            streams.iter().any(|(_, output)| output.contains("out")),
            "{streams:?}"
        );
        assert!(
            streams
                .iter()
                .any(|(_, output)| output.contains("stderr:") && output.contains("err")),
            "{streams:?}"
        );
        assert!(
            streams
                .iter()
                .any(|(_, output)| output.contains("Exit code: 0")),
            "{streams:?}"
        );
        let structured = result
            .structured_content
            .expect("structured content missing");
        assert_eq!(structured["exit_code"].as_i64(), Some(0));
        assert_eq!(
            structured["execution_plan"]["backend"].as_str(),
            Some("docker")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn sandbox_streaming_spawn_failure_is_structured_with_execution_plan() {
        let temp_root = tempdir().unwrap();
        let project_root = temp_root.path().canonicalize().unwrap();
        let gateway = RecordingGateway::default();
        let shell = test_shell_execute(project_root.clone());
        let plan = sandbox_test_plan(
            ShellExecutionBackendKind::Msb,
            SandboxRuntime::Msb,
            &project_root,
        );
        let command_str = plan.command.clone();
        let mut command = Command::new("/definitely/missing/chatspeed-sandbox-runner");
        command.stdout(Stdio::piped()).stderr(Stdio::piped());

        let result = shell
            .call_with_streaming_command(
                &command_str,
                10_000,
                command,
                &gateway,
                "test-session",
                "tool-msb-spawn-failure",
                Some(plan),
            )
            .await;

        assert!(matches!(
            result,
            Err(ToolError::SandboxFailure(failure))
                if failure.reason == SandboxFailureReason::SpawnFailed
                    && failure.backend == ShellExecutionBackendKind::Msb
                    && failure.execution_plan
                        .as_ref()
                        .and_then(|plan| plan.get("backend"))
                        .and_then(|value| value.as_str()) == Some("msb")
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    #[ignore = "mutates process-global PATH; cleanup argv is covered by runner tests"]
    async fn sandbox_timeout_invokes_docker_cleanup_for_named_container() {
        let _env_guard = ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp_root = tempdir().unwrap();
        let bin_dir = temp_root.path().join("bin");
        std::fs::create_dir(&bin_dir).unwrap();
        let log_path = temp_root.path().join("docker.log");
        let docker_path = bin_dir.join("docker");
        std::fs::write(
            &docker_path,
            format!(
                "#!/bin/sh\necho \"$@\" >> {}\nif [ \"$1\" = \"run\" ]; then sleep 5; fi\nexit 0\n",
                log_path.display()
            ),
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&docker_path).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&docker_path, permissions).unwrap();
        }
        let original_path = std::env::var_os("PATH");
        std::env::set_var(
            "PATH",
            format!(
                "{}:{}",
                bin_dir.display(),
                original_path
                    .as_ref()
                    .map(|value| value.to_string_lossy())
                    .unwrap_or_default()
            ),
        );

        let plan = ShellExecutionPlan {
            tool_call_id: "cleanup-test".to_string(),
            command: "sleep 5".to_string(),
            scheme_id: None,
            scheme_revision: None,
            backend: ShellExecutionBackendKind::Docker,
            backend_origin: Default::default(),
            runtime: Some(SandboxRuntime::Docker),
            profile: Some("busybox".to_string()),
            image: Some("busybox:latest".to_string()),
            instance_name: None,
            network: Some(SandboxNetworkPolicy::default()),
            resources: Some(SandboxResourceLimits::default()),
            workspace_access: Some(WorkspaceAccess::ReadOnly),
            mounts: vec![SandboxMountPlan {
                host_path: temp_root.path().display().to_string(),
                guest_path: "/workspace".to_string(),
                access: WorkspaceAccess::ReadOnly,
            }],
            workdir: Some("/workspace".to_string()),
            fallback_reason: None,
            risk_floor: ShellExecutionRiskFloor::Normal,
            status: ShellExecutionPlanStatus::Ready,
        };
        let command = crate::tools::sandbox_command_for_plan(&plan, "sleep 5")
            .unwrap()
            .unwrap();
        let result = run_sandbox_output_with_timeout(command, &plan, 50).await;

        if let Some(path) = original_path {
            std::env::set_var("PATH", path);
        } else {
            std::env::remove_var("PATH");
        }
        assert!(matches!(
            result,
            Err(ToolError::SandboxFailure(failure))
                if failure.backend == ShellExecutionBackendKind::Docker
                    && failure.reason == SandboxFailureReason::TimedOut
                    && failure.message.contains("timed out after 50ms")
        ));
        let log = std::fs::read_to_string(&log_path).unwrap();
        assert!(
            log.contains("run --rm --name chatspeed-shell-cleanup-test"),
            "{log}"
        );
        assert!(log.contains("rm -f chatspeed-shell-cleanup-test"), "{log}");
    }

    #[cfg(unix)]
    #[tokio::test]
    #[ignore = "mutates process-global PATH; Drop cleanup argv is covered by runner tests"]
    async fn sandbox_output_future_drop_still_schedules_cleanup() {
        let _env_guard = ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp_root = tempdir().unwrap();
        let bin_dir = temp_root.path().join("bin");
        std::fs::create_dir(&bin_dir).unwrap();
        let log_path = temp_root.path().join("docker-drop.log");
        let docker_path = bin_dir.join("docker");
        std::fs::write(
            &docker_path,
            format!(
                "#!/bin/sh\necho \"$@\" >> {}\nif [ \"$1\" = \"run\" ]; then sleep 5; fi\nexit 0\n",
                log_path.display()
            ),
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&docker_path).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&docker_path, permissions).unwrap();
        }
        let original_path = std::env::var_os("PATH");
        std::env::set_var(
            "PATH",
            format!(
                "{}:{}",
                bin_dir.display(),
                original_path
                    .as_ref()
                    .map(|value| value.to_string_lossy())
                    .unwrap_or_default()
            ),
        );
        let plan = ShellExecutionPlan {
            tool_call_id: "drop-cleanup-test".to_string(),
            command: "sleep 5".to_string(),
            scheme_id: None,
            scheme_revision: None,
            backend: ShellExecutionBackendKind::Docker,
            backend_origin: Default::default(),
            runtime: Some(SandboxRuntime::Docker),
            profile: Some("busybox".to_string()),
            image: Some("busybox:latest".to_string()),
            instance_name: None,
            network: Some(SandboxNetworkPolicy::default()),
            resources: Some(SandboxResourceLimits::default()),
            workspace_access: Some(WorkspaceAccess::ReadOnly),
            mounts: vec![SandboxMountPlan {
                host_path: temp_root.path().display().to_string(),
                guest_path: "/workspace".to_string(),
                access: WorkspaceAccess::ReadOnly,
            }],
            workdir: Some("/workspace".to_string()),
            fallback_reason: None,
            risk_floor: ShellExecutionRiskFloor::Normal,
            status: ShellExecutionPlanStatus::Ready,
        };
        let command = crate::tools::sandbox_command_for_plan(&plan, "sleep 5")
            .unwrap()
            .unwrap();
        drop(command);
        let cleanup_guard = SandboxCleanupGuard::new(Some(plan));
        drop(cleanup_guard);
        tokio::time::sleep(Duration::from_millis(100)).await;
        if let Some(path) = original_path {
            std::env::set_var("PATH", path);
        } else {
            std::env::remove_var("PATH");
        }
        let log = std::fs::read_to_string(&log_path).unwrap_or_default();
        assert!(
            log.contains("run --rm --name chatspeed-shell-drop-cleanup-test"),
            "{log}"
        );
        assert!(
            log.contains("rm -f chatspeed-shell-drop-cleanup-test"),
            "{log}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn successful_msb_output_invokes_cleanup_for_named_instance() {
        let _env_guard = ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp_root = tempdir().unwrap();
        let bin_dir = temp_root.path().join("bin");
        std::fs::create_dir(&bin_dir).unwrap();
        let log_path = temp_root.path().join("msb-success-cleanup.log");
        let msb_path = bin_dir.join("msb");
        std::fs::write(
            &msb_path,
            format!(
                "#!/bin/sh\necho \"$@\" >> {}\nif [ \"$1\" = \"run\" ]; then echo ok; exit 0; fi\nexit 0\n",
                log_path.display()
            ),
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&msb_path).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&msb_path, permissions).unwrap();
        }
        let original_path = std::env::var_os("PATH");
        std::env::set_var(
            "PATH",
            format!(
                "{}:{}",
                bin_dir.display(),
                original_path
                    .as_ref()
                    .map(|value| value.to_string_lossy())
                    .unwrap_or_default()
            ),
        );
        let plan = ShellExecutionPlan {
            tool_call_id: "msb-success-cleanup-test".to_string(),
            command: "echo ok".to_string(),
            scheme_id: None,
            scheme_revision: None,
            backend: ShellExecutionBackendKind::Msb,
            backend_origin: Default::default(),
            runtime: Some(SandboxRuntime::Msb),
            profile: Some("busybox".to_string()),
            image: Some("busybox:latest".to_string()),
            instance_name: None,
            network: Some(SandboxNetworkPolicy::default()),
            resources: Some(SandboxResourceLimits::default()),
            workspace_access: Some(WorkspaceAccess::ReadOnly),
            mounts: vec![SandboxMountPlan {
                host_path: temp_root.path().display().to_string(),
                guest_path: "/workspace".to_string(),
                access: WorkspaceAccess::ReadOnly,
            }],
            workdir: Some("/workspace".to_string()),
            fallback_reason: None,
            risk_floor: ShellExecutionRiskFloor::Normal,
            status: ShellExecutionPlanStatus::Ready,
        };
        let command = crate::tools::sandbox_command_for_plan(&plan, "echo ok")
            .unwrap()
            .unwrap();
        let result = run_sandbox_output_with_timeout(command, &plan, 5_000).await;

        if let Some(path) = original_path {
            std::env::set_var("PATH", path);
        } else {
            std::env::remove_var("PATH");
        }
        assert!(result.is_ok());
        let log = std::fs::read_to_string(&log_path).unwrap();
        assert!(
            log.contains("run --quiet --no-tty --name chatspeed-shell-msb-success-cleanup-test"),
            "{log}"
        );
        assert!(
            log.contains("remove --force --quiet chatspeed-shell-msb-success-cleanup-test"),
            "{log}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn simple_command_timeout_kills_the_process_group() {
        let temp_root = tempdir().unwrap();
        let project_root = temp_root.path().canonicalize().unwrap();
        write_long_running_child_script(&project_root);
        let gateway = Arc::new(RecordingGateway::default());
        let shell = test_shell_execute(project_root.clone())
            .with_gateway(gateway, "test-session".to_string());

        let result = shell
            .call(json!({
                "command": "sh spawn-child.sh",
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
    async fn host_command_future_drop_kills_the_process_group_without_gateway() {
        let temp_root = tempdir().unwrap();
        let project_root = temp_root.path().canonicalize().unwrap();
        write_long_running_child_script(&project_root);
        let shell = test_shell_execute(project_root.clone());

        let task = tokio::spawn(async move {
            shell
                .call(json!({
                    "command": "sh spawn-child.sh",
                    "timeout": 30_000,
                }))
                .await
        });

        let pid_path = project_root.join("child.pid");
        for _ in 0..100 {
            if pid_path.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(pid_path.exists(), "child process did not start");

        task.abort();
        let _ = task.await;
        assert_recorded_child_stopped(&project_root).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn streaming_command_future_drop_kills_the_process_group() {
        let temp_root = tempdir().unwrap();
        let project_root = temp_root.path().canonicalize().unwrap();
        write_long_running_child_script(&project_root);
        let gateway = Arc::new(RecordingGateway::default());
        let shell = test_shell_execute(project_root.clone())
            .with_gateway(gateway, "test-session".to_string());

        let task = tokio::spawn(async move {
            shell
                .call(json!({
                    "command": "sh spawn-child.sh",
                    "timeout": 30_000,
                }))
                .await
        });

        let pid_path = project_root.join("child.pid");
        for _ in 0..100 {
            if pid_path.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(pid_path.exists(), "child process did not start");

        task.abort();
        let _ = task.await;
        assert_recorded_child_stopped(&project_root).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn safe_compound_execution_uses_one_total_timeout_budget() {
        let temp_root = tempdir().unwrap();
        let project_root = temp_root.path().canonicalize().unwrap();
        initialize_test_git_repository(&project_root);
        let shell = test_shell_execute(project_root.clone());
        let plan =
            parse_safe_compound_command("/bin/sleep 0.08 && /bin/sleep 0.08 && git status --short")
                .expect("command should be syntactically safe");
        let started = Instant::now();

        let result = shell
            .call_safe_compound(plan, 120, Some(project_root), None)
            .await;

        assert!(
            matches!(result, Err(ToolError::ExecutionFailed(message)) if message.contains("timed out after 120ms"))
        );
        assert!(started.elapsed() < Duration::from_millis(400));
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
            .call_safe_compound(plan, 10_000, Some(project_root), None)
            .await;

        assert!(matches!(result, Err(ToolError::Security(_))));
        drop(outside_root);
    }

    #[test]
    fn denied_execution_plan_message_does_not_leak_option_debug_format() {
        let temp_root = tempdir().unwrap();
        let project_root = temp_root.path().canonicalize().unwrap();
        let mut plan = sandbox_test_plan(
            ShellExecutionBackendKind::Msb,
            SandboxRuntime::Msb,
            &project_root,
        );
        plan.status = ShellExecutionPlanStatus::Denied;
        plan.fallback_reason = Some(crate::tools::sandbox::HostFallbackReason::ProfileUnavailable);

        assert_eq!(
            ShellExecute::execution_plan_denied_message(&plan),
            "Sandbox execution is not authorized before spawn: Denied (ProfileUnavailable)"
        );
    }

    #[test]
    fn approved_execution_plan_is_server_owned_and_one_time() {
        let temp_root = tempdir().unwrap();
        let project_root = temp_root.path().canonicalize().unwrap();
        let shell = test_shell_execute(project_root.clone());
        let plan = sandbox_test_plan(
            ShellExecutionBackendKind::Msb,
            SandboxRuntime::Msb,
            &project_root,
        );
        let tool_call_id = plan.tool_call_id.clone();

        shell
            .approved_execution_plans
            .insert(tool_call_id.clone(), plan.clone());

        assert_eq!(
            shell.approved_shell_execution_plan(&tool_call_id),
            Some(plan)
        );
        assert!(shell.approved_shell_execution_plan(&tool_call_id).is_none());
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
