use crate::ai::traits::chat::MCPToolDeclaration;
use crate::tools::llm_output::preview_path_lines_for_llm;
use crate::tools::{NativeToolResult, ToolCallResult, ToolCategory, ToolDefinition, ToolError};
use crate::workflow::react::security::PathGuard;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

const PLANNING_NOTE_FILE: &str = "note.md";
pub(crate) const DEFAULT_READ_FILE_LIMIT: usize = 800;
const READ_FILE_MAX_LINE_LENGTH: usize = 10_000;
pub(crate) const READ_FILE_MAX_OUTPUT_CHARS: usize = 18_000;

fn primary_directory(path_guard: Option<&Arc<RwLock<PathGuard>>>) -> PathBuf {
    path_guard
        .and_then(|guard| guard.read().ok())
        .and_then(|guard| guard.get_primary_root().map(PathBuf::from))
        .or_else(|| std::env::current_dir().ok())
        .and_then(|path| fs::canonicalize(&path).ok().or(Some(path)))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn resolve_tool_path(path_str: &str, path_guard: Option<&Arc<RwLock<PathGuard>>>) -> PathBuf {
    let path = PathBuf::from(path_str);
    if path.is_absolute() {
        path
    } else {
        primary_directory(path_guard).join(path)
    }
}

fn display_path_for_tool_output(
    path: &Path,
    path_guard: Option<&Arc<RwLock<PathGuard>>>,
) -> String {
    let primary_dir = primary_directory(path_guard);
    let canonical_path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if let Ok(relative) = canonical_path.strip_prefix(&primary_dir) {
        if relative.as_os_str().is_empty() {
            ".".to_string()
        } else {
            relative.to_string_lossy().to_string()
        }
    } else {
        path.to_string_lossy().to_string()
    }
}

fn format_read_file_open_error(path_str: &str, error: &std::io::Error) -> ToolError {
    match error.kind() {
        std::io::ErrorKind::NotFound => ToolError::IoError(format!(
            "File not found: {}. Verify the path and use 'list_dir' to inspect nearby directories.",
            path_str
        )),
        std::io::ErrorKind::PermissionDenied => ToolError::IoError(format!(
            "Permission denied while opening file: {}. Check file permissions or ask the user to grant access.",
            path_str
        )),
        _ => ToolError::IoError(format!(
            "Failed to open file {}: {}",
            path_str, error
        )),
    }
}

fn should_skip_list_dir_entry(name: &str) -> bool {
    let name_lower = name.to_lowercase();
    name == "node_modules"
        || name == ".git"
        || name == "__pycache__"
        || name_lower.ends_with(".pyc")
        || name_lower == "thumbs.db"
        || name_lower == ".ds_store"
}

fn planning_note_path(planning_root: &Path) -> PathBuf {
    planning_root.join(PLANNING_NOTE_FILE)
}

fn timestamp_millis() -> Result<u128, ToolError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .map_err(|e| ToolError::IoError(format!("Failed to read system clock: {}", e)))
}

fn sanitize_backup_name_component(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '_' | '-' => ch,
            _ => '_',
        })
        .collect();
    if sanitized.is_empty() {
        "file".to_string()
    } else {
        sanitized
    }
}

fn overwrite_backup_path(path: &Path) -> Result<PathBuf, ToolError> {
    let timestamp = timestamp_millis()?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            ToolError::InvalidParams(format!(
                "Cannot create overwrite backup for path without a valid file name: {}",
                path.display()
            ))
        })?;
    let sanitized_name = sanitize_backup_name_component(file_name);
    Ok(std::env::temp_dir().join(format!(
        "chatspeed-write-file-backup-{sanitized_name}-{timestamp}.bak"
    )))
}

#[derive(Clone)]
struct ListedEntry {
    display_path: String,
    is_dir: bool,
}

fn sort_listed_entries(entries: &mut [ListedEntry]) {
    entries.sort_by(|left, right| compare_listed_entries(left, right));
}

fn listed_entry_segment_sort_key<'a>(
    part: &'a str,
    is_dir_at_level: bool,
) -> (u8, String, &'a str) {
    let kind_rank = if is_dir_at_level { 0 } else { 1 };
    (kind_rank, part.to_ascii_lowercase(), part)
}

fn compare_listed_entries(left: &ListedEntry, right: &ListedEntry) -> std::cmp::Ordering {
    let left_parts: Vec<&str> = left
        .display_path
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    let right_parts: Vec<&str> = right
        .display_path
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    let shared_len = left_parts.len().min(right_parts.len());

    for index in 0..shared_len {
        let left_is_dir_at_level = index + 1 < left_parts.len() || left.is_dir;
        let right_is_dir_at_level = index + 1 < right_parts.len() || right.is_dir;
        let left_key = listed_entry_segment_sort_key(left_parts[index], left_is_dir_at_level);
        let right_key = listed_entry_segment_sort_key(right_parts[index], right_is_dir_at_level);
        let segment_order = left_key.cmp(&right_key);
        if segment_order != std::cmp::Ordering::Equal {
            return segment_order;
        }
    }

    if left_parts.len() != right_parts.len() {
        return left_parts.len().cmp(&right_parts.len());
    }

    if left.is_dir != right.is_dir {
        return if left.is_dir {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Greater
        };
    }

    left.display_path
        .to_ascii_lowercase()
        .cmp(&right.display_path.to_ascii_lowercase())
        .then_with(|| left.display_path.cmp(&right.display_path))
}

