use crate::tools::ToolCallResult;
use serde_json::json;

const MAX_DISPLAY_CHARS: usize = 30_000;
const GENERIC_LLM_MAX_LINES: usize = 40;
const BUILD_LLM_TAIL_LINES: usize = 20;
const TEST_LLM_TAIL_LINES: usize = 30;
const GIT_LOG_HEAD_LINES: usize = 30;
const GIT_DIFF_MAX_FILES: usize = 20;

pub(crate) fn build_shell_tool_result(
    command_str: &str,
    exit_code: i32,
    stdout: &str,
    stderr: &str,
) -> ToolCallResult {
    let display_stdout = truncate_with_marker(stdout, MAX_DISPLAY_CHARS);
    let display_stderr = truncate_with_marker(stderr, MAX_DISPLAY_CHARS);
    let display_content = format_shell_output(exit_code, &display_stdout, &display_stderr);

    let raw_content = format_shell_output(exit_code, stdout, stderr);
    let llm_content = reduce_shell_output_for_llm(command_str, exit_code, stdout, stderr, &raw_content);

    ToolCallResult::success(
        Some(display_content),
        Some(json!({
            "exit_code": exit_code,
            "llm_content": llm_content,
        })),
    )
}

fn format_shell_output(exit_code: i32, stdout: &str, stderr: &str) -> String {
    let mut result = format!("Exit code: {}\n", exit_code);
    if !stdout.is_empty() {
        result.push_str("\nstdout:\n");
        result.push_str(stdout);
    }
    if !stderr.is_empty() {
        result.push_str("\nstderr:\n");
        result.push_str(stderr);
    }
    result
}

fn reduce_shell_output_for_llm(
    command_str: &str,
    exit_code: i32,
    stdout: &str,
    stderr: &str,
    raw_content: &str,
) -> String {
    let normalized_command = normalize_command(command_str);

    if is_plain_cargo_check(&normalized_command)
        || is_plain_cargo_build(&normalized_command)
        || is_plain_cargo_clippy(&normalized_command)
    {
        return reduce_build_like_output(raw_content, BUILD_LLM_TAIL_LINES);
    }

    if is_plain_cargo_test(&normalized_command) || is_plain_go_test(&normalized_command) {
        return reduce_build_like_output(raw_content, TEST_LLM_TAIL_LINES);
    }

    if is_plain_go_build(&normalized_command)
        || is_plain_node_build(&normalized_command)
        || is_plain_frontend_build(&normalized_command)
    {
        return reduce_build_like_output(raw_content, BUILD_LLM_TAIL_LINES);
    }

    if normalized_command.contains("git show") || normalized_command.contains("git diff") {
        return reduce_git_diff_like_output(exit_code, stdout, stderr, raw_content);
    }

    if normalized_command.contains("git log") {
        return reduce_git_log_output(raw_content);
    }

    if raw_content.lines().count() > 120 || raw_content.chars().count() > 8_000 {
        return reduce_generic_output(raw_content, GENERIC_LLM_MAX_LINES);
    }

    raw_content.to_string()
}

fn normalize_command(command: &str) -> String {
    command
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn is_plain_cargo_check(command: &str) -> bool {
    command.ends_with("cargo check")
}

fn is_plain_go_build(command: &str) -> bool {
    command.ends_with("go build")
}

fn is_plain_go_test(command: &str) -> bool {
    command.ends_with("go test")
}

fn is_plain_cargo_build(command: &str) -> bool {
    command.ends_with("cargo build")
}

fn is_plain_cargo_clippy(command: &str) -> bool {
    command.ends_with("cargo clippy")
}

fn is_plain_cargo_test(command: &str) -> bool {
    command.ends_with("cargo test")
}

fn is_plain_node_build(command: &str) -> bool {
    command.ends_with("npm run build") || command.ends_with("yarn build")
}

fn is_plain_frontend_build(command: &str) -> bool {
    command.ends_with("pnpm build") || command.ends_with("pnpm tauri build")
}

fn reduce_build_like_output(raw_content: &str, tail_lines: usize) -> String {
    let lines: Vec<&str> = raw_content.lines().collect();
    if lines.len() <= tail_lines {
        return raw_content.to_string();
    }

    format!(
        "[truncated previous output]\n{}",
        lines[lines.len().saturating_sub(tail_lines)..].join("\n")
    )
}

fn reduce_git_log_output(raw_content: &str) -> String {
    let lines: Vec<&str> = raw_content.lines().collect();
    if lines.len() <= GIT_LOG_HEAD_LINES {
        return raw_content.to_string();
    }

    format!(
        "{}\n[truncated remaining git log output]",
        lines[..GIT_LOG_HEAD_LINES].join("\n")
    )
}

fn reduce_git_diff_like_output(
    exit_code: i32,
    stdout: &str,
    stderr: &str,
    raw_content: &str,
) -> String {
    if stdout.is_empty() && stderr.is_empty() {
        return raw_content.to_string();
    }

    let header_lines: Vec<&str> = stdout
        .lines()
        .take_while(|line| !line.starts_with("diff --git "))
        .take(20)
        .collect();

    let changed_files: Vec<String> = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("diff --git "))
        .filter_map(|rest| rest.split_whitespace().nth(1))
        .map(|path| path.trim_start_matches("b/").to_string())
        .collect();

    if changed_files.is_empty() && raw_content.lines().count() <= GENERIC_LLM_MAX_LINES {
        return raw_content.to_string();
    }

    let mut output = format!("Exit code: {}\n", exit_code);
    if !header_lines.is_empty() {
        output.push_str("\nstdout:\n");
        output.push_str(&header_lines.join("\n"));
        output.push('\n');
    }

    if !changed_files.is_empty() {
        output.push_str("\nChanged files:\n");
        for file in changed_files.iter().take(GIT_DIFF_MAX_FILES) {
            output.push_str("- ");
            output.push_str(file);
            output.push('\n');
        }
        if changed_files.len() > GIT_DIFF_MAX_FILES {
            output.push_str(&format!(
                "- ... and {} more files\n",
                changed_files.len() - GIT_DIFF_MAX_FILES
            ));
        }
    }

    if !stderr.trim().is_empty() {
        output.push_str("\nstderr:\n");
        output.push_str(&tail_lines(stderr, BUILD_LLM_TAIL_LINES));
        output.push('\n');
    }

    output.push_str(
        "\n[full diff omitted for LLM context; inspect specific files with read_file or narrower git commands if needed]",
    );
    output
}

