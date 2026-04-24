use serde_json::{json, Value};
use std::cmp::{max, min};
use std::fs;

const CONTEXT_LINE_COUNT: usize = 5;

pub fn merge_tool_result_into_preview_args(
    preview_args: &mut Value,
    structured_result: Option<&Value>,
    content_result: Option<&str>,
) {
    if let Some(result_obj) = structured_result.and_then(|value| value.as_object()) {
        if let Some(preview_obj) = preview_args.as_object_mut() {
            for (key, value) in result_obj {
                preview_obj.insert(key.clone(), value.clone());
            }
        }
        return;
    }

    if let Some(result_text) = content_result {
        if let Ok(Value::Object(result_obj)) = serde_json::from_str::<Value>(result_text) {
            if let Some(preview_obj) = preview_args.as_object_mut() {
                for (key, value) in result_obj {
                    preview_obj.insert(key, value);
                }
            }
        }
    }
}

pub fn attach_display_context(preview_args: &mut Value, prefer_updated_content: bool) {
    let Some(preview_obj) = preview_args.as_object_mut() else {
        return;
    };

    let replace_all = preview_obj
        .get("replace_all")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if replace_all {
        return;
    }

    let Some(file_path) = preview_obj
        .get("file_path")
        .or_else(|| preview_obj.get("path"))
        .and_then(|value| value.as_str())
    else {
        return;
    };

    let Ok(file_content) = fs::read_to_string(file_path) else {
        return;
    };

    let preferred_block = if prefer_updated_content {
        preview_obj
            .get("new_string")
            .and_then(|value| value.as_str())
            .or_else(|| preview_obj.get("content").and_then(|value| value.as_str()))
            .filter(|value| !value.is_empty())
    } else {
        preview_obj
            .get("old_string")
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .or_else(|| preview_obj.get("content").and_then(|value| value.as_str()))
            .filter(|value| !value.is_empty())
    };

    let fallback_block = if prefer_updated_content {
        preview_obj
            .get("old_string")
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
    } else {
        preview_obj
            .get("new_string")
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
    };

    let block = preferred_block.or(fallback_block);
    let block_line_count = block.map(line_count).unwrap_or(1);
    let explicit_start_line = preview_obj
        .get("start_line")
        .and_then(|value| value.as_u64())
        .and_then(|value| usize::try_from(value).ok());

    let derived_start_line = block
        .and_then(|value| locate_block_start_line(&file_content, value))
        .or(explicit_start_line);

    let Some(start_line) = derived_start_line else {
        return;
    };

    let lines: Vec<&str> = file_content.lines().collect();
    if lines.is_empty() {
        return;
    }

    let safe_start_line = max(1, min(start_line, lines.len()));
    let before_start_line = safe_start_line.saturating_sub(CONTEXT_LINE_COUNT);
    let before_slice_start = before_start_line.saturating_sub(1);
    let before_slice_end = safe_start_line.saturating_sub(1);
    let before_lines: Vec<String> = lines[before_slice_start..before_slice_end]
        .iter()
        .map(|line| (*line).to_string())
        .collect();

    let after_start_line = min(safe_start_line + block_line_count, lines.len() + 1);
    let after_slice_start = after_start_line.saturating_sub(1);
    let after_slice_end = min(after_slice_start + CONTEXT_LINE_COUNT, lines.len());
    let after_lines: Vec<String> = if after_slice_start < after_slice_end {
        lines[after_slice_start..after_slice_end]
            .iter()
            .map(|line| (*line).to_string())
            .collect()
    } else {
        Vec::new()
    };

    preview_obj.insert("context_before".to_string(), json!(before_lines));
    preview_obj.insert("start_line".to_string(), json!(safe_start_line));
    preview_obj.insert(
        "context_before_start_line".to_string(),
        json!(before_start_line.max(1)),
    );
    preview_obj.insert("context_after".to_string(), json!(after_lines));
    preview_obj.insert(
        "context_after_start_line".to_string(),
        json!(after_start_line),
    );
    preview_obj.insert(
        "display_context_line_count".to_string(),
        json!(CONTEXT_LINE_COUNT),
    );
}

fn locate_block_start_line(file_content: &str, block: &str) -> Option<usize> {
    if block.is_empty() {
        return None;
    }

    if let Some(index) = find_first_match_index(file_content, block) {
        return Some(file_content[..index].lines().count() + 1);
    }

    let normalized_file = file_content.replace("\r\n", "\n");
    let normalized_block = block.replace("\r\n", "\n");
    normalized_file
        .find(&normalized_block)
        .map(|index| normalized_file[..index].lines().count() + 1)
}

fn find_first_match_index(file_content: &str, block: &str) -> Option<usize> {
    if let Some(index) = file_content.find(block) {
        return Some(index);
    }

    let windows_block = block.replace('\n', "\r\n");
    if windows_block != block {
        return file_content.find(&windows_block);
    }

    None
}

fn line_count(text: &str) -> usize {
    max(1, text.lines().count())
}
