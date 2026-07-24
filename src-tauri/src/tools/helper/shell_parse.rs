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

pub(crate) fn contains_unquoted_shell_operator(command: &str) -> bool {
    let mut chars = command.chars();
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
        if !in_single_quote && !in_double_quote && matches!(ch, ';' | '&' | '|' | '\n') {
            return true;
        }
    }

    false
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SafeCompoundStage {
    Navigation {
        original: String,
        target: String,
    },
    Command {
        original: String,
        normalized: String,
        tokens: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SafeCompoundCommand {
    pub stages: Vec<SafeCompoundStage>,
}

const MAX_SAFE_COMPOUND_STAGES: usize = 16;

fn split_safe_and_then_stages(command: &str) -> Option<Vec<String>> {
    let mut stages = Vec::new();
    let mut stage_start = 0;
    let mut index = 0;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escaped = false;

    while index < command.len() {
        let character = command[index..].chars().next()?;
        let character_len = character.len_utf8();
        if character == '\n' || character == '\r' {
            return None;
        }
        if escaped {
            escaped = false;
            index += character_len;
            continue;
        }
        if character == '\\' && !in_single_quote {
            escaped = true;
            index += character_len;
            continue;
        }
        if character == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
            index += character_len;
            continue;
        }
        if character == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
            index += character_len;
            continue;
        }
        if !in_single_quote && !in_double_quote && command[index..].starts_with("&&") {
            let stage = command[stage_start..index].trim();
            if stage.is_empty() {
                return None;
            }
            stages.push(stage.to_string());
            if stages.len() >= MAX_SAFE_COMPOUND_STAGES {
                return None;
            }
            index += 2;
            stage_start = index;
            continue;
        }
        index += character_len;
    }

    if escaped || in_single_quote || in_double_quote {
        return None;
    }
    let stage = command[stage_start..].trim();
    if stage.is_empty() || stages.is_empty() {
        return None;
    }
    stages.push(stage.to_string());
    (stages.len() <= MAX_SAFE_COMPOUND_STAGES).then_some(stages)
}

fn strip_safe_stage_redirections(command: &str) -> Option<String> {
    const ALLOWED_REDIRECTIONS: [&str; 7] = [
        "2> /dev/null",
        "1> /dev/null",
        "&>/dev/null",
        "2>/dev/null",
        "1>/dev/null",
        "2>&1",
        "1>&2",
    ];

    let mut sanitized = String::with_capacity(command.len());
    let mut index = 0;
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while index < command.len() {
        let character = command[index..].chars().next()?;
        let character_len = character.len_utf8();
        if character == '\n' || character == '\r' {
            return None;
        }
        if character == '\\' && !in_single_quote {
            let next_index = index + character_len;
            let next = command[next_index..].chars().next()?;
            if next == '\n' || next == '\r' {
                return None;
            }
            sanitized.push(character);
            sanitized.push(next);
            index = next_index + next.len_utf8();
            continue;
        }
        if character == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
            sanitized.push(character);
            index += character_len;
            continue;
        }
        if character == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
            sanitized.push(character);
            index += character_len;
            continue;
        }

        if !in_single_quote && matches!(character, '$' | '`') {
            return None;
        }
        if !in_single_quote && !in_double_quote {
            let starts_at_boundary = index == 0
                || command[..index]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_whitespace);
            if starts_at_boundary {
                if let Some(redirection) = ALLOWED_REDIRECTIONS.iter().find(|redirection| {
                    command[index..].starts_with(**redirection)
                        && command[index + redirection.len()..]
                            .chars()
                            .next()
                            .is_none_or(char::is_whitespace)
                }) {
                    sanitized.push(' ');
                    index += redirection.len();
                    continue;
                }
            }

            if matches!(
                character,
                ';' | '&' | '|' | '<' | '>' | '(' | ')' | '{' | '}' | '#'
            ) {
                return None;
            }
        }

        sanitized.push(character);
        index += character_len;
    }

    (!in_single_quote && !in_double_quote).then_some(sanitized)
}

fn is_safe_environment_assignment(token: &str) -> bool {
    let Some((name, _)) = token.split_once('=') else {
        return false;
    };
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic() || character == '_')
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn is_shell_builtin_or_keyword(command: &str) -> bool {
    matches!(
        command,
        "." | ":"
            | "!"
            | "["
            | "[["
            | "]]"
            | "alias"
            | "bg"
            | "bind"
            | "break"
            | "builtin"
            | "caller"
            | "case"
            | "cd"
            | "command"
            | "compgen"
            | "complete"
            | "compopt"
            | "continue"
            | "coproc"
            | "declare"
            | "dirs"
            | "disown"
            | "do"
            | "done"
            | "else"
            | "enable"
            | "esac"
            | "eval"
            | "exec"
            | "exit"
            | "export"
            | "echo"
            | "false"
            | "fc"
            | "fg"
            | "for"
            | "function"
            | "getopts"
            | "hash"
            | "help"
            | "history"
            | "if"
            | "jobs"
            | "kill"
            | "let"
            | "local"
            | "logout"
            | "mapfile"
            | "popd"
            | "printf"
            | "pushd"
            | "pwd"
            | "read"
            | "readarray"
            | "readonly"
            | "return"
            | "select"
            | "set"
            | "shift"
            | "shopt"
            | "source"
            | "suspend"
            | "test"
            | "then"
            | "time"
            | "times"
            | "trap"
            | "true"
            | "type"
            | "typeset"
            | "ulimit"
            | "umask"
            | "unalias"
            | "unset"
            | "until"
            | "wait"
            | "while"
    )
}