fn reduce_generic_output(raw_content: &str, tail_lines_count: usize) -> String {
    let lines: Vec<&str> = raw_content.lines().collect();
    if lines.len() <= tail_lines_count {
        return raw_content.to_string();
    }

    format!(
        "[truncated previous output]\n{}",
        lines[lines.len().saturating_sub(tail_lines_count)..].join("\n")
    )
}

fn tail_lines(text: &str, count: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(count);
    lines[start..].join("\n")
}

fn truncate_with_marker(content: &str, max_len: usize) -> String {
    if content.len() <= max_len {
        return content.to_string();
    }

    let mut boundary = max_len;
    while boundary > 0 && !content.is_char_boundary(boundary) {
        boundary -= 1;
    }
    let mut truncated = content[..boundary].to_string();
    truncated.push_str("\n[Truncated]");
    truncated
}

#[cfg(test)]
mod tests {
    use super::build_shell_tool_result;

    #[test]
    fn cargo_check_without_args_uses_tail_for_llm_content() {
        let stdout = (1..=30)
            .map(|i| format!("line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let result = build_shell_tool_result("cargo check", 0, &stdout, "");
        let structured = result.structured_content.expect("structured content missing");
        let llm_content = structured["llm_content"]
            .as_str()
            .expect("llm_content should be a string");

        assert!(llm_content.starts_with("[truncated previous output]"));
        assert!(llm_content.contains("line 30"));
        assert!(!llm_content.contains("line 1\n"));
    }

    #[test]
    fn git_show_llm_content_omits_full_diff() {
        let stdout = "commit abc\nAuthor: test\nDate: today\n\ndiff --git a/src/main.rs b/src/main.rs\n@@ -1 +1 @@\n-old\n+new\ndiff --git a/src/lib.rs b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new\n";
        let result = build_shell_tool_result("git show HEAD", 0, stdout, "");
        let structured = result.structured_content.expect("structured content missing");
        let llm_content = structured["llm_content"]
            .as_str()
            .expect("llm_content should be a string");

        assert!(llm_content.contains("Changed files:"));
        assert!(llm_content.contains("src/main.rs"));
        assert!(llm_content.contains("src/lib.rs"));
        assert!(llm_content.contains("full diff omitted"));
        assert!(!llm_content.contains("@@ -1 +1 @@"));
    }

    #[test]
    fn cargo_test_without_args_uses_longer_tail_for_llm_content() {
        let stdout = (1..=40)
            .map(|i| format!("test line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let result = build_shell_tool_result("cargo test", 0, &stdout, "");
        let structured = result.structured_content.expect("structured content missing");
        let llm_content = structured["llm_content"]
            .as_str()
            .expect("llm_content should be a string");

        assert!(llm_content.contains("test line 40"));
        assert!(llm_content.contains("test line 11"));
        assert!(!llm_content.contains("test line 1\n"));
    }

    #[test]
    fn git_diff_llm_content_omits_patch_body() {
        let stdout = "diff --git a/src/main.rs b/src/main.rs\n@@ -1 +1 @@\n-old\n+new\ndiff --git a/src/lib.rs b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new\n";
        let result = build_shell_tool_result("git diff HEAD~1", 0, stdout, "");
        let structured = result.structured_content.expect("structured content missing");
        let llm_content = structured["llm_content"]
            .as_str()
            .expect("llm_content should be a string");

        assert!(llm_content.contains("Changed files:"));
        assert!(llm_content.contains("src/main.rs"));
        assert!(!llm_content.contains("@@ -1 +1 @@"));
    }
}
