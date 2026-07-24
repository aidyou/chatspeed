use std::collections::{BTreeSet, HashSet};

const PRIORITY_LOG_MIN_LINES: usize = 50;
const PRIORITY_LOG_HEAD_LINES: usize = 3;
const PRIORITY_LOG_TAIL_LINES: usize = 10;
const PRIORITY_LOG_MAX_WARNINGS: usize = 5;
const PRIORITY_LOG_MAX_SUMMARIES: usize = 10;

pub(crate) fn should_reduce_priority_log(content: &str) -> bool {
    content.lines().count() >= PRIORITY_LOG_MIN_LINES && is_likely_log_output(content)
}

pub(crate) fn reduce_priority_log_output(raw_content: &str) -> String {
    let lines = raw_content.lines().collect::<Vec<_>>();
    let mut selected = BTreeSet::new();
    let mut warning_lines = BTreeSet::new();
    let mut summary_lines = BTreeSet::new();
    let mut seen_warnings = HashSet::new();

    selected.extend(0..lines.len().min(PRIORITY_LOG_HEAD_LINES));
    selected.extend(lines.len().saturating_sub(PRIORITY_LOG_TAIL_LINES)..lines.len());

    for (index, line) in lines.iter().enumerate() {
        if is_error_or_failure_line(line) {
            selected.insert(index);
            if index > 0 {
                selected.insert(index - 1);
            }
            if index + 1 < lines.len() {
                selected.insert(index + 1);
            }
        } else if is_warning_line(line)
            && warning_lines.len() < PRIORITY_LOG_MAX_WARNINGS
            && seen_warnings.insert(normalize_log_line(line))
        {
            warning_lines.insert(index);
            selected.insert(index);
        } else if is_summary_line(line) && summary_lines.len() < PRIORITY_LOG_MAX_SUMMARIES {
            summary_lines.insert(index);
            selected.insert(index);
        }
    }

    if selected.len() == lines.len() {
        return raw_content.to_string();
    }

    let mut output = String::new();
    let mut previous_index = None;
    for index in selected {
        if previous_index.is_some_and(|previous| index > previous + 1) {
            output.push_str("[omitted log lines]\n");
        }
        output.push_str(lines[index]);
        output.push('\n');
        previous_index = Some(index);
    }
    output.push_str(
        "[full log output omitted for LLM context; inspect the saved output for complete details]",
    );
    output
}

fn is_likely_log_output(content: &str) -> bool {
    content
        .lines()
        .filter(|line| {
            is_error_or_failure_line(line) || is_warning_line(line) || is_summary_line(line)
        })
        .take(3)
        .count()
        > 0
}

fn is_error_or_failure_line(line: &str) -> bool {
    let line = line.to_ascii_lowercase();
    line.contains("error")
        || line.contains("failed")
        || line.contains("failure")
        || line.contains("fatal")
        || line.contains("panic")
        || line.contains("traceback")
        || line.contains("exception")
}

fn is_warning_line(line: &str) -> bool {
    let line = line.to_ascii_lowercase();
    line.contains("warning") || line.contains(" warn ") || line.starts_with("warn")
}

fn is_summary_line(line: &str) -> bool {
    let line = line.to_ascii_lowercase();
    line.contains("test result:")
        || line.contains("tests passed")
        || line.contains("tests failed")
        || line.contains("compiled successfully")
        || line.contains("build completed")
        || line.contains("finished ")
        || line.contains("passed")
        || line.contains("failed")
}

fn normalize_log_line(line: &str) -> String {
    line.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::{reduce_priority_log_output, should_reduce_priority_log};

    #[test]
    fn retains_early_errors_context_summaries_and_unique_warnings() {
        let output = (1..=70)
            .map(|line| match line {
                5 => "before important error".to_string(),
                6 => "error: required dependency is missing".to_string(),
                7 => "after important error".to_string(),
                20 | 21 => "warning: deprecated option".to_string(),
                40 => "test result: FAILED. 1 passed; 1 failed".to_string(),
                _ => format!("progress line {line}"),
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(should_reduce_priority_log(&output));
        let reduced = reduce_priority_log_output(&output);
        assert!(reduced.contains("before important error"));
        assert!(reduced.contains("error: required dependency is missing"));
        assert!(reduced.contains("after important error"));
        assert_eq!(reduced.matches("warning: deprecated option").count(), 1);
        assert!(reduced.contains("test result: FAILED"));
        assert!(reduced.contains("[omitted log lines]"));
        assert!(reduced.contains("full log output omitted"));
    }

    #[test]
    fn ignores_long_plain_text() {
        let output = (1..=60)
            .map(|line| format!("plain text line {line}"))
            .collect::<Vec<_>>()
            .join("\n");

        assert!(!should_reduce_priority_log(&output));
    }
}