pub(crate) fn parse_safe_compound_command(command: &str) -> Option<SafeCompoundCommand> {
    use super::output_reducer::supports_command_output_reduction;

    let raw_stages = split_safe_and_then_stages(command)?;
    let mut stages = Vec::with_capacity(raw_stages.len());
    let mut has_reducer_stage = false;

    for (stage_index, original) in raw_stages.into_iter().enumerate() {
        let sanitized = strip_safe_stage_redirections(&original)?;
        let tokens = shell_tokens(&sanitized)?;
        if tokens.is_empty() {
            return None;
        }

        let assignment_count = tokens
            .iter()
            .take_while(|token| is_safe_environment_assignment(token))
            .count();
        if assignment_count == tokens.len() {
            return None;
        }
        let command_tokens = &tokens[assignment_count..];
        let leading_command = command_tokens.first()?.as_str();

        if leading_command == "cd" {
            if stage_index != 0 || assignment_count != 0 || command_tokens.len() != 2 {
                return None;
            }
            let target = command_tokens[1].clone();
            if target == "-"
                || target.starts_with('~')
                || target.contains('$')
                || target.contains('`')
            {
                return None;
            }
            stages.push(SafeCompoundStage::Navigation { original, target });
            continue;
        }
        if is_shell_builtin_or_keyword(leading_command) {
            return None;
        }
        if command_tokens.iter().enumerate().any(|(index, token)| {
            let basename = token.rsplit(['/', '\\']).next().unwrap_or(token);
            basename == "env"
                && command_tokens[index + 1..].iter().any(|argument| {
                    argument == "-S"
                        || argument.starts_with("-S")
                        || argument.starts_with("--split-string")
                })
        }) {
            return None;
        }
        let inline_script_runners = [
            "python", "python3", "node", "bash", "sh", "zsh", "ruby", "perl",
        ];
        if command_tokens.iter().enumerate().any(|(index, token)| {
            let basename = token.rsplit(['/', '\\']).next().unwrap_or(token);
            inline_script_runners.contains(&basename)
                && command_tokens[index + 1..].iter().any(|argument| {
                    argument == "-" || argument.starts_with("-c") || argument.starts_with("-e")
                })
        }) {
            return None;
        }

        let ShellStage::Command { tokens, .. } = classify_shell_stage(&sanitized)? else {
            return None;
        };
        let normalized = sanitized.trim().to_string();
        has_reducer_stage |= supports_command_output_reduction(&normalized);
        stages.push(SafeCompoundStage::Command {
            original,
            normalized,
            tokens,
        });
    }

    has_reducer_stage.then_some(SafeCompoundCommand { stages })
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
    use super::{
        classify_shell_stage, contains_unquoted_shell_operator, generate_shell_approval_patterns,
        parse_safe_compound_command, SafeCompoundStage, ShellStage,
    };

    #[test]
    fn detects_unquoted_compound_operators_without_matching_quoted_arguments() {
        for command in [
            "cargo test && git status",
            "cargo test; git status",
            "cargo test || git status",
            "cargo test | cat",
            "cargo test |& cat",
            "cargo test &",
            "cargo test\ngit status",
        ] {
            assert!(
                contains_unquoted_shell_operator(command),
                "expected operator in {command}"
            );
        }
        for command in [
            "cargo test -- --exact 'a;b'",
            "echo \"a|b\"",
            "echo 'a&&b'",
            "cargo test \\& literal",
        ] {
            assert!(
                !contains_unquoted_shell_operator(command),
                "unexpected operator in {command}"
            );
        }
    }

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
    fn safe_compound_parser_preserves_supported_and_plain_stages() {
        let parsed = parse_safe_compound_command(
            "cd app && python script.py --name 'a && b' && git diff -- src/main.rs && diff a b",
        )
        .expect("safe reducer chain should parse");

        assert_eq!(parsed.stages.len(), 4);
        assert!(matches!(
            &parsed.stages[0],
            SafeCompoundStage::Navigation { original, target }
                if original == "cd app" && target == "app"
        ));
        assert!(matches!(
            &parsed.stages[1],
            SafeCompoundStage::Command { original, normalized, .. }
                if original == "python script.py --name 'a && b'"
                    && normalized == "python script.py --name 'a && b'"
        ));
        assert!(matches!(
            &parsed.stages[2],
            SafeCompoundStage::Command { normalized, .. }
                if normalized == "git diff -- src/main.rs"
        ));
    }

    #[test]
    fn safe_compound_parser_requires_a_specialized_reducer_stage() {
        for command in [
            "python a.py && python b.py",
            "ls -la && env",
            "git log -1 && python script.py",
            "echo one && echo two",
        ] {
            assert!(
                parse_safe_compound_command(command).is_none(),
                "unexpected split plan for {command}"
            );
        }
        assert!(parse_safe_compound_command("python script.py && git status").is_some());
        assert!(parse_safe_compound_command("CI=1 cargo check && python script.py").is_some());
    }

    #[test]
    fn safe_compound_parser_rejects_ambiguous_shell_syntax() {
        for command in [
            "git status | cat",
            "git status |& cat",
            "git status || echo failed",
            "git status; echo done",
            "git status & echo done",
            "git status\necho done",
            "(git status) && python script.py",
            "{ git status; } && python script.py",
            "git status && echo $(pwd)",
            "git status && echo $HOME",
            "git status && echo `pwd`",
            "git status && echo ${HOME}",
            "git status && echo $((1 + 1))",
            "git status > output.txt && python script.py",
            "git status < input.txt && python script.py",
            "git status <<EOF && python script.py",
            "git status && # comment",
            "git status &&",
            "&& git status",
            "git status &&& python script.py",
            "git status && python script.py \\",
            "git status && 'unterminated",
        ] {
            assert!(
                parse_safe_compound_command(command).is_none(),
                "unexpected split plan for {command}"
            );
        }
    }

    #[test]
    fn safe_compound_parser_rejects_stateful_stages_and_inline_runners() {
        for command in [
            "export X=1 && git status",
            "unset X && git status",
            "set -e && git status",
            "umask 077 && git status",
            "alias gs='git status' && git status",
            "source env.sh && git status",
            ". env.sh && git status",
            "eval true && git status",
            "exec true && git status",
            "X=1 && git status",
            "echo ok && git status",
            "printf ok && git status",
            "python -c 'print(1)' && git status",
            "node -e 'console.log(1)' && git status",
            "sh -c 'true' && git status",
            "python - && git status",
            "/usr/bin/python3 -c 'print(1)' && git status",
            "/bin/sh -c 'true' && git status",
            "/usr/bin/env python3 -c 'print(1)' && git status",
            "env -S 'sh -c \"false\"' && git status",
            "env --split-string='python3 -c print(1)' && git status",
            "nohup env -S 'sh -c \"false\"' && git status",
            "nice /usr/bin/env --split-string='python3 -c print(1)' && git status",
            "sh -c'true' && git status",
        ] {
            assert!(
                parse_safe_compound_command(command).is_none(),
                "unexpected split plan for {command}"
            );
        }
        assert!(parse_safe_compound_command("python script.py && git status").is_some());
        assert!(parse_safe_compound_command("sh script.sh && git status").is_some());
    }

    #[test]
    fn safe_compound_parser_accepts_only_one_static_leading_cd() {
        for command in [
            "git status && cd app",
            "cd app extra && git status",
            "cd - && git status",
            "cd ~/app && git status",
            "cd '$HOME' && git status",
            "cd app && cd nested && git status",
            "pushd app && git status",
            "popd && git status",
        ] {
            assert!(
                parse_safe_compound_command(command).is_none(),
                "unexpected split plan for {command}"
            );
        }
        assert!(parse_safe_compound_command("cd 'app dir' && git status").is_some());
    }

    #[test]
    fn safe_compound_parser_allows_only_known_stream_redirections() {
        for command in [
            "git status 2>&1 && python script.py",
            "git status 1>&2 && python script.py",
            "git status 2>/dev/null && python script.py",
            "git status 2> /dev/null && python script.py",
            "git status 1>/dev/null && python script.py",
            "git status 1> /dev/null && python script.py",
            "git status &>/dev/null && python script.py",
        ] {
            assert!(
                parse_safe_compound_command(command).is_some(),
                "expected split plan for {command}"
            );
        }
        for command in [
            "git status 2>>/dev/null && python script.py",
            "git status >/tmp/status && python script.py",
            "git status 3>&1 && python script.py",
            "git status 2>& 1 && python script.py",
        ] {
            assert!(
                parse_safe_compound_command(command).is_none(),
                "unexpected split plan for {command}"
            );
        }
    }

    #[test]
    fn safe_compound_parser_enforces_the_stage_cap() {
        let at_limit = std::iter::repeat_n("python script.py", 15)
            .chain(std::iter::once("git status"))
            .collect::<Vec<_>>()
            .join(" && ");
        assert!(parse_safe_compound_command(&at_limit).is_some());

        let over_limit = format!("python script.py && {at_limit}");
        assert!(parse_safe_compound_command(&over_limit).is_none());
    }

    #[test]
    fn generate_shell_approval_patterns_keeps_primary_command_with_benign_stream_merge() {
        let patterns = generate_shell_approval_patterns(
            "cd . && cargo check --manifest-path src-tauri/Cargo.toml 2>&1 | tail -10",
        );

        assert_eq!(patterns, vec!["^cargo check($| .*)".to_string()]);
    }
}
