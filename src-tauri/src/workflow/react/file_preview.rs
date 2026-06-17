use serde_json::{json, Value};
use std::cmp::{max, min};
use std::fs;
use std::path::{Path, PathBuf};

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

pub fn normalize_preview_details(value: Value) -> Value {
    match value {
        Value::String(text) => decode_preview_json_string(&text).unwrap_or(Value::String(text)),
        other => other,
    }
}

pub fn attach_write_file_overwrite_old_content(
    preview_args: &mut Value,
    primary_root: Option<&Path>,
) {
    let Some(preview_obj) = preview_args.as_object_mut() else {
        return;
    };

    let overwrite = preview_obj
        .get("overwrite")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if !overwrite || preview_obj.get("old_string").is_some() {
        return;
    }

    let Some(file_path) = preview_obj
        .get("file_path")
        .and_then(|value| value.as_str())
    else {
        return;
    };

    let resolved_path = resolve_preview_file_path(file_path, primary_root);
    let Ok(old_content) = fs::read_to_string(resolved_path) else {
        return;
    };

    preview_obj.insert("old_string".to_string(), json!(old_content));
}

pub fn render_preview_details_text(details: &Value, display_type: &str) -> String {
    match display_type {
        "diff" => render_diff_preview_text(details),
        "markdown" => details
            .as_str()
            .map(ToString::to_string)
            .unwrap_or_else(|| serde_json::to_string_pretty(details).unwrap_or_default()),
        _ => details
            .as_str()
            .map(ToString::to_string)
            .unwrap_or_else(|| serde_json::to_string_pretty(details).unwrap_or_default()),
    }
}

pub fn attach_display_context(
    preview_args: &mut Value,
    prefer_updated_content: bool,
    primary_root: Option<&Path>,
) {
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

    let resolved_path = resolve_preview_file_path(file_path, primary_root);

    let Ok(file_content) = fs::read_to_string(&resolved_path) else {
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

fn resolve_preview_file_path(file_path: &str, primary_root: Option<&Path>) -> PathBuf {
    let path = Path::new(file_path);
    if path.is_absolute() {
        return path.to_path_buf();
    }

    if let Some(root) = primary_root {
        return root.join(path);
    }

    path.to_path_buf()
}

fn decode_preview_json_string(value: &str) -> Option<Value> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    let looks_like_json = trimmed.starts_with('{')
        || trimmed.starts_with('[')
        || (trimmed.starts_with('"') && (trimmed.contains('{') || trimmed.contains('[')));
    if !looks_like_json {
        return None;
    }

    let mut current = trimmed.to_string();
    for _ in 0..2 {
        let Ok(parsed) = serde_json::from_str::<Value>(&current) else {
            break;
        };

        match parsed {
            Value::String(inner) => current = inner,
            other => return Some(other),
        }
    }

    None
}

fn render_diff_preview_text(details: &Value) -> String {
    let Some(data) = details.as_object() else {
        return serde_json::to_string_pretty(details).unwrap_or_default();
    };

    let path = data
        .get("display_path")
        .or_else(|| data.get("file_path"))
        .or_else(|| data.get("path"))
        .and_then(|value| value.as_str())
        .unwrap_or("file");
    let start_line = data
        .get("start_line")
        .and_then(|value| value.as_u64())
        .unwrap_or(1);
    let old_str = data
        .get("old_string")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let new_str = data
        .get("new_string")
        .and_then(|value| value.as_str())
        .or_else(|| data.get("content").and_then(|value| value.as_str()))
        .unwrap_or("");

    let mut parts = vec![
        format!("File: {}", path),
        format!("Start line: {}", start_line),
    ];

    if let Some(before) = data
        .get("context_before")
        .and_then(|value| value.as_array())
    {
        if !before.is_empty() {
            parts.push("Context before:".to_string());
            parts.extend(
                before
                    .iter()
                    .filter_map(|line| line.as_str())
                    .map(ToString::to_string),
            );
        }
    }

    if !old_str.is_empty() {
        parts.push("<old_string>".to_string());
        parts.push(old_str.to_string());
        parts.push("</old_string>".to_string());
    }

    if !new_str.is_empty() {
        parts.push("<new_string>".to_string());
        parts.push(new_str.to_string());
        parts.push("</new_string>".to_string());
    }

    if let Some(after) = data.get("context_after").and_then(|value| value.as_array()) {
        if !after.is_empty() {
            parts.push("Context after:".to_string());
            parts.extend(
                after
                    .iter()
                    .filter_map(|line| line.as_str())
                    .map(ToString::to_string),
            );
        }
    }

    parts.join("\n")
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn attach_display_context_resolves_relative_paths_from_primary_root() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("file_preview_test_{unique}"));
        let nested = root.join("src");
        fs::create_dir_all(&nested).unwrap();
        let file_path = nested.join("example.txt");
        fs::write(
            &file_path,
            "line1\nline2\nmatch start\nmatch end\nline5\nline6\n",
        )
        .unwrap();

        let mut preview_args = json!({
            "file_path": "src/example.txt",
            "old_string": "match start\nmatch end",
            "new_string": "updated start\nupdated end"
        });

        attach_display_context(&mut preview_args, false, Some(root.as_path()));

        assert_eq!(
            preview_args.get("start_line").and_then(|v| v.as_u64()),
            Some(3)
        );
        assert_eq!(
            preview_args
                .get("context_after_start_line")
                .and_then(|v| v.as_u64()),
            Some(5)
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn attach_write_file_overwrite_old_content_reads_existing_file() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("file_preview_overwrite_test_{unique}"));
        fs::create_dir_all(&root).unwrap();
        let file_path = root.join("demo.txt");
        fs::write(&file_path, "original content").unwrap();

        let mut preview_args = json!({
            "file_path": "demo.txt",
            "content": "new content",
            "overwrite": true
        });

        attach_write_file_overwrite_old_content(&mut preview_args, Some(root.as_path()));

        assert_eq!(
            preview_args
                .get("old_string")
                .and_then(|value| value.as_str()),
            Some("original content")
        );

        let _ = fs::remove_dir_all(&root);
    }
}