fn execute_read_file(
    path_str: &str,
    offset: usize,
    limit: usize,
    path_guard: Option<&Arc<RwLock<PathGuard>>>,
) -> NativeToolResult {
    if limit == 0 {
        return Err(ToolError::InvalidParams(
            "limit must be greater than 0".to_string(),
        ));
    }

    let resolved_path = resolve_tool_path(path_str, path_guard);
    let display_path = display_path_for_tool_output(&resolved_path, path_guard);
    let path = resolved_path.as_path();

    if path.is_dir() {
        return Err(ToolError::InvalidParams(format!(
            "Path is a directory, not a file: {}. Use 'list_dir' to inspect directories.",
            display_path
        )));
    }

    let file = fs::File::open(path).map_err(|e| format_read_file_open_error(&display_path, &e))?;
    let reader = BufReader::new(file);

    let file_size_bytes = fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let mut lines = Vec::new();
    let mut raw_lines = Vec::new();
    let mut total_chars = 0usize;
    let end_offset = offset.saturating_add(limit);
    let mut truncated_by_limit = false;
    let mut truncated_by_size = false;
    let mut total_lines = 0usize;
    let mut next_offset: Option<usize> = None;

    for (i, line) in reader.lines().enumerate() {
        total_lines = i + 1;
        if i < offset {
            continue;
        }
        if i >= end_offset {
            truncated_by_limit = true;
            next_offset = Some(end_offset);
            continue;
        }
        let content = line.map_err(|e| {
            ToolError::IoError(format!(
                "Failed to read line {} from {}: {}",
                i + 1,
                display_path,
                e
            ))
        })?;
        if truncated_by_size {
            continue;
        }
        if content.len() > READ_FILE_MAX_LINE_LENGTH {
            return Err(ToolError::ExecutionFailed(format!(
                "Line {} is too long. Use 'grep' or read_file with a smaller targeted range.",
                i + 1
            )));
        }
        let rendered_line = format!("{:>6}\t{}", i + 1, content);
        let rendered_len = rendered_line.chars().count() + 1;
        if total_chars + rendered_len > READ_FILE_MAX_OUTPUT_CHARS {
            truncated_by_size = true;
            next_offset = Some(i);
            continue;
        }
        total_chars += rendered_len;
        lines.push(rendered_line);
        raw_lines.push(content);
    }

    let lines_returned = lines.len();
    if lines.is_empty() {
        return Ok(ToolCallResult::success(
            Some(
                "[No content returned. The file is empty or the requested offset is beyond EOF.]"
                    .into(),
            ),
            Some(json!({
                "file_path": resolved_path.to_string_lossy(),
                "display_path": display_path,
                "offset": offset,
                "limit": limit,
                "lines_returned": 0,
                "truncated": false,
                "file_size_bytes": file_size_bytes,
                "total_lines": total_lines
            })),
        ));
    }

    let truncated = truncated_by_limit || truncated_by_size;
    let last_line_number = offset + lines_returned;
    let llm_content = build_read_file_llm_content(
        &display_path,
        offset,
        last_line_number,
        &raw_lines,
        truncated_by_limit,
        truncated_by_size,
        total_lines,
        next_offset,
    );

    if truncated_by_size {
        let truncated_body = lines.join("\n");
        let resume_offset = next_offset.unwrap_or(end_offset);
        let reminder = format!(
            "<SYSTEM_REMINDER>Partial file through line {} of {} ({} bytes). Continue with read_file offset={} and a narrower range.</SYSTEM_REMINDER>",
            last_line_number,
            total_lines,
            file_size_bytes,
            resume_offset
        );
        return Ok(ToolCallResult::success(
            Some(format!(
                "<truncated_content>\n{}\n</truncated_content>\n{}",
                truncated_body, reminder
            )),
            Some(json!({
                "file_path": resolved_path.to_string_lossy(),
                "display_path": display_path,
                "offset": offset,
                "limit": limit,
                "lines_returned": lines_returned,
                "truncated": true,
                "truncated_by_size": truncated_by_size,
                "truncated_by_limit": truncated_by_limit,
                "llm_content": llm_content,
                "next_offset": resume_offset,
                "file_size_bytes": file_size_bytes,
                "total_lines": total_lines
            })),
        ));
    } else if truncated_by_limit {
        let resume_offset = next_offset.unwrap_or(end_offset);
        lines.push(format!(
            "<SYSTEM_REMINDER>Stopped at line {} of {} due to the requested limit. Continue with read_file offset={} if more exact content is needed.</SYSTEM_REMINDER>",
            last_line_number,
            total_lines,
            resume_offset
        ));
    } else if offset > 0 {
        lines.push(format!(
            "<SYSTEM_REMINDER>EOF reached at line {} of {}. Do not reuse offset={}; use a lower offset or grep if you need earlier context.</SYSTEM_REMINDER>",
            offset + lines_returned,
            total_lines,
            offset
        ));
    }

    Ok(ToolCallResult::success(
        Some(lines.join("\n")),
        Some(json!({
            "file_path": resolved_path.to_string_lossy(),
            "display_path": display_path,
            "offset": offset,
            "limit": limit,
            "lines_returned": lines_returned,
            "truncated": truncated,
            "llm_content": llm_content,
            "next_offset": Value::Null,
            "file_size_bytes": file_size_bytes,
            "total_lines": total_lines
        })),
    ))
}

fn build_read_file_llm_content(
    display_path: &str,
    offset: usize,
    last_line_number: usize,
    raw_lines: &[String],
    truncated_by_limit: bool,
    truncated_by_size: bool,
    total_lines: usize,
    next_offset: Option<usize>,
) -> String {
    let exact_content = raw_lines.join("\n");
    let escaped_path = escape_xml_attribute(display_path);
    let mut sections = vec![
        format!(
            "<file_content path=\"{}\" offset=\"{}\" end_line=\"{}\" total_lines=\"{}\" truncated=\"{}\">",
            escaped_path,
            offset,
            last_line_number,
            total_lines,
            if truncated_by_limit || truncated_by_size {
                "true"
            } else {
                "false"
            }
        ),
        exact_content,
        "</file_content>".to_string(),
        "<SYSTEM_REMINDER>`file_content` contains the exact file bytes for the returned line range, without read_file line-number prefixes. When calling edit_file, copy old_string from inside <file_content> only. For a follow-up read after this block, use read_file offset=end_line.</SYSTEM_REMINDER>".to_string(),
    ];

    if truncated_by_size {
        sections.push(format!(
            "<SYSTEM_REMINDER>Partial file through line {} of {}. Continue with read_file offset={} and a narrower range if more exact content is needed.</SYSTEM_REMINDER>",
            last_line_number,
            total_lines,
            next_offset.unwrap_or(offset + raw_lines.len())
        ));
    } else if truncated_by_limit {
        sections.push(format!(
            "<SYSTEM_REMINDER>Stopped at line {} of {} due to the requested limit. Continue with read_file offset={} if more exact content is needed.</SYSTEM_REMINDER>",
            last_line_number,
            total_lines,
            next_offset.unwrap_or(offset + raw_lines.len())
        ));
    }

    sections.join("\n")
}

fn escape_xml_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn execute_edit_file(
    path_str: &str,
    old_str_unix: &str,
    new_str_unix: &str,
    replace_all: bool,
    path_guard: Option<&Arc<RwLock<PathGuard>>>,
) -> NativeToolResult {
    if old_str_unix == new_str_unix {
        return Err(ToolError::InvalidParams(
            "old_string and new_string are identical. No changes performed.".into(),
        ));
    }

    let resolved_path = resolve_tool_path(path_str, path_guard);
    let raw_content = fs::read_to_string(&resolved_path).map_err(|e| {
        ToolError::IoError(format!(
            "Read failed: {}. Ensure the file exists and is readable.",
            e
        ))
    })?;

    let mut final_content = String::new();
    let mut match_found = false;
    let old_str_win = old_str_unix.replace("\n", "\r\n");
    let new_str_win = new_str_unix.replace("\n", "\r\n");
    let mut start_line = 0;

    if raw_content.contains(&old_str_win) {
        let matches: Vec<usize> = raw_content
            .match_indices(&old_str_win)
            .map(|(i, _)| i)
            .collect();
        if !replace_all && matches.len() > 1 {
            return Err(ToolError::ExecutionFailed(format!(
                "The old_string is not unique (found {} matches with Windows line endings). Please provide more surrounding context to uniquely identify the location.",
                matches.len()
            )));
        }

        start_line = raw_content[..matches[0]].lines().count();
        final_content = if replace_all {
            raw_content.replace(&old_str_win, &new_str_win)
        } else {
            raw_content.replacen(&old_str_win, &new_str_win, 1)
        };
        match_found = true;
    } else if raw_content.contains(old_str_unix) {
        let matches: Vec<usize> = raw_content
            .match_indices(old_str_unix)
            .map(|(i, _)| i)
            .collect();
        if !replace_all && matches.len() > 1 {
            return Err(ToolError::ExecutionFailed(format!(
                "The old_string is not unique (found {} matches with Unix line endings). Please provide more surrounding context to uniquely identify the location.",
                matches.len()
            )));
        }

        start_line = raw_content[..matches[0]].lines().count();
        final_content = if replace_all {
            raw_content.replace(old_str_unix, new_str_unix)
        } else {
            raw_content.replacen(old_str_unix, new_str_unix, 1)
        };
        match_found = true;
    }

    if !match_found {
        let normalized_file = raw_content.replace("\r\n", "\n");
        if normalized_file.contains(old_str_unix) {
            let matches: Vec<usize> = normalized_file
                .match_indices(old_str_unix)
                .map(|(i, _)| i)
                .collect();
            if !replace_all && matches.len() > 1 {
                return Err(ToolError::ExecutionFailed(format!(
                    "The old_string is not unique (found {} matches after normalization). Please provide more surrounding context.",
                    matches.len()
                )));
            }

            start_line = normalized_file[..matches[0]].lines().count();
            let replaced_normalized = if replace_all {
                normalized_file.replace(old_str_unix, new_str_unix)
            } else {
                normalized_file.replacen(old_str_unix, new_str_unix, 1)
            };

            final_content = if raw_content.contains("\r\n") {
                replaced_normalized.replace("\n", "\r\n")
            } else {
                replaced_normalized
            };
            match_found = true;
        }
    }

    if !match_found {
        let lines_count = raw_content.lines().count();
        return Err(ToolError::ExecutionFailed(format!(
            "The old_string was not found in the file (checked {} lines). Please ensure you copied the text EXACTLY, including all whitespace and indentation.",
            lines_count
        )));
    }

    fs::write(&resolved_path, &final_content).map_err(|e| {
        ToolError::IoError(format!("Edit write failed: {}. Check file permissions.", e))
    })?;

    let result_json = json!({
        "file_path": resolved_path.to_string_lossy(),
        "display_path": display_path_for_tool_output(&resolved_path, path_guard),
        "old_string": old_str_unix,
        "new_string": new_str_unix,
        "replace_all": replace_all,
        "start_line": start_line + 1,
    });

    Ok(ToolCallResult::success(
        Some(serde_json::to_string(&result_json).unwrap_or_default()),
        Some(result_json),
    ))
}

