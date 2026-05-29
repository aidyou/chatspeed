#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ShellStage {
    Navigation {
        command: String,
        target: Option<String>,
    },
    Command {
        normalized: String,
        tokens: Vec<String>,
    },
}

fn strip_benign_shell_redirection(command: &str) -> String {
    command
        .replace("2>&1", " ")
        .replace("1>&2", " ")
        .replace("2>/dev/null", " ")
        .replace("2> /dev/null", " ")
        .replace("1>/dev/null", " ")
        .replace("1> /dev/null", " ")
        .replace("&>/dev/null", " ")
}

pub(crate) fn split_shell_command_segments(command: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut chars = command.chars().peekable();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escaped = false;

    while let Some(ch) = chars.next() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }

        if ch == '\\' && !in_single_quote {
            current.push(ch);
            escaped = true;
            continue;
        }

        if ch == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
            current.push(ch);
            continue;
        }

        if ch == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
            current.push(ch);
            continue;
        }

        if !in_single_quote && !in_double_quote {
            match ch {
                ';' => {
                    if !current.trim().is_empty() {
                        segments.push(current.trim().to_string());
                    }
                    current.clear();
                    continue;
                }
                '&' | '|' => {
                    if chars.peek() == Some(&ch) {
                        let _ = chars.next();
                        if !current.trim().is_empty() {
                            segments.push(current.trim().to_string());
                        }
                        current.clear();
                        continue;
                    }
                }
                _ => {}
            }
        }

        current.push(ch);
    }

    if !current.trim().is_empty() {
        segments.push(current.trim().to_string());
    }

    segments
}

pub(crate) fn contains_unquoted_shell_redirection(command: &str) -> bool {
    let mut chars = command.chars().peekable();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escaped = false;

    while let Some(ch) = chars.next() {
        if escaped {
            escaped = false;
            continue;
        }

        if ch == '\\' && !in_single_quote {
            escaped = true;
            continue;
        }

        if ch == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
            continue;
        }

        if ch == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
            continue;
        }

        if !in_single_quote && !in_double_quote && matches!(ch, '>' | '<') {
            return true;
        }
    }

    false
}

pub(crate) fn shell_tokens(command: &str) -> Option<Vec<String>> {
    shlex::split(command.trim())
}

pub(crate) fn leading_command_index(tokens: &[String]) -> usize {
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index].as_str();
        if token.contains('=') && !token.starts_with('=') && !token.ends_with('=') {
            let mut parts = token.splitn(2, '=');
            let key = parts.next().unwrap_or("");
            if !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                index += 1;
                continue;
            }
        }
        break;
    }

    index
}

pub(crate) fn classify_shell_stage(command: &str) -> Option<ShellStage> {
    let sanitized = strip_benign_shell_redirection(command);
    let trimmed = sanitized.trim();
    if trimmed.is_empty() || trimmed.contains('\n') {
        return None;
    }
    if contains_unquoted_shell_redirection(trimmed) {
        return None;
    }

    let tokens = shell_tokens(trimmed)?;
    if tokens.is_empty() {
        return None;
    }

    let index = leading_command_index(&tokens);
    if index >= tokens.len() {
        return None;
    }

    let base = tokens[index].as_str();
    if matches!(base, "cd" | "pushd" | "popd") {
        return Some(ShellStage::Navigation {
            command: base.to_string(),
            target: tokens.get(index + 1).cloned(),
        });
    }

    let args = &tokens[index + 1..];
    let inline_script_runners = [
        "python", "python3", "node", "bash", "sh", "zsh", "ruby", "perl",
    ];
    if inline_script_runners.contains(&base)
        && args
            .iter()
            .any(|arg| matches!(arg.as_str(), "-c" | "-e" | "-"))
    {
        return None;
    }

    let normalized_tokens = tokens[index..].to_vec();
    Some(ShellStage::Command {
        normalized: normalized_tokens.join(" "),
        tokens: normalized_tokens,
    })
}

pub(crate) fn approvable_shell_pattern_for_stage(stage_tokens: &[String]) -> Option<String> {
    if stage_tokens.is_empty() {
        return None;
    }

    let base_cmd = stage_tokens[0].as_str();

    let destructive_commands = [
        "rm",
        "rmdir",
        "del",
        "mv",
        "move",
        "chmod",
        "chown",
        "chgrp",
        "kill",
        "killall",
        "pkill",
        "ln",
        "crontab",
        "curl",
        "wget",
        "apt",
        "apt-get",
        "yum",
        "dnf",
        "pacman",
        "brew",
        "docker",
        "podman",
        "systemctl",
        "service",
    ];
    if destructive_commands.contains(&base_cmd) {
        return None;
    }

    let simple_safe_commands = [
        "ls", "dir", "tree", "pwd", "whoami", "id", "groups", "hostname", "date", "uptime",
        "uname", "arch", "env", "printenv", "ps", "df", "free", "mount", "lsblk", "cat", "head",
        "tail", "less", "more", "stat", "file", "wc", "du",
    ];
    if simple_safe_commands.contains(&base_cmd) {
        return Some(format!("^{}($| .*)", regex::escape(base_cmd)));
    }

    let allow_base_with_subcommand = ["git", "cargo", "go", "pnpm", "npm", "yarn"];
    if allow_base_with_subcommand.contains(&base_cmd) {
        let subcommand = stage_tokens
            .iter()
            .skip(1)
            .find(|token| !token.starts_with('-'))
            .map(|token| token.as_str());
        if let Some(subcommand) = subcommand {
            return Some(format!(
                "^{} {}($| .*)",
                regex::escape(base_cmd),
                regex::escape(subcommand)
            ));
        }
        return Some(format!("^{}($| .*)", regex::escape(base_cmd)));
    }

    None
}

pub(crate) fn generate_shell_approval_patterns(command: &str) -> Vec<String> {
    let mut patterns = Vec::new();
    for segment in split_shell_command_segments(command) {
        let Some(ShellStage::Command { tokens, .. }) = classify_shell_stage(&segment) else {
            continue;
        };

        if let Some(pattern) = approvable_shell_pattern_for_stage(&tokens) {
            if !patterns.iter().any(|existing| existing == &pattern) {
                patterns.push(pattern);
            }
        }
    }
    patterns
}

#[cfg(test)]
mod tests {
    use super::{classify_shell_stage, generate_shell_approval_patterns, ShellStage};

    #[test]
    fn classify_shell_stage_ignores_benign_stream_merge_and_tail_filter() {
        let stage = classify_shell_stage(
            "cargo check --manifest-path src-tauri/Cargo.toml 2>&1 | tail -10",
        );

        assert!(matches!(
            stage,
            Some(ShellStage::Command { normalized, .. })
            if normalized.starts_with("cargo check --manifest-path src-tauri/Cargo.toml")
        ));
    }

    #[test]
    fn generate_shell_approval_patterns_keeps_primary_command_with_benign_stream_merge() {
        let patterns = generate_shell_approval_patterns(
            "cd . && cargo check --manifest-path src-tauri/Cargo.toml 2>&1 | tail -10",
        );

        assert_eq!(patterns, vec!["^cargo check($| .*)".to_string()]);
    }
}
