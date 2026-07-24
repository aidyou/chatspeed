use super::{reduce_priority_log_output, should_reduce_priority_log, CommandOutputReducer};
use crate::tools::helper::{contains_unquoted_shell_operator, shell_tokens};

const BUILD_TAIL_LINES: usize = 20;
const TEST_TAIL_LINES: usize = 30;

pub(crate) struct RustGoReducer;

impl CommandOutputReducer for RustGoReducer {
    fn matches(&self, normalized_command: &str) -> bool {
        is_rust_or_go_build_command(normalized_command)
            || is_rust_or_go_test_command(normalized_command)
    }

    fn reduce(&self, normalized_command: &str, _exit_code: i32, raw_content: &str) -> String {
        if should_reduce_priority_log(raw_content) {
            return reduce_priority_log_output(raw_content);
        }

        let tail_lines = if is_rust_or_go_test_command(normalized_command) {
            TEST_TAIL_LINES
        } else {
            BUILD_TAIL_LINES
        };
        reduce_to_tail(raw_content, tail_lines)
    }
}

/// Returns whether a command runs `cargo` or `go` in a build-like mode.
pub(crate) fn is_rust_or_go_build_command(normalized_command: &str) -> bool {
    command_segments(normalized_command).iter().any(|tokens| {
        matches!(
            command_name(tokens),
            (Some("cargo"), Some("check" | "build" | "clippy")) | (Some("go"), Some("build"))
        )
    })
}

/// Returns whether a command runs `cargo test` or `go test`.
pub(crate) fn is_rust_or_go_test_command(normalized_command: &str) -> bool {
    command_segments(normalized_command)
        .iter()
        .any(|tokens| matches!(command_name(tokens), (Some("cargo" | "go"), Some("test"))))
}

/// Returns whether a command runs `go build` or `go test`.
pub(crate) fn is_go_build_or_test_command(normalized_command: &str) -> bool {
    command_segments(normalized_command)
        .iter()
        .any(|tokens| matches!(command_name(tokens), (Some("go"), Some("build" | "test"))))
}

fn command_segments(command: &str) -> Vec<Vec<String>> {
    if contains_unquoted_shell_operator(command) {
        return Vec::new();
    }

    shell_tokens(command)
        .map(|tokens| {
            tokens
                .into_iter()
                .skip_while(is_environment_assignment)
                .collect()
        })
        .into_iter()
        .collect()
}

fn command_name(tokens: &[String]) -> (Option<&str>, Option<&str>) {
    (
        tokens.first().map(String::as_str),
        tokens.get(1).map(String::as_str),
    )
}

fn is_environment_assignment(token: &String) -> bool {
    token.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && name
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
    })
}

fn reduce_to_tail(raw_content: &str, tail_lines: usize) -> String {
    let lines = raw_content.lines().collect::<Vec<_>>();
    if lines.len() <= tail_lines {
        return raw_content.to_string();
    }

    format!(
        "[truncated previous output]\n{}",
        lines[lines.len().saturating_sub(tail_lines)..].join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::{is_rust_or_go_build_command, is_rust_or_go_test_command, RustGoReducer};
    use crate::tools::helper::CommandOutputReducer;

    #[test]
    fn recognizes_rust_and_go_commands_with_arguments_and_shell_segments() {
        for command in [
            "cargo check",
            "cargo check --workspace",
            "RUSTFLAGS='-D warnings' cargo clippy --all-targets",
            "go build ./...",
        ] {
            assert!(is_rust_or_go_build_command(command), "expected {command}");
        }

        for command in [
            "cargo test",
            "cargo test reducer --lib",
            "GOFLAGS=-mod=mod go test ./pkg/...",
        ] {
            assert!(is_rust_or_go_test_command(command), "expected {command}");
        }
    }

    #[test]
    fn does_not_match_unrelated_cargo_or_go_commands() {
        for command in ["cargo run", "cargo fmt", "go fmt ./...", "echo cargo build"] {
            assert!(
                !is_rust_or_go_build_command(command),
                "unexpected {command}"
            );
            assert!(!is_rust_or_go_test_command(command), "unexpected {command}");
        }
    }

    #[test]
    fn does_not_match_compound_commands() {
        for command in [
            "cargo test && git status",
            "cargo build; git status",
            "go test ./... || git status",
            "cargo test | cat",
            "go build ./... |& cat",
        ] {
            assert!(
                !is_rust_or_go_build_command(command),
                "unexpected {command}"
            );
            assert!(!is_rust_or_go_test_command(command), "unexpected {command}");
        }
    }

    #[test]
    fn reduces_build_output_to_the_diagnostic_tail() {
        let output = (1..=25)
            .map(|line| format!("build line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let reduced = RustGoReducer.reduce("cargo build", 0, &output);

        assert!(reduced.starts_with("[truncated previous output]"));
        assert!(reduced.contains("build line 25"));
        assert!(!reduced.contains("build line 1\n"));
    }
}