#[derive(Clone, Default)]
pub struct ReadFile {
    path_guard: Option<Arc<RwLock<PathGuard>>>,
}

impl ReadFile {
    pub fn new(path_guard: Option<Arc<RwLock<PathGuard>>>) -> Self {
        Self { path_guard }
    }
}

#[async_trait]
impl ToolDefinition for ReadFile {
    fn name(&self) -> &str {
        crate::tools::TOOL_READ_FILE
    }
    fn description(&self) -> &str {
        "Reads a file from the local filesystem. You can access any file directly by using this tool.\n\
        Assume this tool is able to read all files on the machine. If the user provides a path to a file assume that path is valid. \
        It is okay to read a file that does not exist; an error will be returned.\n\n\
        Usage:\n\
        - Paths under the primary working directory may be provided as relative paths\n\
        - For files outside the primary working directory but still within other authorized directories, use absolute paths\n\
        - By default, it reads up to 800 lines starting from the beginning of the file\n\
        - Use offset and limit for long files or after grep finds a relevant line. offset is zero-based: offset=0 starts at line 1, offset=200 starts at line 201\n\
        - Any single line longer than 10000 characters returns an error instead of being truncated\n\
        - Total returned file content is capped below the workflow observation truncation threshold; if exceeded, the tool returns <truncated_content> plus a SYSTEM_REMINDER with file size, total lines, and the next suggested offset\n\
        - Results are returned using cat -n style format: right-aligned 1-based line number, tab, then line content\n\
        - In workflow LLM context, this tool also provides a structured `<file_content path=\"...\" offset=\"...\" end_line=\"...\" total_lines=\"...\" truncated=\"...\">...</file_content>` block containing the exact returned file content without line-number prefixes\n\
        - If output stops because limit was reached, a SYSTEM_REMINDER tells you the next offset to continue from\n\
        - If output reaches EOF before the limit, a SYSTEM_REMINDER tells you not to reread the same offset\n\
        - This tool can only read text files, not directories. To inspect a directory, use list_dir.\n\
        - You can call multiple tools in a single response. It is always better to speculatively read multiple potentially useful files in parallel.\n\
        - If no content is returned, the tool will explicitly say whether the file appears empty or the offset is beyond EOF."
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::FileSystem
    }

    fn scope(&self) -> crate::tools::ToolScope {
        crate::tools::ToolScope::Workflow
    }

    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string", "description": "Path to the file to read. Use a relative path for files under the primary working directory; use an absolute path for files in other authorized directories." },
                    "offset": { "type": "integer", "description": "Zero-based line offset to start reading from. offset=0 starts at line 1; offset=200 starts at line 201. Only provide if the file is too large to read at once.", "default": 0, "minimum": 0 },
                    "limit": { "type": "integer", "description": "Number of lines to read. When omitted, defaults to 800. This is not a hard maximum; larger values are allowed, but output may still be truncated by workflow observation limits. Use smaller ranges for large files.", "default": 800, "minimum": 1 }
                },
                "required": ["file_path"]
            }),
            output_schema: None,
            disabled: false,
            scope: Some(self.scope()),
        }
    }
    async fn call(&self, params: Value) -> NativeToolResult {
        let path_str = params["file_path"]
            .as_str()
            .ok_or(ToolError::InvalidParams(
                "file_path is required".to_string(),
            ))?;
        let offset = params["offset"].as_u64().unwrap_or(0) as usize;
        let limit = params["limit"]
            .as_u64()
            .unwrap_or(DEFAULT_READ_FILE_LIMIT as u64) as usize;
        execute_read_file(path_str, offset, limit, self.path_guard.as_ref())
    }
}

#[derive(Clone, Default)]
pub struct WriteFile {
    path_guard: Option<Arc<RwLock<PathGuard>>>,
}

impl WriteFile {
    pub fn new(path_guard: Option<Arc<RwLock<PathGuard>>>) -> Self {
        Self { path_guard }
    }
}

#[async_trait]
impl ToolDefinition for WriteFile {
    fn name(&self) -> &str {
        crate::tools::TOOL_WRITE_FILE
    }
    fn description(&self) -> &str {
        "Writes a file to the local filesystem.\n\n\
        Usage:\n\
        - This tool is only for creating brand-new files.\n\
        - If a file already exists at the provided path, this tool fails unless you explicitly pass `overwrite: true`.\n\
        - When `overwrite: true` is used, the existing file is first backed up to a timestamped file in the system temporary directory before the new content is written.\n\
        - Parent directories are created automatically when they do not already exist.\n\
        - Use a relative path when creating files under the primary working directory.\n\
        - For other authorized directories, use an absolute path.\n\
        - ALWAYS prefer editing existing files in the codebase. Only use this tool when you are creating a new file that does not already exist.\n\
        - NEVER proactively create documentation files (*.md) or README files. Only create documentation files if explicitly requested by the user.\n\
        - Only use emojis if the user explicitly requests it. Avoid writing emojis to files unless asked."
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::FileSystem
    }

    fn scope(&self) -> crate::tools::ToolScope {
        crate::tools::ToolScope::Workflow
    }

    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string", "description": "Path to the new file. Use a relative path under the primary working directory, or an absolute path for other authorized directories. Parent directories are created automatically." },
                    "content": { "type": "string", "description": "The content to write to the file" },
                    "overwrite": { "type": "boolean", "description": "Allow replacing an existing file. When true, the existing file is backed up to a timestamped file in the system temporary directory first.", "default": false }
                },
                "required": ["file_path", "content"]
            }),
            output_schema: None,
            disabled: false,
            scope: Some(self.scope()),
        }
    }
    async fn call(&self, params: Value) -> NativeToolResult {
        let path_str = params["file_path"]
            .as_str()
            .ok_or(ToolError::InvalidParams(
                "file_path is required".to_string(),
            ))?;
        let content = params["content"]
            .as_str()
            .ok_or(ToolError::InvalidParams("content is required".to_string()))?;
        let overwrite = params["overwrite"].as_bool().unwrap_or(false);
        let path = resolve_tool_path(path_str, self.path_guard.as_ref());
        let mut backup_path: Option<PathBuf> = None;
        let mut overwritten = false;

        if path.exists() {
            if path.is_dir() {
                return Err(ToolError::InvalidParams(format!(
                    "file_path points to a directory, not a file: {}",
                    path.display()
                )));
            }

            if !overwrite {
                return Err(ToolError::InvalidParams(format!(
                    "file_path already exists: {}. `write_file` can only create new files unless `overwrite: true` is provided; use `edit_file` for precise modifications",
                    path.display()
                )));
            }

            let backup = overwrite_backup_path(&path)?;
            fs::copy(&path, &backup).map_err(|e| {
                ToolError::IoError(format!(
                    "Failed to create overwrite backup {}: {}",
                    backup.display(),
                    e
                ))
            })?;
            backup_path = Some(backup);
            overwritten = true;
        }

        // Ensure parent directories exist for new files
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok();
        }

        fs::write(&path, content)
            .map_err(|e| ToolError::IoError(format!("Write failed: {}", e)))?;

        Ok(ToolCallResult::success(
            Some(if overwritten {
                "File written successfully; existing file was backed up first.".to_string()
            } else {
                "New file created successfully.".to_string()
            }),
            Some(json!({
                "file_path": path.to_string_lossy(),
                "display_path": display_path_for_tool_output(&path, self.path_guard.as_ref()),
                "bytes_written": content.len(),
                "overwritten": overwritten,
                "backup_path": backup_path.as_ref().map(|value| value.to_string_lossy().to_string())
            })),
        ))
    }
}

