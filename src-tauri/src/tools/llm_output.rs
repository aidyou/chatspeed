const DEFAULT_PATH_PREVIEW_LINES: usize = 200;
const DEFAULT_GREP_CONTENT_LINES: usize = 120;
const DEFAULT_GREP_FILE_LINES: usize = 160;

pub(crate) fn preview_path_lines_for_llm(entries: &[String]) -> Option<String> {
    preview_lines_with_notice(entries, DEFAULT_PATH_PREVIEW_LINES)
}

pub(crate) fn preview_grep_lines_for_llm(entries: &[String], output_mode: &str) -> Option<String> {
    let max_lines = if output_mode == "content" {
        DEFAULT_GREP_CONTENT_LINES
    } else {
        DEFAULT_GREP_FILE_LINES
    };
    preview_lines_with_notice(entries, max_lines)
}

fn preview_lines_with_notice(lines: &[String], max_lines: usize) -> Option<String> {
    if lines.is_empty() {
        return None;
    }

    if lines.len() <= max_lines {
        return Some(lines.join("\n"));
    }

    let omitted = lines.len().saturating_sub(max_lines);
    Some(format!(
        "{}\n[truncated {} additional lines for LLM context; inspect further only if needed]",
        lines[..max_lines].join("\n"),
        omitted
    ))
}

#[cfg(test)]
mod tests {
    use super::{preview_grep_lines_for_llm, preview_path_lines_for_llm};

    #[test]
    fn path_preview_keeps_head_and_adds_notice() {
        let lines = (1..=250).map(|i| format!("entry-{}", i)).collect::<Vec<_>>();
        let preview = preview_path_lines_for_llm(&lines).expect("preview should exist");

        assert!(preview.contains("entry-1"));
        assert!(preview.contains("entry-200"));
        assert!(!preview.contains("entry-250"));
        assert!(preview.contains("truncated 50 additional lines"));
    }

    #[test]
    fn grep_preview_uses_content_limit() {
        let lines = (1..=130).map(|i| format!("file:{}:match", i)).collect::<Vec<_>>();
        let preview =
            preview_grep_lines_for_llm(&lines, "content").expect("preview should exist");

        assert!(preview.contains("file:1:match"));
        assert!(preview.contains("file:120:match"));
        assert!(!preview.contains("file:130:match"));
    }
}
