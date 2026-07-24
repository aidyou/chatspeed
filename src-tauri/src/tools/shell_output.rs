use crate::libs::ai_temp::{
    persist_large_tool_output, persist_tool_output, LARGE_TOOL_OUTPUT_CHAR_LIMIT,
};
use crate::tools::helper::{
    contains_unquoted_shell_operator, detect_json_stdout, is_git_log_command,
    is_go_build_or_test_command, is_node_build_command, reduce_command_output,
    reduce_priority_log_output, should_reduce_priority_log,
};
use crate::tools::ToolCallResult;
use serde_json::json;

const MAX_DISPLAY_CHARS: usize = 30_000;
const GENERIC_LLM_MAX_LINES: usize = 40;
const EXPLICIT_SHAPED_LLM_MAX_LINES: usize = 240;
const EXPLICIT_SHAPED_LLM_MAX_CHARS: usize = 20_000;
const KSP_LLM_PRESERVE_CHARS: usize = 15_000;

pub(crate) fn build_shell_tool_result(
    command_str: &str,
    exit_code: i32,
    stdout: &str,
    stderr: &str,
) -> ToolCallResult {
    let (normalized_stdout, normalized_stderr) =
        normalize_shell_output_streams(command_str, exit_code, stdout, stderr);
    let raw_content = format_shell_output(exit_code, &normalized_stdout, &normalized_stderr);
    let normalized_command = normalize_command(command_str);
    let has_compound_shell_operator = contains_unquoted_shell_operator(command_str);
    let json_output = detect_json_stdout(&normalized_stdout, &normalized_stderr);
    let command_reduction = (!has_compound_shell_operator)
        .then(|| reduce_command_output(&normalized_command, exit_code, &raw_content))
        .flatten();
    let (display_content, llm_content, output_was_reduced) =
        if let Some(reduction) = command_reduction.as_ref() {
            let display_stdout = truncate_with_marker(&normalized_stdout, MAX_DISPLAY_CHARS);
            let display_stderr = truncate_with_marker(&normalized_stderr, MAX_DISPLAY_CHARS);
            let display_content = format_shell_output(exit_code, &display_stdout, &display_stderr);
            (
                display_content,
                reduction.content.clone(),
                reduction.persist_complete_output
                    || reduction.preserve_raw_output
                    || reduction.content != raw_content,
            )
        } else if let Some(json_output) = json_output {
            let json_content = format_json_output(exit_code, &json_output.compact_content);
            if json_content.chars().count() <= LARGE_TOOL_OUTPUT_CHAR_LIMIT {
                (json_content.clone(), json_content, json_output.was_minified)
            } else {
                let summary = summarize_large_json_output(
                    exit_code,
                    json_output.compact_content.chars().count(),
                    json_output.was_minified,
                );
                (summary.clone(), summary, true)
            }
        } else {
            let display_stdout = truncate_with_marker(&normalized_stdout, MAX_DISPLAY_CHARS);
            let display_stderr = truncate_with_marker(&normalized_stderr, MAX_DISPLAY_CHARS);
            let display_content = format_shell_output(exit_code, &display_stdout, &display_stderr);
            let llm_content = reduce_shell_output_for_llm(&normalized_command, &raw_content);
            let output_was_reduced = llm_content != raw_content;
            (display_content, llm_content, output_was_reduced)
        };
    let persisted_output = if output_was_reduced {
        persist_tool_output(&raw_content).map(Some)
    } else {
        persist_large_tool_output(&raw_content)
    };
    let persisted_output = match persisted_output {
        Ok(persisted) => persisted,
        Err(error) => {
            log::warn!("Failed to persist complete bash output: {}", error);
            None
        }
    };

    let mut structured_content = json!({
        "exit_code": exit_code,
        "llm_content": llm_content,
    });
    if let Some(persisted) = persisted_output {
        structured_content["persisted_output"] = json!({
            "path": persisted.path,
            "file_size_bytes": persisted.file_size_bytes,
            "reason": if output_was_reduced { "reduced" } else { "large" },
        });
    }

    ToolCallResult::success(Some(display_content), Some(structured_content))
}

pub(crate) fn should_render_stderr_line_as_stdout(command_str: &str, line: &str) -> bool {
    should_collect_stderr_line_as_stdout(command_str, line)
}

pub(crate) fn should_collect_stderr_line_as_stdout(command_str: &str, line: &str) -> bool {
    !contains_unquoted_shell_operator(command_str)
        && is_go_build_or_test_command(&normalize_command(command_str))
        && line.starts_with("go: downloading ")
}

pub(crate) fn normalize_shell_output_streams(
    command_str: &str,
    exit_code: i32,
    stdout: &str,
    stderr: &str,
) -> (String, String) {
    let stdout = strip_ansi_escape_sequences(stdout);
    let stderr = strip_ansi_escape_sequences(stderr);

    if exit_code != 0 || stderr.trim().is_empty() || contains_unquoted_shell_operator(command_str) {
        return (stdout, stderr);
    }

    let normalized_command = normalize_command(command_str);
    if is_node_build_command(&normalized_command) {
        return (append_output(stdout, &stderr), String::new());
    }

    let mut moved_stderr_lines = Vec::new();
    let mut remaining_stderr_lines = Vec::new();

    for line in stderr.lines() {
        if should_render_stderr_line_as_stdout(command_str, line) {
            moved_stderr_lines.push(line);
        } else {
            remaining_stderr_lines.push(line);
        }
    }

    if moved_stderr_lines.is_empty() {
        return (stdout, stderr);
    }

    let mut normalized_stdout = stdout;
    for line in moved_stderr_lines {
        if !normalized_stdout.is_empty() && !normalized_stdout.ends_with('\n') {
            normalized_stdout.push('\n');
        }
        normalized_stdout.push_str(line);
        normalized_stdout.push('\n');
    }

    let normalized_stderr = if remaining_stderr_lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", remaining_stderr_lines.join("\n"))
    };

    (normalized_stdout, normalized_stderr)
}

fn append_output(mut stdout: String, stderr: &str) -> String {
    if !stdout.is_empty() && !stdout.ends_with('\n') {
        stdout.push('\n');
    }
    stdout.push_str(stderr);
    stdout
}