#[derive(Clone, Default)]
pub struct EditFile {
    path_guard: Option<Arc<RwLock<PathGuard>>>,
}

impl EditFile {
    pub fn new(path_guard: Option<Arc<RwLock<PathGuard>>>) -> Self {
        Self { path_guard }
    }
}

#[async_trait]
impl ToolDefinition for EditFile {
    fn name(&self) -> &str {
        crate::tools::TOOL_EDIT_FILE
    }
    fn description(&self) -> &str {
        "Performs exact string replacements in files.\n\n\
        Usage:\n\
        - Ensure you have viewed the relevant file content (e.g., via `read_file` or user-provided context) to confirm exact text and indentation before editing.\n\
        - When editing, preserve the exact indentation (tabs/spaces). If you used `read_file`, prefer copying `old_string` from inside the structured `<file_content ...>...</file_content>` block in LLM context because that block contains the exact file content without display line numbers.\n\
        - The visible `read_file` output still includes `cat -n` style prefixes for readability. Never include any part of that display line-number prefix or separator in `old_string` or `new_string`.\n\
        - ALWAYS prefer editing existing files in the codebase. NEVER write new files unless explicitly required.\n\
        - Only use emojis if the user explicitly requests it. Avoid adding emojis to files unless asked.\n\
        - The edit will FAIL if `old_string` is not unique in the file. Either provide a larger string with more surrounding context to make it unique or use `replace_all` to change every instance of `old_string`.\n\
        - Use `replace_all` only when every occurrence of `old_string` should be changed, such as safe file-wide literal updates.\n\
        - When multiple independent edits are needed in the same file, issue them as batched `edit_file` tool calls in the same turn when the tool system supports parallel/batched calls.\n\
        - Batch same-file edit calls only when the edits are independent, precise, and do not rely on the result of another edit in the same batch.\n\
        - Use sequential `edit_file` calls with re-reading when edits depend on previous changes, affect overlapping or uncertain regions, require updated context, or would be harder to review as a batch.\n\
        - Keep edits precise and minimal. Do not combine unrelated files, unrelated regions, or unrelated behavior changes just to reduce tool calls.\n\
        - The tool preserves existing CRLF/LF line endings when possible. If an exact match fails because of line-ending differences, it tries a normalized fallback."
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::FileSystem
    }

    fn scope(&self) -> crate::tools::ToolScope {
        crate::tools::ToolScope::Workflow
    }

    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string", "description": "Path to the file to modify. Use a relative path for files under the primary working directory; use an absolute path for files in other authorized directories." },
                    "old_string": { "type": "string", "description": "The text to replace" },
                    "new_string": { "type": "string", "description": "The text to replace it with (must be different from old_string)" },
                    "replace_all": { "type": "boolean", "description": "Replace all occurrences of old_string (default false)", "default": false }
                },
                "required": ["file_path", "old_string", "new_string"]
            }),
            output_schema: None,
            disabled: false,
            scope: Some(self.scope()),
        }
    }
    async fn call(&self, params: Value) -> NativeToolResult {
        let path_str = params["file_path"]
            .as_str()
            .ok_or(ToolError::InvalidParams(
                "file_path is required".to_string(),
            ))?;
        let old_str_unix = params["old_string"]
            .as_str()
            .ok_or(ToolError::InvalidParams(
                "old_string is required".to_string(),
            ))?;
        let new_str_unix = params["new_string"]
            .as_str()
            .ok_or(ToolError::InvalidParams(
                "new_string is required".to_string(),
            ))?;
        let replace_all = params["replace_all"].as_bool().unwrap_or(false);
        execute_edit_file(
            path_str,
            old_str_unix,
            new_str_unix,
            replace_all,
            self.path_guard.as_ref(),
        )
    }
}

pub struct PlanReadNote {
    planning_root: PathBuf,
}

impl PlanReadNote {
    pub fn new(planning_root: PathBuf) -> Self {
        Self { planning_root }
    }
}

#[async_trait]
impl ToolDefinition for PlanReadNote {
    fn name(&self) -> &str {
        crate::tools::TOOL_PLAN_READ_NOTE
    }

    fn description(&self) -> &str {
        "Reads the fixed planning note from `.cs/note.md` in the active workspace during strict manual plan mode.\n\
        This tool can only access that planning note and cannot read arbitrary workspace files.\n\
        In workflow LLM context, it also provides the exact returned note content inside a structured `<file_content ...>...</file_content>` block for precise follow-up edits.\n\
        Use this tool to review your planning draft or research notes before calling `submit_plan`."
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::FileSystem
    }

    fn scope(&self) -> crate::tools::ToolScope {
        crate::tools::ToolScope::Workflow
    }

    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "offset": { "type": "integer", "default": 0, "minimum": 0 },
                    "limit": { "type": "integer", "default": 800, "minimum": 1 }
                },
                "additionalProperties": false
            }),
            output_schema: None,
            disabled: false,
            scope: Some(self.scope()),
        }
    }

    async fn call(&self, params: Value) -> NativeToolResult {
        let offset = params["offset"].as_u64().unwrap_or(0) as usize;
        let limit = params["limit"]
            .as_u64()
            .unwrap_or(DEFAULT_READ_FILE_LIMIT as u64) as usize;
        let path = planning_note_path(&self.planning_root);
        execute_read_file(&path.to_string_lossy(), offset, limit, None)
    }
}

pub struct PlanWriteNote {
    planning_root: PathBuf,
}

impl PlanWriteNote {
    pub fn new(planning_root: PathBuf) -> Self {
        Self { planning_root }
    }
}

#[async_trait]
impl ToolDefinition for PlanWriteNote {
    fn name(&self) -> &str {
        crate::tools::TOOL_PLAN_WRITE_NOTE
    }

    fn description(&self) -> &str {
        "Creates or fully replaces the fixed planning note at `.cs/note.md` in strict manual plan mode.\n\
        This tool is only for planning artifacts, not workspace implementation.\n\
        Use it to capture structured notes, draft the proposed plan, or persist investigation output."
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::FileSystem
    }

    fn scope(&self) -> crate::tools::ToolScope {
        crate::tools::ToolScope::Workflow
    }

    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "content": { "type": "string", "description": "Complete file content to store in the planning note" }
                },
                "required": ["content"],
                "additionalProperties": false
            }),
            output_schema: None,
            disabled: false,
            scope: Some(self.scope()),
        }
    }

    async fn call(&self, params: Value) -> NativeToolResult {
        let content = params["content"]
            .as_str()
            .ok_or(ToolError::InvalidParams("content is required".to_string()))?;
        let path = planning_note_path(&self.planning_root);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                ToolError::IoError(format!("Failed to prepare planning directory: {}", e))
            })?;
        }
        fs::write(&path, content)
            .map_err(|e| ToolError::IoError(format!("Write failed: {}", e)))?;

        Ok(ToolCallResult::success(
            Some("Planning note written successfully.".to_string()),
            Some(json!({
                "file_path": path.to_string_lossy(),
                "note_name": PLANNING_NOTE_FILE,
                "bytes_written": content.len()
            })),
        ))
    }
}

