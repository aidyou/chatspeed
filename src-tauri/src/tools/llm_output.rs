const DEFAULT_PATH_PREVIEW_LINES: usize = 200;
const DEFAULT_GREP_CONTENT_LINES: usize = 120;
const DEFAULT_GREP_FILE_LINES: usize = 160;
const MAX_LLM_PREVIEW_CHARS: usize = 20_000;
const LLM_CONTEXT_TRUNCATION_NOTICE: &str =
    "[truncated for LLM context; narrow the query before inspecting more output]";

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

    let visible_lines = lines.len().min(max_lines);
    let preview = lines[..visible_lines].join("\n");
    let line_limit_notice = if lines.len() > max_lines {
        Some(format!(
            "[truncated {} additional lines for LLM context; inspect further only if needed]",
            lines.len().saturating_sub(max_lines)
        ))
    } else {
        None
    };

    if let Some(notice) = line_limit_notice {
        if preview.chars().count() + 1 + notice.chars().count() <= MAX_LLM_PREVIEW_CHARS {
            return Some(format!("{preview}\n{notice}"));
        }
    } else if preview.chars().count() <= MAX_LLM_PREVIEW_CHARS {
        return Some(preview);
    }

    let max_preview_chars = MAX_LLM_PREVIEW_CHARS
        .saturating_sub(LLM_CONTEXT_TRUNCATION_NOTICE.chars().count())
        .saturating_sub(1);
    let bounded_preview: String = preview.chars().take(max_preview_chars).collect();
    Some(format!(
        "{bounded_preview}\n{LLM_CONTEXT_TRUNCATION_NOTICE}"
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        preview_grep_lines_for_llm, preview_path_lines_for_llm, LLM_CONTEXT_TRUNCATION_NOTICE,
        MAX_LLM_PREVIEW_CHARS,
    };

    #[test]
    fn path_preview_keeps_head_and_adds_notice() {
        let lines = (1..=250)
            .map(|i| format!("entry-{}", i))
            .collect::<Vec<_>>();
        let preview = preview_path_lines_for_llm(&lines).expect("preview should exist");

        assert!(preview.contains("entry-1"));
        assert!(preview.contains("entry-200"));
        assert!(!preview.contains("entry-250"));
        assert!(preview.contains("truncated 50 additional lines"));
    }

    #[test]
    fn grep_preview_uses_content_limit() {
        let lines = (1..=130)
            .map(|i| format!("file:{}:match", i))
            .collect::<Vec<_>>();
        let preview = preview_grep_lines_for_llm(&lines, "content").expect("preview should exist");

        assert!(preview.contains("file:1:match"));
        assert!(preview.contains("file:120:match"));
        assert!(!preview.contains("file:130:match"));
    }

    #[test]
    fn previews_bound_a_few_oversized_entries_by_characters() {
        let entries = vec![format!(
            "very-long-entry:{}",
            "x".repeat(MAX_LLM_PREVIEW_CHARS * 2)
        )];

        let path_preview = preview_path_lines_for_llm(&entries).expect("path preview should exist");
        let grep_preview =
            preview_grep_lines_for_llm(&entries, "content").expect("grep preview should exist");

        for preview in [path_preview, grep_preview] {
            assert!(preview.starts_with("very-long-entry:"));
            assert!(preview.contains(LLM_CONTEXT_TRUNCATION_NOTICE));
            assert!(preview.chars().count() <= MAX_LLM_PREVIEW_CHARS);
        }
    }
}