#[derive(Default)]
pub(crate) struct AnsiOutputSanitizer {
    state: AnsiOutputSanitizerState,
}

#[derive(Default)]
enum AnsiOutputSanitizerState {
    #[default]
    Text,
    Escape,
    Csi,
    EscSequence,
    Osc,
    OscEscape,
    String,
    StringEscape,
}

impl AnsiOutputSanitizer {
    pub(crate) fn sanitize(&mut self, content: &str) -> String {
        let mut output = String::with_capacity(content.len());

        for character in content.chars() {
            match self.state {
                AnsiOutputSanitizerState::Text => match character {
                    '\u{1b}' => self.state = AnsiOutputSanitizerState::Escape,
                    '\u{009B}' => self.state = AnsiOutputSanitizerState::Csi,
                    '\u{009D}' => self.state = AnsiOutputSanitizerState::Osc,
                    '\u{0090}' | '\u{0098}' | '\u{009E}' | '\u{009F}' => {
                        self.state = AnsiOutputSanitizerState::String
                    }
                    '\u{0080}'..='\u{009C}' => {}
                    _ => output.push(character),
                },
                AnsiOutputSanitizerState::Escape => match character {
                    '[' => self.state = AnsiOutputSanitizerState::Csi,
                    ']' => self.state = AnsiOutputSanitizerState::Osc,
                    'P' | 'X' | '^' | '_' => self.state = AnsiOutputSanitizerState::String,
                    '\u{20}'..='\u{2F}' => self.state = AnsiOutputSanitizerState::EscSequence,
                    _ => self.state = AnsiOutputSanitizerState::Text,
                },
                AnsiOutputSanitizerState::Csi => {
                    if ('@'..='~').contains(&character) {
                        self.state = AnsiOutputSanitizerState::Text;
                    }
                }
                AnsiOutputSanitizerState::EscSequence => {
                    if ('\u{30}'..='\u{7E}').contains(&character) {
                        self.state = AnsiOutputSanitizerState::Text;
                    }
                }
                AnsiOutputSanitizerState::Osc => match character {
                    '\u{7}' | '\u{009C}' => self.state = AnsiOutputSanitizerState::Text,
                    '\u{1b}' => self.state = AnsiOutputSanitizerState::OscEscape,
                    _ => {}
                },
                AnsiOutputSanitizerState::OscEscape => {
                    self.state = if character == '\\' {
                        AnsiOutputSanitizerState::Text
                    } else if character == '\u{1b}' {
                        AnsiOutputSanitizerState::OscEscape
                    } else {
                        AnsiOutputSanitizerState::Osc
                    };
                }
                AnsiOutputSanitizerState::String => match character {
                    '\u{009C}' => self.state = AnsiOutputSanitizerState::Text,
                    '\u{1b}' => self.state = AnsiOutputSanitizerState::StringEscape,
                    _ => {}
                },
                AnsiOutputSanitizerState::StringEscape => {
                    self.state = if character == '\\' {
                        AnsiOutputSanitizerState::Text
                    } else if character == '\u{1b}' {
                        AnsiOutputSanitizerState::StringEscape
                    } else {
                        AnsiOutputSanitizerState::String
                    };
                }
            }
        }

        output
    }
}

pub(crate) fn strip_ansi_escape_sequences(content: &str) -> String {
    AnsiOutputSanitizer::default().sanitize(content)
}

fn format_json_output(exit_code: i32, compact_json: &str) -> String {
    format!("Exit code: {exit_code}\n\nstdout (JSON):\n{compact_json}")
}