pub struct PlanEditNote {
    planning_root: PathBuf,
}

impl PlanEditNote {
    pub fn new(planning_root: PathBuf) -> Self {
        Self { planning_root }
    }
}

#[async_trait]
impl ToolDefinition for PlanEditNote {
    fn name(&self) -> &str {
        crate::tools::TOOL_PLAN_EDIT_NOTE
    }

    fn description(&self) -> &str {
        "Edits the fixed planning note at `.cs/note.md` using exact string replacement.\n\
        This tool cannot touch arbitrary workspace files.\n\
        If you previously used `plan_read_note`, prefer copying `old_string` from inside the structured `<file_content ...>...</file_content>` block in LLM context.\n\
        Use this after `plan_read_note` when you need a precise update to the planning document."
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::FileSystem
    }

    fn scope(&self) -> crate::tools::ToolScope {
        crate::tools::ToolScope::Workflow
    }

    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "old_string": { "type": "string", "description": "The exact existing text to replace" },
                    "new_string": { "type": "string", "description": "The replacement text" },
                    "replace_all": { "type": "boolean", "default": false }
                },
                "required": ["old_string", "new_string"],
                "additionalProperties": false
            }),
            output_schema: None,
            disabled: false,
            scope: Some(self.scope()),
        }
    }

    async fn call(&self, params: Value) -> NativeToolResult {
        let old_string = params["old_string"]
            .as_str()
            .ok_or(ToolError::InvalidParams(
                "old_string is required".to_string(),
            ))?;
        let new_string = params["new_string"]
            .as_str()
            .ok_or(ToolError::InvalidParams(
                "new_string is required".to_string(),
            ))?;
        let replace_all = params["replace_all"].as_bool().unwrap_or(false);
        let path = planning_note_path(&self.planning_root);
        execute_edit_file(
            &path.to_string_lossy(),
            old_string,
            new_string,
            replace_all,
            None,
        )
    }
}

#[derive(Clone, Default)]
pub struct ListDir {
    path_guard: Option<Arc<RwLock<PathGuard>>>,
}

impl ListDir {
    pub fn new(path_guard: Option<Arc<RwLock<PathGuard>>>) -> Self {
        Self { path_guard }
    }
}

#[async_trait]
impl ToolDefinition for ListDir {
    fn name(&self) -> &str {
        crate::tools::TOOL_LIST_DIR
    }
    fn description(&self) -> &str {
        "Lists files and directories in a given directory path. To read file contents, use read_file instead.\n\n\
        Usage:\n\
        - Returns one path per line. Paths under the primary working directory are shown as relative paths; entries in other authorized directories remain absolute.\n\
        - By default, lists only the immediate children of the directory.\n\
        - Set recursive=true to walk descendants recursively.\n\
        - Respects .gitignore and standard ignore filters, while still showing hidden files unless skipped explicitly.\n\
        - Skips common noisy entries such as node_modules, .git, __pycache__, .pyc, .DS_Store, and thumbs.db.\n\
        - Output is capped at 1000 entries."
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::FileSystem
    }

