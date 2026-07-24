use crate::libs::ai_temp::{
    persist_large_tool_output, persist_tool_output, LARGE_TOOL_OUTPUT_CHAR_LIMIT,
};
use crate::tools::helper::{
    detect_json_stdout, is_go_build_or_test_command, is_node_build_command, reduce_command_output,
    reduce_priority_log_output, should_reduce_priority_log,
};
use crate::tools::ToolCallResult;
use serde_json::json;

const MAX_DISPLAY_CHARS: usize = 30_000;
const GENERIC_LLM_MAX_LINES: usize = 40;
const EXPLICIT_SHAPED_LLM_MAX_LINES: usize = 240;
const EXPLICIT_SHAPED_LLM_MAX_CHARS: usize = 20_000;
const KSP_LLM_PRESERVE_CHARS: usize = 15_000;
const BUILD_LLM_TAIL_LINES: usize = 20;
const GIT_LOG_HEAD_LINES: usize = 30;
const GIT_DIFF_MAX_FILES: usize = 20;

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
    let json_output = detect_json_stdout(&normalized_stdout, &normalized_stderr);
    let command_reduction = if json_output.is_none() {
        reduce_command_output(&normalized_command, exit_code, &raw_content)
    } else {
        None
    };
    let (display_content, llm_content, output_was_reduced) = if let Some(json_output) = json_output
    {
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
        let llm_content = if let Some(reduction) = command_reduction.as_ref() {
            reduction.content.clone()
        } else {
            reduce_shell_output_for_llm(
                &normalized_command,
                exit_code,
                &normalized_stdout,
                &normalized_stderr,
                &raw_content,
            )
        };
        let output_was_reduced = command_reduction
            .as_ref()
            .is_some_and(|reduction| reduction.persist_complete_output)
            || llm_content != raw_content;
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
    let normalized_command = normalize_command(command_str);
    is_go_build_or_test_command(&normalized_command) && line.starts_with("go: downloading ")
}

pub(crate) fn normalize_shell_output_streams(
    command_str: &str,
    exit_code: i32,
    stdout: &str,
    stderr: &str,
) -> (String, String) {
    let stdout = strip_ansi_escape_sequences(stdout);
    let stderr = strip_ansi_escape_sequences(stderr);

    if exit_code != 0 || stderr.trim().is_empty() {
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

fn reduce_shell_output_for_llm(
    normalized_command: &str,
    exit_code: i32,
    stdout: &str,
    stderr: &str,
    raw_content: &str,
) -> String {
    if normalized_command.contains("git show") || normalized_command.contains("git diff") {
        return reduce_git_diff_like_output(exit_code, stdout, stderr, raw_content);
    }

    if normalized_command.contains("git log") {
        return reduce_git_log_output(raw_content);
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
    fn git_show_llm_content_omits_full_diff() {
        let stdout = "commit abc\nAuthor: test\nDate: today\n\ndiff --git a/src/main.rs b/src/main.rs\n@@ -1 +1 @@\n-old\n+new\ndiff --git a/src/lib.rs b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new\n";
        let result = build_shell_tool_result("git show HEAD", 0, stdout, "");
        let structured = result
            .structured_content
            .expect("structured content missing");
        let llm_content = structured["llm_content"]
            .as_str()
            .expect("llm_content should be a string");

        assert!(llm_content.contains("Changed files:"));
        assert!(llm_content.contains("src/main.rs"));
        assert!(llm_content.contains("src/lib.rs"));
        assert!(llm_content.contains("full diff omitted"));
        assert!(!llm_content.contains("@@ -1 +1 @@"));

        let ai_path = structured["persisted_output"]["path"]
            .as_str()
            .expect("persisted output path missing");
        fs::remove_file(resolve_ai_temp_path(Path::new(ai_path))).unwrap();
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
    fn git_diff_llm_content_omits_patch_body() {
        let stdout = "diff --git a/src/main.rs b/src/main.rs\n@@ -1 +1 @@\n-old\n+new\ndiff --git a/src/lib.rs b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new\n";
        let result = build_shell_tool_result("git diff HEAD~1", 0, stdout, "");
        let structured = result
            .structured_content
            .expect("structured content missing");
        let llm_content = structured["llm_content"]
            .as_str()
            .expect("llm_content should be a string");

        assert!(llm_content.contains("Changed files:"));
        assert!(llm_content.contains("src/main.rs"));
        assert!(!llm_content.contains("@@ -1 +1 @@"));

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