fn summarize_large_json_output(
    exit_code: i32,
    compact_char_count: usize,
    was_minified: bool,
) -> String {
    let minification_note = if was_minified {
        " Pretty-printed whitespace was removed before storage."
    } else {
        ""
    };
    format!(
        "Exit code: {exit_code}\n\nstdout (JSON):\n[Valid JSON output is {compact_char_count} characters and was omitted without truncation. The complete original output was saved for inspection.]{}",
        minification_note
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

fn reduce_shell_output_for_llm(normalized_command: &str, raw_content: &str) -> String {
    if is_git_log_command(normalized_command) {
        return raw_content.to_string();
    }

    if is_explicitly_shaped_read_output(&normalized_command) {
        return reduce_explicitly_shaped_output(raw_content);
    }

    if is_ksp_command(&normalized_command) && raw_content.chars().count() <= KSP_LLM_PRESERVE_CHARS
    {
        return raw_content.to_string();
    }

    if should_reduce_priority_log(raw_content) {
        return reduce_priority_log_output(raw_content);
    }

    if raw_content.lines().count() > 120 || raw_content.chars().count() > 8_000 {
        return reduce_generic_output(raw_content, GENERIC_LLM_MAX_LINES);
    }

    raw_content.to_string()
}

pub(crate) fn normalize_command(command: &str) -> String {
    command
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn is_ksp_command(command: &str) -> bool {
    command == "ksp"
        || command.starts_with("ksp ")
        || command.contains(" ksp ")
        || command.contains("/ksp ")
}

fn is_explicitly_shaped_read_output(command: &str) -> bool {
    command.contains("| head ")
        || command.contains("| tail ")
        || command.starts_with("head ")
        || command.starts_with("tail ")
        || command.contains("sed -n ")
        || command.contains("awk ")
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

fn reduce_explicitly_shaped_output(raw_content: &str) -> String {
    if raw_content.lines().count() <= EXPLICIT_SHAPED_LLM_MAX_LINES
        && raw_content.chars().count() <= EXPLICIT_SHAPED_LLM_MAX_CHARS
    {
        return raw_content.to_string();
    }

    let lines: Vec<&str> = raw_content.lines().collect();
    let head_count = EXPLICIT_SHAPED_LLM_MAX_LINES / 2;
    let tail_count = EXPLICIT_SHAPED_LLM_MAX_LINES.saturating_sub(head_count);
    let head = lines
        .iter()
        .take(head_count)
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    let tail = lines
        .iter()
        .skip(lines.len().saturating_sub(tail_count))
        .copied()
        .collect::<Vec<_>>()
        .join("\n");

    format!("{}\n[truncated middle output]\n{}", head, tail)
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
    use super::{
        build_shell_tool_result, normalize_shell_output_streams,
        should_render_stderr_line_as_stdout, strip_ansi_escape_sequences,
    };
    use crate::libs::ai_temp::{resolve_ai_temp_path, LARGE_TOOL_OUTPUT_CHAR_LIMIT};
    use std::fs;
    use std::path::Path;

    #[test]
    fn cargo_check_without_args_uses_tail_for_llm_content() {
        let stdout = (1..=30)
            .map(|i| format!("line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let result = build_shell_tool_result("cargo check", 0, &stdout, "");
        let structured = result
            .structured_content
            .expect("structured content missing");
        let llm_content = structured["llm_content"]
            .as_str()
            .expect("llm_content should be a string");

        assert!(llm_content.starts_with("[truncated previous output]"));
        assert!(llm_content.contains("line 30"));
        assert!(!llm_content.contains("line 1\n"));
        assert_eq!(
            structured["persisted_output"]["reason"].as_str(),
            Some("reduced")
        );

        let ai_path = structured["persisted_output"]["path"]
            .as_str()
            .expect("persisted output path missing");
        fs::remove_file(resolve_ai_temp_path(Path::new(ai_path))).unwrap();
    }

    #[test]
    fn valid_pretty_json_is_compacted_and_original_output_is_persisted() {
        let stdout = "{\n  \"items\": [\n    { \"id\": 1 },\n    { \"id\": 2 }\n  ]\n}\n";
        let result = build_shell_tool_result("cargo check --workspace", 0, stdout, "");
        let content = result.content.as_deref().expect("display content missing");
        let structured = result
            .structured_content
            .expect("structured content missing");

        assert_eq!(
            content,
            "Exit code: 0\n\nstdout (JSON):\n{\"items\":[{\"id\":1},{\"id\":2}]}"
        );
        assert_eq!(structured["llm_content"].as_str(), Some(content));
        assert_eq!(
            structured["persisted_output"]["reason"].as_str(),
            Some("reduced")
        );

        let ai_path = structured["persisted_output"]["path"]
            .as_str()
            .expect("persisted output path missing");
        let physical_path = resolve_ai_temp_path(Path::new(ai_path));
        assert_eq!(
            fs::read_to_string(&physical_path).unwrap(),
            format!("Exit code: 0\n\nstdout:\n{stdout}")
        );
        fs::remove_file(physical_path).unwrap();
    }

    #[test]
    fn test_reporter_json_uses_the_specialized_summary_and_persists_raw_output() {
        let stdout = "{\"numTotalTests\":3,\"numPassedTests\":2,\"numFailedTests\":1,\"numPendingTests\":0,\"testResults\":[{\"name\":\"src/example.test.ts\",\"assertionResults\":[{\"fullName\":\"example fails\",\"status\":\"failed\",\"failureMessages\":[\"expected true to be false\"]}]}]}";
        let result = build_shell_tool_result("pnpm vitest run --reporter=json", 1, stdout, "");
        let structured = result
            .structured_content
            .expect("structured content missing");
        let llm_content = structured["llm_content"]
            .as_str()
            .expect("llm content missing");

        assert!(llm_content.contains("Vitest result:"));
        assert!(llm_content.contains("Tests 3 total | 2 passed | 1 failed | 0 skipped"));
        assert!(llm_content.contains("src/example.test.ts > example fails"));
        assert_eq!(
            structured["persisted_output"]["reason"].as_str(),
            Some("reduced")
        );

        let ai_path = structured["persisted_output"]["path"]
            .as_str()
            .expect("persisted output path missing");
        let physical_path = resolve_ai_temp_path(Path::new(ai_path));
        assert_eq!(
            fs::read_to_string(&physical_path).unwrap(),
            format!("Exit code: 1\n\nstdout:\n{stdout}")
        );
        fs::remove_file(physical_path).unwrap();
    }

    #[test]
    fn uv_pytest_retains_diagnostics_and_persists_raw_output() {
        let stdout = "============================= test session starts =============================\ncollected 2 items\n\ntests/example.py .F [100%]\n\n=================================== FAILURES ===================================\n_______________________________ test_value _______________________________\n>       assert actual == 2\nE       AssertionError: expected 2\ntests/example.py:12: AssertionError\n\n=========================== short test summary info ============================\nFAILED tests/example.py::test_value - AssertionError: expected 2\n========================= 1 passed, 1 failed in 0.12s =========================";
        let result = build_shell_tool_result("uv run pytest", 1, stdout, "");
        let structured = result
            .structured_content
            .expect("structured content missing");
        let llm_content = structured["llm_content"]
            .as_str()
            .expect("llm content missing");

        assert!(llm_content.contains("Pytest result:"));
        assert!(llm_content.contains("assert actual == 2"));
        assert!(llm_content.contains("tests/example.py:12"));
        assert_eq!(
            structured["persisted_output"]["reason"].as_str(),
            Some("reduced")
        );

        let ai_path = structured["persisted_output"]["path"]
            .as_str()
            .expect("persisted output path missing");
        let physical_path = resolve_ai_temp_path(Path::new(ai_path));
        assert_eq!(
            fs::read_to_string(&physical_path).unwrap(),
            format!("Exit code: 1\n\nstdout:\n{stdout}")
        );
        fs::remove_file(physical_path).unwrap();
    }

    #[test]
    fn large_complete_json_is_summarized_without_partial_json() {
        let stdout = format!(
            "{{\"payload\":\"{}\"}}",
            "x".repeat(LARGE_TOOL_OUTPUT_CHAR_LIMIT)
        );
        let result = build_shell_tool_result("cargo test --workspace", 0, &stdout, "");
        let content = result.content.as_deref().expect("display content missing");
        let structured = result
            .structured_content
            .expect("structured content missing");

        assert!(content.contains("Valid JSON output is"));
        assert!(content.contains("omitted without truncation"));
        assert!(!content.contains("[Truncated]"));
        assert!(!content.contains("\"payload\":\"xxx"));
        assert_eq!(structured["llm_content"].as_str(), Some(content));
        assert_eq!(
            structured["persisted_output"]["reason"].as_str(),
            Some("reduced")
        );

        let ai_path = structured["persisted_output"]["path"]
            .as_str()
            .expect("persisted output path missing");
        let physical_path = resolve_ai_temp_path(Path::new(ai_path));
        assert_eq!(
            fs::read_to_string(&physical_path).unwrap(),
            format!("Exit code: 0\n\nstdout:\n{stdout}")
        );
        fs::remove_file(physical_path).unwrap();
    }

    #[test]
    fn large_shell_output_is_persisted_without_display_truncation_loss() {
        let stdout = (1..=500)
            .map(|line| format!("line {line:03}: {}", "x".repeat(80)))
            .collect::<Vec<_>>()
            .join("\n");
        let result = build_shell_tool_result("custom-command", 0, &stdout, "");
        let structured = result.structured_content.unwrap();
        let persisted = &structured["persisted_output"];
        let ai_path = persisted["path"].as_str().unwrap();
        let physical_path = resolve_ai_temp_path(Path::new(ai_path));
        let saved_output = fs::read_to_string(&physical_path).unwrap();

        assert!(ai_path.starts_with("/tmp/"));
        assert_eq!(Path::new(ai_path).file_name().unwrap().len(), 13);
        assert_eq!(
            persisted["file_size_bytes"].as_u64(),
            Some(saved_output.len() as u64)
        );
        assert!(saved_output.contains("line 001:"));
        assert!(saved_output.contains("line 500:"));
        assert!(!saved_output.contains("[Truncated]"));

        fs::remove_file(physical_path).unwrap();
    }

    #[test]
    fn long_log_prioritizes_early_diagnostics_and_persists_original_output() {
        let stdout = (1..=70)
            .map(|line| match line {
                5 => "before error context".to_string(),
                6 => "ERROR: unable to compile package".to_string(),
                7 => "after error context".to_string(),
                25 | 26 => "warning: deprecated setting".to_string(),
                45 => "test result: FAILED. 1 passed; 1 failed".to_string(),
                _ => format!("progress line {line}"),
            })
            .collect::<Vec<_>>()
            .join("\n");
        let result = build_shell_tool_result("custom-command", 1, &stdout, "");
        let structured = result
            .structured_content
            .expect("structured content missing");
        let llm_content = structured["llm_content"]
            .as_str()
            .expect("llm content missing");

        assert!(llm_content.contains("before error context"));
        assert!(llm_content.contains("ERROR: unable to compile package"));
        assert!(llm_content.contains("after error context"));
        assert_eq!(
            llm_content.matches("warning: deprecated setting").count(),
            1
        );
        assert!(llm_content.contains("test result: FAILED"));
        assert!(llm_content.contains("[omitted log lines]"));
        assert_eq!(
            structured["persisted_output"]["reason"].as_str(),
            Some("reduced")
        );

        let ai_path = structured["persisted_output"]["path"]
            .as_str()
            .expect("persisted output path missing");
        fs::remove_file(resolve_ai_temp_path(Path::new(ai_path))).unwrap();
    }

    #[test]
    fn chained_git_commands_preserve_aggregate_output() {
        let stdout = "diff --git a/src/main.rs b/src/main.rs\n@@ -1 +1 @@\n-old\n+new\nOn branch main\nChanges not staged for commit:\n\tmodified:   src/main.rs\n";
        let result = build_shell_tool_result("git diff && git status", 0, stdout, "");
        let structured = result
            .structured_content
            .expect("structured content missing");
        let llm_content = structured["llm_content"]
            .as_str()
            .expect("llm content missing");

        assert!(llm_content.contains("+new"));
        assert!(llm_content.contains("On branch main"));
        assert!(llm_content.contains("modified:   src/main.rs"));
        assert!(!llm_content.contains("Changes:\n"));
    }

    #[test]
    fn git_pipelines_preserve_aggregate_output() {
        let stdout = "diff --git a/src/main.rs b/src/main.rs\n@@ -1 +1 @@\n-old\n+new\ntrailing filtered output\n";
        for command in ["git diff | cat", "git diff |& sed -n '1,10p'"] {
            let result = build_shell_tool_result(command, 0, stdout, "");
            let structured = result
                .structured_content
                .expect("structured content missing");
            let llm_content = structured["llm_content"]
                .as_str()
                .expect("llm content missing");

            assert!(
                llm_content.contains("+new"),
                "expected raw output for {command}"
            );
            assert!(
                llm_content.contains("trailing filtered output"),
                "expected trailing output for {command}"
            );
            assert!(!llm_content.contains("Changes:\n"));
        }
    }

    #[test]
    fn compound_specialized_commands_preserve_aggregate_output() {
        let cases = [
            (
                "cargo test && git status",
                "running test suite\ntest result: ok\nOn branch main\nmodified: src/main.rs\n",
                "On branch main",
            ),
            (
                "pnpm build; git status",
                "✓ built in 1.00s\nOn branch main\nmodified: src/main.rs\n",
                "On branch main",
            ),
            (
                "pytest || git status",
                "FAILED tests/example.py::test_value\nOn branch main\nmodified: src/main.rs\n",
                "On branch main",
            ),
            (
                "pnpm vitest run |& cat",
                "Tests  1 passed (1)\ntrailing pipeline output\n",
                "trailing pipeline output",
            ),
            (
                "cargo test\ngit status",
                "test result: ok\nOn branch main\nmodified: src/main.rs\n",
                "On branch main",
            ),
            (
                "pnpm build\ngit status",
                "✓ built in 1.00s\nOn branch main\nmodified: src/main.rs\n",
                "On branch main",
            ),
            (
                "pytest\ngit status",
                "FAILED tests/example.py::test_value\nOn branch main\nmodified: src/main.rs\n",
                "On branch main",
            ),
        ];

        for (command, stdout, required_output) in cases {
            let result = build_shell_tool_result(command, 0, stdout, "");
            let structured = result
                .structured_content
                .expect("structured content missing");
            let llm_content = structured["llm_content"]
                .as_str()
                .expect("llm content missing");

            assert!(
                llm_content.contains(required_output),
                "expected aggregate output for {command}"
            );
            assert!(!llm_content.contains("Build result:"));
            assert!(!llm_content.contains("Pytest result:"));
            assert!(!llm_content.contains("Vitest result:"));
        }
    }

    #[test]
    fn git_status_removes_hints_and_persists_complete_output() {
        let stdout = "On branch main\nYour branch is ahead of 'origin/main' by 1 commit.\n\nChanges not staged for commit:\n  (use \"git add <file>...\" to update what will be committed)\n  (use \"git restore <file>...\" to discard changes in working directory)\n\tmodified:   src/main.rs\n\nUntracked files:\n  (use \"git add <file>...\" to include in what will be committed)\n\tnew file.txt\n";
        let result = build_shell_tool_result("git status", 0, stdout, "");
        let structured = result
            .structured_content
            .expect("structured content missing");
        let llm_content = structured["llm_content"]
            .as_str()
            .expect("llm content missing");

        assert!(llm_content.contains("On branch main"));
        assert!(llm_content.contains("modified:   src/main.rs"));
        assert!(llm_content.contains("new file.txt"));
        assert!(!llm_content.contains("use \"git add"));
        assert_eq!(
            structured["persisted_output"]["reason"].as_str(),
            Some("reduced")
        );

        let ai_path = structured["persisted_output"]["path"]
            .as_str()
            .expect("persisted output path missing");
        let physical_path = resolve_ai_temp_path(Path::new(ai_path));
        assert_eq!(
            fs::read_to_string(&physical_path).unwrap(),
            format!("Exit code: 0\n\nstdout:\n{stdout}")
        );
        fs::remove_file(physical_path).unwrap();
    }

    #[test]
    fn git_write_successes_are_compacted_and_failures_are_preserved() {
        let cases = [
            ("git add src/main.rs", 0, "", "", "ok"),
            (
                "git commit -m message",
                0,
                "[main abc1234def] message\n 1 file changed\n",
                "",
                "Exit code: 0\n\nGit commit: ok abc1234",
            ),
            (
                "git push",
                0,
                "",
                "Writing objects: 100%\n   abc1234..def5678  main -> main\n",
                "Exit code: 0\n\nGit push: ok main",
            ),
            (
                "git pull",
                0,
                "Updating abc1234..def5678\n 3 files changed, 10 insertions(+), 2 deletions(-)\n",
                "",
                "Exit code: 0\n\nGit pull: ok 3 files +10 -2",
            ),
        ];

        for (command, exit_code, stdout, stderr, expected) in cases {
            let result = build_shell_tool_result(command, exit_code, stdout, stderr);
            let structured = result
                .structured_content
                .expect("structured content missing");
            assert_eq!(structured["llm_content"].as_str(), Some(expected));
            assert_eq!(
                structured["persisted_output"]["reason"].as_str(),
                Some("reduced")
            );

            let ai_path = structured["persisted_output"]["path"]
                .as_str()
                .expect("persisted output path missing");
            fs::remove_file(resolve_ai_temp_path(Path::new(ai_path))).unwrap();
        }

        let stdout = "remote: Permission denied\nfatal: unable to access remote\n";
        let result = build_shell_tool_result("git push", 1, "", stdout);
        let structured = result
            .structured_content
            .expect("structured content missing");
        assert_eq!(
            structured["llm_content"].as_str(),
            Some("Exit code: 1\n\nstderr:\nremote: Permission denied\nfatal: unable to access remote\n")
        );
    }

    #[test]
    fn large_git_write_failures_remain_complete_for_llm() {
        let diagnostic = (1..=130)
            .map(|line| format!("fatal diagnostic {line}"))
            .collect::<Vec<_>>()
            .join("\n");

        for command in [
            "git add src/main.rs",
            "git commit -m message",
            "git push",
            "git pull",
        ] {
            let result = build_shell_tool_result(command, 1, "", &diagnostic);
            let structured = result
                .structured_content
                .expect("structured content missing");
            let expected = format!("Exit code: 1\n\nstderr:\n{diagnostic}");
            assert_eq!(
                structured["llm_content"].as_str(),
                Some(expected.as_str()),
                "expected complete diagnostics for {command}"
            );
            assert_eq!(
                structured["persisted_output"]["reason"].as_str(),
                Some("reduced")
            );

            let ai_path = structured["persisted_output"]["path"]
                .as_str()
                .expect("persisted output path missing");
            fs::remove_file(resolve_ai_temp_path(Path::new(ai_path))).unwrap();
        }
    }

    #[test]
    fn failed_git_diff_and_status_remain_complete_for_llm() {
        let cases = [
            (
                "git diff --exit-code",
                "diff --git a/src/main.rs b/src/main.rs\n@@ -1 +1 @@\n-old\n+new\n",
            ),
            (
                "git status --porcelain",
                "fatal: not a git repository (or any of the parent directories): .git\n",
            ),
        ];

        for (command, stdout) in cases {
            let result = build_shell_tool_result(command, 1, stdout, "");
            let structured = result
                .structured_content
                .expect("structured content missing");
            let expected = format!("Exit code: 1\n\nstdout:\n{stdout}");
            assert_eq!(
                structured["llm_content"].as_str(),
                Some(expected.as_str()),
                "expected complete diagnostics for {command}"
            );

            let ai_path = structured["persisted_output"]["path"]
                .as_str()
                .expect("persisted output path missing");
            let physical_path = resolve_ai_temp_path(Path::new(ai_path));
            assert_eq!(fs::read_to_string(&physical_path).unwrap(), expected);
            fs::remove_file(physical_path).unwrap();
        }
    }

    #[test]
    fn unsupported_git_failures_remain_complete_for_llm() {
        let diagnostic = (1..=130)
            .map(|line| format!("git failure detail {line}"))
            .collect::<Vec<_>>()
            .join("\n");

        for command in ["git fetch", "git merge feature"] {
            let result = build_shell_tool_result(command, 1, "", &diagnostic);
            let structured = result
                .structured_content
                .expect("structured content missing");
            let expected = format!("Exit code: 1\n\nstderr:\n{diagnostic}");
            assert_eq!(
                structured["llm_content"].as_str(),
                Some(expected.as_str()),
                "expected complete diagnostics for {command}"
            );

            let ai_path = structured["persisted_output"]["path"]
                .as_str()
                .expect("persisted output path missing");
            let physical_path = resolve_ai_temp_path(Path::new(ai_path));
            assert_eq!(fs::read_to_string(&physical_path).unwrap(), expected);
            fs::remove_file(physical_path).unwrap();
        }
    }

    #[test]
    fn invalid_global_git_option_failure_remains_complete_for_llm() {
        let diagnostic = (1..=130)
            .map(|line| format!("unknown option diagnostic {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let result = build_shell_tool_result("git --invalid-global-option", 129, "", &diagnostic);
        let structured = result
            .structured_content
            .expect("structured content missing");
        let expected = format!("Exit code: 129\n\nstderr:\n{diagnostic}");
        assert_eq!(structured["llm_content"].as_str(), Some(expected.as_str()));

        let ai_path = structured["persisted_output"]["path"]
            .as_str()
            .expect("persisted output path missing");
        let physical_path = resolve_ai_temp_path(Path::new(ai_path));
        assert_eq!(fs::read_to_string(&physical_path).unwrap(), expected);
        fs::remove_file(physical_path).unwrap();
    }

    #[test]
    fn git_show_llm_content_retains_hunks_and_patch_lines() {
        let stdout = format!(
            "commit abc\nAuthor: test\nDate: today\n\ndiff --git a/src/main.rs b/src/main.rs\nindex 123..456 100644\n--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,102 +1,102 @@\n-old\n+new\n{}\ndiff --git a/src/lib.rs b/src/lib.rs\nindex 789..abc 100644\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,102 +1,102 @@\n-old\n+new\n{}",
            (1..=100)
                .map(|line| format!(" unchanged main context {line}"))
                .collect::<Vec<_>>()
                .join("\n"),
            (1..=100)
                .map(|line| format!(" unchanged lib context {line}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        let result = build_shell_tool_result("git show HEAD", 0, &stdout, "");
        let structured = result
            .structured_content
            .expect("structured content missing");
        let llm_content = structured["llm_content"]
            .as_str()
            .expect("llm_content should be a string");

        assert!(llm_content.contains("Commit metadata:"));
        assert!(llm_content.contains("src/main.rs"));
        assert!(llm_content.contains("src/lib.rs"));
        assert!(llm_content.contains("@@ -1,102 +1,102 @@"));
        assert!(llm_content.contains("-old"));
        assert!(llm_content.contains("+new"));

        let ai_path = structured["persisted_output"]["path"]
            .as_str()
            .expect("persisted output path missing");
        fs::remove_file(resolve_ai_temp_path(Path::new(ai_path))).unwrap();
    }

    #[test]
    fn git_log_with_global_options_remains_complete_for_llm() {
        let stdout = (1..=60)
            .map(|line| format!("commit detail line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let result = build_shell_tool_result("git --no-pager log -1", 0, &stdout, "");
        let structured = result
            .structured_content
            .expect("structured content missing");
        let llm_content = structured["llm_content"]
            .as_str()
            .expect("llm content missing");

        assert!(llm_content.contains("commit detail line 1"));
        assert!(llm_content.contains("commit detail line 60"));
        assert!(!llm_content.contains("truncated"));
    }

    #[test]
    fn cargo_test_without_args_uses_longer_tail_for_llm_content() {
        let stdout = (1..=40)
            .map(|i| format!("test line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let result = build_shell_tool_result("cargo test", 0, &stdout, "");
        let structured = result
            .structured_content
            .expect("structured content missing");
        let llm_content = structured["llm_content"]
            .as_str()
            .expect("llm_content should be a string");

        assert!(llm_content.contains("test line 40"));
        assert!(llm_content.contains("test line 11"));
        assert!(!llm_content.contains("test line 1\n"));

        let ai_path = structured["persisted_output"]["path"]
            .as_str()
            .expect("persisted output path missing");
        fs::remove_file(resolve_ai_temp_path(Path::new(ai_path))).unwrap();
    }

    #[test]
    fn ansi_escape_sequences_are_removed_from_all_output_streams() {
        let (stdout, stderr) = normalize_shell_output_streams(
            "cargo check",
            1,
            "\u{1b}[32mcheck succeeded\u{1b}[0m\n",
            "\u{1b}]8;;https://example.com\u{1b}\\error\u{1b}]8;;\u{1b}\\\n",
        );

        assert_eq!(stdout, "check succeeded\n");
        assert_eq!(stderr, "error\n");
        for sequence in [
            "\u{1b}]8;;https://example.com\u{1b}\\",
            "\u{1b}P1;2|payload\u{1b}\\",
            "\u{1b}Xsos\u{1b}\\",
            "\u{1b}^pm\u{1b}\\",
            "\u{1b}_apc\u{1b}\\",
            "\u{1b}(0",
            "\u{1b}#8",
            "\u{009D}title\u{009C}",
            "\u{0090}payload\u{009C}",
            "\u{0098}sos\u{009C}",
            "\u{009E}pm\u{009C}",
            "\u{009F}apc\u{009C}",
        ] {
            assert_eq!(
                strip_ansi_escape_sequences(&format!("before{sequence}after")),
                "beforeafter"
            );
        }
        assert_eq!(
            strip_ansi_escape_sequences("\u{009B}31mplain\u{009B}0m"),
            "plain"
        );
        assert_eq!(
            strip_ansi_escape_sequences("\u{0090}secret\u{7}payload\u{009C}visible"),
            "visible"
        );
    }

    #[test]
    fn successful_node_build_moves_stderr_to_stdout_and_persists_reduced_output() {
        let stdout = "bin/assets/index.js 3,303.40 kB │ gzip: 1,023.37 kB\n✓ built in 16.02s\n";
        let stderr = "(!) Some chunks are larger than 500 kB after minification. Consider:\n- Using dynamic import() to code-split the application\n";
        let (normalized_stdout, normalized_stderr) =
            normalize_shell_output_streams("pnpm build", 0, stdout, stderr);

        assert_eq!(normalized_stdout, format!("{stdout}{stderr}"),);
        assert!(normalized_stderr.is_empty());

        let result = build_shell_tool_result("pnpm build", 0, stdout, stderr);
        let expected_content = format!("Exit code: 0\n\nstdout:\n{stdout}{stderr}");
        assert_eq!(result.content.as_deref(), Some(expected_content.as_str()));
        let structured = result
            .structured_content
            .expect("structured content missing");
        let llm_content = structured["llm_content"]
            .as_str()
            .expect("llm_content should be a string");
        assert_eq!(
            llm_content,
            "Exit code: 0\n\nBuild result:\nBuild output: 1 file, 3.23 MB (gzip: 1023.37 kB across 1 file)\n✓ built in 16.02s"
        );
        assert!(!llm_content.contains("bin/assets/index.js"));
        assert_eq!(
            structured["persisted_output"]["reason"].as_str(),
            Some("reduced")
        );

        let ai_path = structured["persisted_output"]["path"]
            .as_str()
            .expect("persisted output path missing");
        let physical_path = resolve_ai_temp_path(Path::new(ai_path));
        assert_eq!(
            fs::read_to_string(&physical_path).unwrap(),
            expected_content
        );
        fs::remove_file(physical_path).unwrap();
    }

    #[test]
    fn failed_node_build_keeps_stderr_for_diagnostics() {
        let (stdout, stderr) = normalize_shell_output_streams(
            "pnpm build",
            1,
            "building...\n",
            "error: build failed\n",
        );

        assert_eq!(stdout, "building...\n");
        assert_eq!(stderr, "error: build failed\n");
    }

    #[test]
    fn go_build_download_progress_is_normalized_to_stdout_on_success() {
        let (stdout, stderr) = normalize_shell_output_streams(
            "go build ./...",
            0,
            "",
            "go: downloading github.com/foo/bar v1.0.0\n",
        );

        assert!(stdout.contains("go: downloading github.com/foo/bar v1.0.0"));
        assert!(stderr.is_empty());
    }

    #[test]
    fn go_build_download_progress_stays_on_stderr_on_failure() {
        let (stdout, stderr) = normalize_shell_output_streams(
            "go build ./...",
            1,
            "",
            "go: downloading github.com/foo/bar v1.0.0\nbuild failed\n",
        );

        assert!(stdout.is_empty());
        assert!(stderr.contains("go: downloading github.com/foo/bar v1.0.0"));
        assert!(stderr.contains("build failed"));
    }

    #[test]
    fn compound_commands_do_not_reclassify_successful_stderr() {
        let node_command = "pnpm build\ngit status";
        let node_stderr = "build warning\n";
        let (node_stdout, node_remaining_stderr) =
            normalize_shell_output_streams(node_command, 0, "On branch main\n", node_stderr);
        assert_eq!(node_stdout, "On branch main\n");
        assert_eq!(node_remaining_stderr, node_stderr);

        let go_command = "go build ./...\ngit status";
        let go_stderr = "go: downloading github.com/foo/bar v1.0.0\n";
        let (go_stdout, go_remaining_stderr) =
            normalize_shell_output_streams(go_command, 0, "On branch main\n", go_stderr);
        assert_eq!(go_stdout, "On branch main\n");
        assert_eq!(go_remaining_stderr, go_stderr);
        assert!(!should_render_stderr_line_as_stdout(
            go_command,
            "go: downloading github.com/foo/bar v1.0.0"
        ));
    }

    #[test]
    fn node_build_stderr_waits_for_exit_code_before_stream_classification() {
        assert!(!should_render_stderr_line_as_stdout(
            "pnpm build",
            "(!) Some chunks are larger than 500 kB after minification. Consider:"
        ));
        assert!(!should_render_stderr_line_as_stdout(
            "cd app && pnpm build",
            "error: failed to build"
        ));
    }

    #[test]
    fn go_build_streaming_stderr_download_line_is_stdout_like() {
        assert!(should_render_stderr_line_as_stdout(
            "go build ./...",
            "go: downloading github.com/foo/bar v1.0.0"
        ));
        assert!(!should_render_stderr_line_as_stdout(
            "go build ./...",
            "some actual error"
        ));
    }

    #[test]
    fn node_build_llm_content_summarizes_assets_and_preserves_complete_output() {
        let stdout = "vite v6.0.0 building for production...\nbin/index.html 0.76 kB\nbin/assets/index.js 3,303.40 kB │ gzip: 1,023.37 kB\n(!) Some chunks are larger than 500 kB after minification. Consider:\n- Use dynamic import() to code-split the application\n✓ built in 15.99s\n";
        for command in [
            "pnpm build",
            "pnpm run build",
            "npm build",
            "npm run build",
            "yarn build",
            "yarn run build",
            "pnpm tauri build",
            "pnpm run tauri build",
            "npm tauri build",
            "npm run tauri build",
            "yarn tauri build",
            "yarn run tauri build",
            "yarm tauri build",
        ] {
            let result = build_shell_tool_result(command, 0, stdout, "");
            let structured = result
                .structured_content
                .expect("structured content missing");
            let llm_content = structured["llm_content"]
                .as_str()
                .expect("llm_content should be a string");

            assert_eq!(
                llm_content,
                "Exit code: 0\n\nBuild result:\nBuild output: 2 files, 3.23 MB (gzip: 1023.37 kB across 1 file)\n✓ built in 15.99s",
                "unexpected LLM output for {command}"
            );
            assert!(!llm_content.contains("bin/assets/index.js"));
            assert!(!llm_content.contains("Some chunks are larger"));
            assert_eq!(
                structured["persisted_output"]["reason"].as_str(),
                Some("reduced")
            );

            let ai_path = structured["persisted_output"]["path"]
                .as_str()
                .expect("persisted output path missing");
            fs::remove_file(resolve_ai_temp_path(Path::new(ai_path))).unwrap();
        }
    }

    #[test]
    fn webpack_and_cra_builds_reduce_assets_and_persist_complete_output() {
        let builds = [
            (
                "npm run build",
                "asset main.js 244 KiB [emitted] [minimized] (name: main)\nasset main.css 12.5 KiB [emitted] [minimized]\nWARNING in asset size limit: The following asset(s) exceed the recommended size limit (244 KiB).\nwebpack 5.95.0 compiled successfully in 8432 ms\n",
                "Build output: 2 files, 256.50 kB",
                "webpack 5.95.0 compiled successfully in 8432 ms",
            ),
            (
                "yarn build",
                "Creating an optimized production build...\nCompiled successfully.\n\nFile sizes after gzip:\n\n  46.6 kB  build/static/js/main.abc.js\n  1.77 kB  build/static/css/main.def.css\n\nThe build folder is ready to be deployed.\n",
                "Build output: 2 files (gzip: 48.37 kB across 2 files)",
                "Compiled successfully.",
            ),
        ];

        for (command, stdout, summary, completion) in builds {
            let result = build_shell_tool_result(command, 0, stdout, "");
            let structured = result
                .structured_content
                .expect("structured content missing");
            let llm_content = structured["llm_content"]
                .as_str()
                .expect("llm_content should be a string");

            assert!(
                llm_content.contains(summary),
                "unexpected LLM output for {command}"
            );
            assert!(
                llm_content.contains(completion),
                "unexpected LLM output for {command}"
            );
            assert!(!llm_content.contains("asset main.js"));
            assert!(!llm_content.contains("build/static/js/main.abc.js"));
            assert!(!llm_content.contains("WARNING in asset size limit"));
            assert_eq!(
                structured["persisted_output"]["reason"].as_str(),
                Some("reduced")
            );

            let ai_path = structured["persisted_output"]["path"]
                .as_str()
                .expect("persisted output path missing");
            fs::remove_file(resolve_ai_temp_path(Path::new(ai_path))).unwrap();
        }
    }

    #[test]
    fn failed_node_build_llm_content_uses_uniform_diagnostic_tail() {
        let stdout = (1..=35)
            .map(|line| format!("build output {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let result = build_shell_tool_result(
            "yarn run tauri build",
            1,
            &stdout,
            "error: failed to bundle application\n",
        );
        let structured = result
            .structured_content
            .expect("structured content missing");
        let llm_content = structured["llm_content"]
            .as_str()
            .expect("llm_content should be a string");

        assert!(llm_content.starts_with(
            "Exit code: 1\n\nBuild failed. Diagnostic tail:\n[truncated previous output]"
        ));
        assert!(llm_content.contains("error: failed to bundle application"));
        assert!(!llm_content.contains("build output 1\n"));
        assert_eq!(
            structured["persisted_output"]["reason"].as_str(),
            Some("reduced")
        );

        let ai_path = structured["persisted_output"]["path"]
            .as_str()
            .expect("persisted output path missing");
        fs::remove_file(resolve_ai_temp_path(Path::new(ai_path))).unwrap();
    }

    #[test]
    fn git_diff_llm_content_retains_patch_hunks_and_lines() {
        let stdout = format!(
            "diff --git a/src/main.rs b/src/main.rs\nindex 123..456 100644\n--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,102 +1,102 @@\n-old\n+new\n{}\ndiff --git a/src/lib.rs b/src/lib.rs\nindex 789..abc 100644\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,102 +1,102 @@\n-old\n+new\n{}",
            (1..=100)
                .map(|line| format!(" unchanged main context {line}"))
                .collect::<Vec<_>>()
                .join("\n"),
            (1..=100)
                .map(|line| format!(" unchanged lib context {line}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        let result = build_shell_tool_result("git diff HEAD~1", 0, &stdout, "");
        let structured = result
            .structured_content
            .expect("structured content missing");
        let llm_content = structured["llm_content"]
            .as_str()
            .expect("llm_content should be a string");

        assert!(llm_content.contains("Changes:"));
        assert!(llm_content.contains("src/main.rs"));
        assert!(llm_content.contains("src/lib.rs"));
        assert!(llm_content.contains("@@ -1,102 +1,102 @@"));
        assert!(llm_content.contains("-old"));
        assert!(llm_content.contains("+new"));

        let ai_path = structured["persisted_output"]["path"]
            .as_str()
            .expect("persisted output path missing");
        fs::remove_file(resolve_ai_temp_path(Path::new(ai_path))).unwrap();
    }

    #[test]
    fn explicitly_shaped_head_output_is_preserved_for_llm() {
        let stdout = (1..=180)
            .map(|i| format!("knowledge line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let result = build_shell_tool_result("ksp load abc123 2>&1 | head -200", 0, &stdout, "");
        let structured = result
            .structured_content
            .expect("structured content missing");
        let llm_content = structured["llm_content"]
            .as_str()
            .expect("llm_content should be a string");

        assert!(llm_content.contains("knowledge line 1"));
        assert!(llm_content.contains("knowledge line 180"));
        assert!(!llm_content.contains("[truncated previous output]"));
    }

    #[test]
    fn oversized_explicitly_shaped_output_keeps_head_and_tail() {
        let stdout = (1..=400)
            .map(|i| format!("slice {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let result = build_shell_tool_result("sed -n '1,400p' notes.md", 0, &stdout, "");
        let structured = result
            .structured_content
            .expect("structured content missing");
        let llm_content = structured["llm_content"]
            .as_str()
            .expect("llm_content should be a string");

        assert!(llm_content.contains("slice 1"));
        assert!(llm_content.contains("slice 400"));
        assert!(llm_content.contains("[truncated middle output]"));
    }

    #[test]
    fn ksp_output_under_15k_is_preserved_for_llm() {
        let stdout = (1..=180)
            .map(|i| format!("knowledge result line {:03} {}", i, "x".repeat(40)))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(stdout.chars().count() < super::KSP_LLM_PRESERVE_CHARS);

        let result =
            build_shell_tool_result("ksp search --keywords \"ctp,spi\" --json", 0, &stdout, "");
        let structured = result
            .structured_content
            .expect("structured content missing");
        let llm_content = structured["llm_content"]
            .as_str()
            .expect("llm_content should be a string");

        assert_eq!(llm_content, format!("Exit code: 0\n\nstdout:\n{}", stdout));
        assert!(!llm_content.contains("[truncated previous output]"));
    }

    #[test]
    fn ksp_output_over_15k_still_uses_generic_reduction() {
        let stdout = (1..=320)
            .map(|i| format!("knowledge result line {:03} {}", i, "y".repeat(80)))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(stdout.chars().count() > super::KSP_LLM_PRESERVE_CHARS);

        let result = build_shell_tool_result("ksp load ctp-callback-threading", 0, &stdout, "");
        let structured = result
            .structured_content
            .expect("structured content missing");
        let llm_content = structured["llm_content"]
            .as_str()
            .expect("llm_content should be a string");

        assert!(llm_content.starts_with("[truncated previous output]"));
        assert!(llm_content.contains("knowledge result line 320"));

        if let Some(path) = structured["persisted_output"]["path"].as_str() {
            fs::remove_file(resolve_ai_temp_path(Path::new(path))).unwrap();
        }
    }
}