    fn scope(&self) -> crate::tools::ToolScope {
        crate::tools::ToolScope::Workflow
    }

    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory path to list. Use a relative path for the primary working directory; use an absolute path for other authorized directories." },
                    "recursive": { "type": "boolean", "description": "When true, recursively lists descendant entries. Defaults to false.", "default": false }
                },
                "required": ["path"]
            }),
            output_schema: None,
            disabled: false,
            scope: Some(self.scope()),
        }
    }
    async fn call(&self, params: Value) -> NativeToolResult {
        let path_str = params["path"]
            .as_str()
            .ok_or(ToolError::InvalidParams("path is required".to_string()))?;
        let recursive = params["recursive"].as_bool().unwrap_or(false);
        let mut entries = vec![];
        let path = resolve_tool_path(path_str, self.path_guard.as_ref());
        let display_path = display_path_for_tool_output(&path, self.path_guard.as_ref());

        if !path.exists() {
            return Err(ToolError::IoError(format!(
                "Directory not found: {}",
                display_path
            )));
        }
        if !path.is_dir() {
            return Err(ToolError::InvalidParams(format!(
                "Path is a file, not a directory: {}. Use 'read_file' to read file contents.",
                display_path
            )));
        }

        // Use ignore crate to respect .gitignore
        let mut builder = ignore::WalkBuilder::new(&path);
        builder.standard_filters(true).hidden(false);
        if !recursive {
            builder.max_depth(Some(1));
        }

        for result in builder.build() {
            let entry = match result {
                Ok(e) => e,
                Err(_) => continue,
            };

            // Skip the root path itself
            if entry.depth() == 0 {
                continue;
            }

            let name = entry.file_name().to_string_lossy();
            if should_skip_list_dir_entry(&name) {
                continue;
            }

            entries.push(ListedEntry {
                display_path: display_path_for_tool_output(entry.path(), self.path_guard.as_ref()),
                is_dir: entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false),
            });
            if entries.len() >= 1000 {
                break;
            }
        }

        sort_listed_entries(&mut entries);
        let rendered_entries: Vec<String> = entries
            .iter()
            .map(|entry| entry.display_path.clone())
            .collect();

        if rendered_entries.is_empty() {
            Ok(ToolCallResult::success(
                Some("Directory is empty.".into()),
                Some(json!({
                    "path": path.to_string_lossy(),
                    "display_path": display_path,
                    "recursive": recursive,
                    "count": 0,
                    "truncated": false,
                    "llm_content": "[Directory is empty]"
                })),
            ))
        } else {
            let count = rendered_entries.len();
            Ok(ToolCallResult::success(
                Some(rendered_entries.join("\n")),
                Some(json!({
                    "path": path.to_string_lossy(),
                    "display_path": display_path,
                    "recursive": recursive,
                    "count": count,
                    "truncated": count >= 1000,
                    "llm_content": preview_path_lines_for_llm(&rendered_entries)
                })),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::{tempdir, tempdir_in, NamedTempFile};

    fn make_relative_test_dir() -> (tempfile::TempDir, PathBuf) {
        let root = primary_directory(None);
        let temp_dir = tempdir_in(&root).unwrap();
        let relative = temp_dir.path().strip_prefix(&root).unwrap().to_path_buf();
        (temp_dir, relative)
    }

    #[tokio::test]
    async fn test_read_file_basic() {
        let tool = ReadFile::default();
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        // Write some content
        fs::write(&path, "line1\nline2\nline3").unwrap();

        // Read entire file
        let params = json!({
            "file_path": path
        });
        let result = tool.call(params).await.unwrap();
        let output = result.content.unwrap();
        assert!(output.contains("line1"));
        assert!(output.contains("line2"));
        assert!(output.contains("line3"));
    }

    #[tokio::test]
    async fn test_read_file_with_offset_and_limit() {
        let tool = ReadFile::default();
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        // Write 10 lines
        let content: Vec<String> = (1..=10).map(|i| format!("line {}", i)).collect();
        fs::write(&path, content.join("\n")).unwrap();

        // Read lines 3-7 (offset=2, limit=5)
        let params = json!({
            "file_path": path,
            "offset": 2,
            "limit": 5
        });
        let result = tool.call(params).await.unwrap();
        let output = result.content.unwrap();

        // Should contain lines 3-7
        assert!(output.contains("line 3"));
        assert!(output.contains("line 7"));
        assert!(output.contains("offset=7"));
        assert!(!output.contains("<truncated_content>"));
        // Should NOT contain lines 1-2 or 8-10
        assert!(!output.contains("line 1"));
        assert!(!output.contains("line 10"));
    }

    #[tokio::test]
    async fn test_read_file_with_offset_reports_eof() {
        let tool = ReadFile::default();
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        fs::write(&path, "line 1\nline 2\nline 3").unwrap();

        let params = json!({
            "file_path": path,
            "offset": 2,
            "limit": 10
        });
        let result = tool.call(params).await.unwrap();
        let output = result.content.unwrap();

        assert!(output.contains("line 3"));
        assert!(output.contains("Reached EOF after line 3"));
        assert!(output.contains("same offset=2"));
    }

    #[tokio::test]
    async fn test_read_file_empty_file_returns_clear_message() {
        let tool = ReadFile::default();
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        fs::write(&path, "").unwrap();

        let params = json!({
            "file_path": path
        });
        let result = tool.call(params).await.unwrap();
        let output = result.content.unwrap();

        assert!(output.contains("No content returned"));
    }

    #[tokio::test]
    async fn test_read_file_rejects_zero_limit() {
        let tool = ReadFile::default();
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        fs::write(&path, "line1").unwrap();

        let params = json!({
            "file_path": path,
            "limit": 0
        });
        let result = tool.call(params).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            ToolError::InvalidParams(msg) => assert!(msg.contains("limit")),
            other => panic!("Expected InvalidParams, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_read_file_line_length_limit() {
        let tool = ReadFile::default();
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        // Write a line that exceeds MAX_LINE_LENGTH (10,000)
        let long_line = "x".repeat(15000);
        fs::write(&path, long_line).unwrap();

        let params = json!({
            "file_path": path
        });
        let result = tool.call(params).await;
        assert!(result.is_err());
        if let Err(e) = result {
            match e {
                ToolError::ExecutionFailed(msg) => assert!(msg.contains("too long")),
                _ => panic!("Expected ExecutionFailed error"),
            }
        }
    }

    #[tokio::test]
    async fn test_read_file_total_size_limit_returns_truncated_content() {
        let tool = ReadFile::default();
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        let line = "x".repeat(400);
        let content: Vec<String> = (0..120).map(|_| line.clone()).collect();
        fs::write(&path, content.join("\n")).unwrap();

        let params = json!({
            "file_path": path
        });
        let result = tool.call(params).await.unwrap();
        let output = result.content.unwrap();
        let structured = result.structured_content.unwrap();

        assert!(output.contains("<truncated_content>"));
        assert!(output.contains("File size:"));
        assert!(output.contains("total lines:"));
        assert_eq!(structured["truncated_by_size"].as_bool(), Some(true));
        assert!(structured["next_offset"].as_u64().is_some());
        assert_eq!(structured["total_lines"].as_u64(), Some(120));
    }

    #[tokio::test]
    async fn test_read_file_includes_exact_llm_content_without_line_numbers() {
        let tool = ReadFile::default();
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        fs::write(&path, "func test() {\n\tcall()\n    spaced()\n}\n").unwrap();

        let result = tool
            .call(json!({
                "file_path": path
            }))
            .await
            .unwrap();

        let structured = result.structured_content.unwrap();
        let llm_content = structured["llm_content"].as_str().unwrap_or_default();

        assert!(llm_content.contains("<file_content "));
        assert!(llm_content.contains("offset=\"0\""));
        assert!(llm_content.contains("truncated=\"false\""));
        assert!(llm_content.contains("\tcall()"));
        assert!(llm_content.contains("    spaced()"));
        assert!(!llm_content.contains("     2\t"));
    }

    #[tokio::test]
    async fn test_read_file_not_found() {
        let tool = ReadFile::default();
        let params = json!({
            "file_path": "/nonexistent/path/file.txt"
        });
        let result = tool.call(params).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ToolError::IoError(msg) => {
                assert!(msg.contains("File not found"));
                assert!(msg.contains("/nonexistent/path/file.txt"));
            }
            other => panic!("Expected IoError, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_read_file_directory_returns_invalid_params() {
        let tool = ReadFile::default();
        let temp_dir = tempdir().unwrap();
        let params = json!({
            "file_path": temp_dir.path().to_string_lossy().to_string()
        });
        let result = tool.call(params).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ToolError::InvalidParams(msg) => {
                assert!(msg.contains("Path is a directory"));
                assert!(msg.contains("list_dir"));
            }
            other => panic!("Expected InvalidParams, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_read_file_supports_relative_path_from_primary_directory() {
        let tool = ReadFile::default();
        let (_temp_dir, relative_root) = make_relative_test_dir();
        let relative_file = relative_root.join("nested.txt");
        let absolute_file = primary_directory(None).join(&relative_file);

        fs::write(&absolute_file, "alpha\nbeta").unwrap();

        let result = tool
            .call(json!({
                "file_path": relative_file.to_string_lossy().to_string()
            }))
            .await
            .unwrap();

        let output = result.content.unwrap();
        assert!(output.contains("alpha"));
        assert!(output.contains("beta"));
    }

    #[tokio::test]
    async fn test_read_file_default_limit_is_reduced() {
        let tool = ReadFile::default();
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();
        let content: Vec<String> = (1..=850).map(|i| format!("line {}", i)).collect();
        fs::write(&path, content.join("\n")).unwrap();

        let result = tool.call(json!({ "file_path": path })).await.unwrap();
        let output = result.content.unwrap();
        let structured = result.structured_content.unwrap();

        assert!(output.contains("line 800"));
        assert!(!output.contains("line 801"));
        assert_eq!(structured["truncated_by_limit"].as_bool(), Some(true));
        assert_eq!(structured["next_offset"].as_u64(), Some(800));
    }

    #[tokio::test]
    async fn test_write_file_new() {
        let tool = WriteFile::default();
        let temp_dir = tempdir().unwrap();
        let path = temp_dir.path().join("test.txt");
        let path_str = path.to_string_lossy().to_string();

        let content = "Hello, World!";
        let params = json!({
            "file_path": path_str,
            "content": content
        });

        let result = tool.call(params).await.unwrap();
        assert_eq!(result.content.unwrap(), "New file created successfully.");
        let structured = result.structured_content.unwrap();
        assert_eq!(
            structured["bytes_written"].as_u64().unwrap(),
            content.len() as u64
        );
        assert_eq!(structured["overwritten"].as_bool(), Some(false));
        assert!(structured["backup_path"].is_null());

        // Verify file was written
        let actual_content = fs::read_to_string(&path).unwrap();
        assert_eq!(actual_content, content);
    }

    #[tokio::test]
    async fn test_write_file_new_with_overwrite_flag_does_not_report_overwrite() {
        let tool = WriteFile::default();
        let temp_dir = tempdir().unwrap();
        let path = temp_dir.path().join("test.txt");

        let result = tool
            .call(json!({
                "file_path": path.to_string_lossy().to_string(),
                "content": "Hello, World!",
                "overwrite": true
            }))
            .await
            .unwrap();

        assert_eq!(result.content.unwrap(), "New file created successfully.");
        let structured = result.structured_content.unwrap();
        assert_eq!(structured["overwritten"].as_bool(), Some(false));
        assert!(structured["backup_path"].is_null());
    }

    #[tokio::test]
    async fn test_write_file_existing_file_rejected() {
        let tool = WriteFile::default();
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        // Write initial content
        fs::write(&path, "original content").unwrap();

        // Overwrite with new content
        let new_content = "new content";
        let params = json!({
            "file_path": path,
            "content": new_content
        });

        let result = tool.call(params).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ToolError::InvalidParams(msg) => {
                assert!(msg.contains("file_path already exists"));
                assert!(msg.contains("overwrite: true"));
            }
            other => panic!("Expected InvalidParams error, got {:?}", other),
        }

        // Verify existing content is unchanged
        let actual_content = fs::read_to_string(&path).unwrap();
        assert_eq!(actual_content, "original content");
    }

    #[tokio::test]
    async fn test_write_file_existing_file_with_overwrite_creates_backup() {
        let tool = WriteFile::default();
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_path_buf();
        let path_str = path.to_string_lossy().to_string();

        fs::write(&path, "original content").unwrap();

        let result = tool
            .call(json!({
                "file_path": path_str,
                "content": "new content",
                "overwrite": true
            }))
            .await
            .unwrap();

        assert_eq!(
            result.content.unwrap(),
            "File written successfully; existing file was backed up first."
        );
        let structured = result.structured_content.unwrap();
        assert_eq!(structured["overwritten"].as_bool(), Some(true));

        let backup_path = structured["backup_path"]
            .as_str()
            .expect("backup path should be present");
        assert!(backup_path.ends_with(".bak"));
        assert!(backup_path.starts_with(std::env::temp_dir().to_string_lossy().as_ref()));
        assert_eq!(fs::read_to_string(&path).unwrap(), "new content");
        assert_eq!(fs::read_to_string(backup_path).unwrap(), "original content");
    }

    #[tokio::test]
    async fn test_write_file_supports_relative_path_from_primary_directory() {
        let tool = WriteFile::default();
        let (_temp_dir, relative_root) = make_relative_test_dir();
        let relative_file = relative_root.join("new.txt");
        let absolute_file = primary_directory(None).join(&relative_file);

        tool.call(json!({
            "file_path": relative_file.to_string_lossy().to_string(),
            "content": "hello"
        }))
        .await
        .unwrap();

        assert_eq!(fs::read_to_string(&absolute_file).unwrap(), "hello");
    }

    #[tokio::test]
    async fn test_edit_file_basic() {
        let tool = EditFile::default();
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        // Write initial content
        let initial = "Old content\nMore old content";
        fs::write(&path, initial).unwrap();

        // Replace "Old" with "New"
        let params = json!({
            "file_path": path,
            "old_string": "Old",
            "new_string": "New"
        });

        let result = tool.call(params).await.unwrap();
        assert!(result.content.unwrap().contains("\"start_line\""));

        let new_content = fs::read_to_string(&path).unwrap();
        assert_eq!(new_content, "New content\nMore old content");
    }

    #[tokio::test]
    async fn test_edit_file_replace_all() {
        let tool = EditFile::default();
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        let initial = "apple apple apple";
        fs::write(&path, initial).unwrap();

        // Replace all "apple" with "orange"
        let params = json!({
            "file_path": path,
            "old_string": "apple",
            "new_string": "orange",
            "replace_all": true
        });

        let result = tool.call(params).await.unwrap();
        assert!(result.content.unwrap().contains("\"start_line\""));

        let new_content = fs::read_to_string(&path).unwrap();
        assert_eq!(new_content, "orange orange orange");
    }

    #[tokio::test]
    async fn test_edit_file_non_unique_string() {
        let tool = EditFile::default();
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        let initial = "test test test";
        fs::write(&path, initial).unwrap();

        // Try to replace "test" without replace_all - should fail
        let params = json!({
            "file_path": path,
            "old_string": "test",
            "new_string": "updated"
        });

        let result = tool.call(params).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ToolError::ExecutionFailed(msg) => assert!(msg.contains("not unique")),
            _ => panic!("Expected ExecutionFailed error"),
        }
    }

    #[tokio::test]
    async fn test_edit_file_string_not_found() {
        let tool = EditFile::default();
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        fs::write(&path, "some content").unwrap();

        let params = json!({
            "file_path": path,
            "old_string": "nonexistent",
            "new_string": "new"
        });

        let result = tool.call(params).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ToolError::ExecutionFailed(msg) => assert!(msg.contains("not found")),
            _ => panic!("Expected ExecutionFailed error"),
        }
    }

    #[tokio::test]
    async fn test_edit_file_complex_indentation() {
        let tool = EditFile::default();
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        // Mix of spaces and tabs
        let initial = "class MyClass:\n    def method(self):\n\t\tprint('hello')  \n    # end";
        fs::write(&path, initial).unwrap();

        // 1. Test trailing space matching
        let params = json!({
            "file_path": path,
            "old_string": "print('hello')  ", // Matches exact trailing spaces
            "new_string": "print('world')"
        });
        tool.call(params).await.unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("print('world')"));
        assert!(!content.contains("print('hello')"));

        // 2. Test Tab matching
        let params = json!({
            "file_path": path,
            "old_string": "\t\tprint('world')",
            "new_string": "        print('fixed')"
        });
        tool.call(params).await.unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("        print('fixed')"));
    }

    #[tokio::test]
    async fn test_edit_file_multiline_block() {
        let tool = EditFile::default();
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        let initial = "line1\nline2\nline3\nline4\nline5";
        fs::write(&path, initial).unwrap();

        let params = json!({
            "file_path": path,
            "old_string": "line2\nline3\nline4",
            "new_string": "inserted_block"
        });

        tool.call(params).await.unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content, "line1\ninserted_block\nline5");
    }

    #[tokio::test]
    async fn test_edit_file_boundary_conditions() {
        let tool = EditFile::default();
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        let initial = "START_LINE\nmiddle\nEND_LINE";
        fs::write(&path, initial).unwrap();

        // Replace first line
        tool.call(json!({
            "file_path": path,
            "old_string": "START_LINE",
            "new_string": "NEW_START"
        }))
        .await
        .unwrap();

        // Replace last line
        tool.call(json!({
            "file_path": path,
            "old_string": "END_LINE",
            "new_string": "NEW_END"
        }))
        .await
        .unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content, "NEW_START\nmiddle\nNEW_END");
    }

    #[tokio::test]
    async fn test_edit_file_special_and_unicode() {
        let tool = EditFile::default();
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        let initial = "path = \"C:\\\\Users\\\\Test\"\n# 注释: 这是一个表情 🚀";
        fs::write(&path, initial).unwrap();

        // 1. Test slashes and quotes
        tool.call(json!({
            "file_path": path,
            "old_string": "path = \"C:\\\\Users\\\\Test\"",
            "new_string": "path = '/tmp/test'"
        }))
        .await
        .unwrap();

        // 2. Test Unicode and Emoji
        tool.call(json!({
            "file_path": path,
            "old_string": "# 注释: 这是一个表情 🚀",
            "new_string": "# Update: 🛠️"
        }))
        .await
        .unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("path = '/tmp/test'"));
        assert!(content.contains("# Update: 🛠️"));
    }

    #[tokio::test]
    async fn test_edit_file_real_project_snippet() {
        let tool = EditFile::default();
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_string_lossy().to_string();

        // 1. Initial content with EXACT indentation (8 spaces)
        let initial = r#"    fn description(&self) -> &str {
        "Performs exact string replacements in files.\n\n\
        Usage:\n\
        - Ensure you have viewed the full content of the file (e.g., via `read_file` or user-provided context) to confirm exact text and indentation before editing. \n\
        - When editing, ensure you preserve the exact indentation (tabs/spaces). If you used `read_file`, prefer copying old_string from inside the structured <file_content ...>...</file_content> block in LLM context because that block contains the exact file content without display line numbers. Never include any part of the visible read_file line number prefix in the old_string or new_string.\n\
        - ALWAYS prefer editing existing files in the codebase. NEVER write new files unless explicitly required.\n\
        - Only use emojis if the user explicitly requests it. Avoid adding emojis to files unless asked.\n\
        - The edit will FAIL if `old_string` is not unique in the file. Either provide a larger string with more surrounding context to make it unique or use `replace_all` to change every instance of `old_string`.\n\
        - Use `replace_all` for replacing and renaming strings across the file. This parameter is useful if you want to rename a variable for instance."
    }"#;

        fs::write(&path, initial).unwrap();

        // 2. Search pattern must ALSO have EXACTLY 8 spaces to match
        let old_string = r#"        - ALWAYS prefer editing existing files in the codebase. NEVER write new files unless explicitly required.\n\
        - Only use emojis if the user explicitly requests it. Avoid adding emojis to files unless asked.\n\"#;

        let new_string = r#"        - Only use emojis if the user explicitly requests it. Avoid adding emojis to files unless asked.\n\"#;

        let params = json!({
            "file_path": path,
            "old_string": old_string,
            "new_string": new_string
        });

        tool.call(params)
            .await
            .expect("This must pass now because indentation matches exactly");

        let final_content = fs::read_to_string(&path).unwrap();
        assert!(final_content.contains(new_string));
        assert!(!final_content.contains(old_string));
    }

    #[tokio::test]
    async fn test_edit_file_supports_relative_path_from_primary_directory() {
        let tool = EditFile::default();
        let (_temp_dir, relative_root) = make_relative_test_dir();
        let relative_file = relative_root.join("edit.txt");
        let absolute_file = primary_directory(None).join(&relative_file);

        fs::write(&absolute_file, "before value").unwrap();

        tool.call(json!({
            "file_path": relative_file.to_string_lossy().to_string(),
            "old_string": "before",
            "new_string": "after"
        }))
        .await
        .unwrap();

        assert_eq!(fs::read_to_string(&absolute_file).unwrap(), "after value");
    }

    #[tokio::test]
    async fn test_list_dir_basic() {
        let tool = ListDir::default();
        let temp_dir = tempdir().unwrap();
        let path_str = temp_dir.path().to_string_lossy().to_string();

        // Create some files and directories
        fs::write(temp_dir.path().join("file1.txt"), "").unwrap();
        fs::write(temp_dir.path().join("file2.txt"), "").unwrap();
        fs::create_dir(temp_dir.path().join("subdir")).unwrap();

        let params = json!({
            "path": path_str
        });

        let result = tool.call(params).await.unwrap();
        let content = result.content.unwrap();
        let entries: Vec<&str> = content.lines().collect();

        assert!(entries.len() >= 3);

        let path_strs: Vec<String> = entries
            .iter()
            .map(|p| {
                Path::new(p)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            })
            .collect();
        assert!(path_strs.contains(&"file1.txt".to_string()));
        assert!(path_strs.contains(&"file2.txt".to_string()));
        assert_eq!(
            result.structured_content.unwrap()["count"]
                .as_u64()
                .unwrap(),
            entries.len() as u64
        );
    }

    #[tokio::test]
    async fn test_list_dir_sorts_directories_before_files() {
        let tool = ListDir::default();
        let temp_dir = tempdir().unwrap();
        let path_str = temp_dir.path().to_string_lossy().to_string();

        fs::write(temp_dir.path().join("b.php"), "").unwrap();
        fs::write(temp_dir.path().join("a.php"), "").unwrap();
        fs::create_dir(temp_dir.path().join("control")).unwrap();
        fs::create_dir(temp_dir.path().join("app")).unwrap();
        fs::create_dir(temp_dir.path().join(".cs")).unwrap();

        let result = tool.call(json!({ "path": path_str })).await.unwrap();
        let entries: Vec<String> = result
            .content
            .unwrap()
            .lines()
            .map(|line| {
                Path::new(line)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            })
            .collect();

        assert_eq!(
            entries,
            vec![
                ".cs".to_string(),
                "app".to_string(),
                "control".to_string(),
                "a.php".to_string(),
                "b.php".to_string()
            ]
        );
    }

    #[test]
    fn test_sort_listed_entries_handles_non_tree_inputs_without_panicking() {
        let mut entries = vec![
            ListedEntry {
                display_path: "a/y".to_string(),
                is_dir: true,
            },
            ListedEntry {
                display_path: "a/x".to_string(),
                is_dir: false,
            },
            ListedEntry {
                display_path: "a/x/z".to_string(),
                is_dir: false,
            },
        ];

        sort_listed_entries(&mut entries);

        let sorted_paths: Vec<String> = entries
            .iter()
            .map(|entry| entry.display_path.clone())
            .collect();
        assert_eq!(
            sorted_paths,
            vec!["a/x/z".to_string(), "a/y".to_string(), "a/x".to_string()]
        );
    }

    #[tokio::test]
    async fn test_list_dir_recursive() {
        let tool = ListDir::default();
        let temp_dir = tempdir().unwrap();
        let path_str = temp_dir.path().to_string_lossy().to_string();

        // Create nested structure
        let subdir = temp_dir.path().join("subdir");
        fs::create_dir(&subdir).unwrap();
        fs::write(subdir.join("nested.txt"), "").unwrap();

        let params = json!({
            "path": path_str,
            "recursive": true
        });

        let result = tool.call(params).await.unwrap();
        let content = result.content.unwrap();
        let entries: Vec<&str> = content.lines().collect();

        assert!(entries.len() >= 2);
        assert!(entries.iter().any(|p| p.contains("nested.txt")));
    }

    #[tokio::test]
    async fn test_list_dir_nonexistent() {
        let tool = ListDir::default();
        let params = json!({
            "path": "/nonexistent/directory"
        });

        let result = tool.call(params).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ToolError::IoError(msg) => assert!(msg.contains("Directory not found")),
            other => panic!("Expected IoError, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_list_dir_filters_git_and_common_noise() {
        let tool = ListDir::default();
        let temp_dir = tempdir().unwrap();
        let path_str = temp_dir.path().to_string_lossy().to_string();

        fs::create_dir(temp_dir.path().join(".git")).unwrap();
        fs::create_dir(temp_dir.path().join("__pycache__")).unwrap();
        fs::write(temp_dir.path().join("visible.txt"), "").unwrap();

        let params = json!({
            "path": path_str
        });

        let result = tool.call(params).await.unwrap();
        let content = result.content.unwrap();

        assert!(content.contains("visible.txt"));
        assert!(!content.contains(".git"));
        assert!(!content.contains("__pycache__"));
    }

    #[tokio::test]
    async fn test_list_dir_returns_relative_paths_under_primary_directory() {
        let tool = ListDir::default();
        let (_temp_dir, relative_root) = make_relative_test_dir();
        let absolute_root = primary_directory(None).join(&relative_root);
        fs::create_dir(absolute_root.join("subdir")).unwrap();
        fs::write(absolute_root.join("subdir").join("file.txt"), "").unwrap();

        let result = tool
            .call(json!({
                "path": relative_root.to_string_lossy().to_string(),
                "recursive": true
            }))
            .await
            .unwrap();

        let content = result.content.unwrap();
        assert!(content.contains(&format!(
            "{}/subdir/file.txt",
            relative_root.to_string_lossy()
        )));
        assert!(!content.contains(&primary_directory(None).to_string_lossy().to_string()));
    }

    #[tokio::test]
    async fn test_list_dir_includes_llm_content_preview() {
        let tool = ListDir::default();
        let (_temp_dir, relative_root) = make_relative_test_dir();
        let absolute_root = primary_directory(None).join(&relative_root);

        for i in 0..220 {
            fs::write(absolute_root.join(format!("file-{:03}.txt", i)), "").unwrap();
        }

        let result = tool
            .call(json!({
                "path": relative_root.to_string_lossy().to_string()
            }))
            .await
            .unwrap();

        let structured = result.structured_content.unwrap();
        let llm_content = structured["llm_content"].as_str().unwrap_or_default();

        assert!(llm_content.contains("file-000.txt"));
        assert!(llm_content.contains("file-199.txt"));
        assert!(llm_content.contains("truncated 20 additional lines"));
    }
}
