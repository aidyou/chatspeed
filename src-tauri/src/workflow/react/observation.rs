use crate::libs::tsid::TsidGenerator;
use crate::tools::{
    ToolError, TOOL_BASH, TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY, TOOL_EDIT_FILE, TOOL_GLOB,
    TOOL_GREP, TOOL_LIST_DIR, TOOL_PLAN_EDIT_NOTE, TOOL_PLAN_READ_NOTE, TOOL_PLAN_WRITE_NOTE,
    TOOL_READ_FILE, TOOL_SUBMIT_PLAN, TOOL_SUBMIT_RESULT, TOOL_TODO_CREATE, TOOL_TODO_GET,
    TOOL_TODO_LIST, TOOL_TODO_UPDATE, TOOL_WEB_FETCH, TOOL_WEB_SEARCH, TOOL_WRITE_FILE,
};
use crate::workflow::react::file_preview::{
    attach_display_context, merge_tool_result_into_preview_args,
};

use rust_i18n::t;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;

pub struct ObservationReinforcer;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationKind {
    TurnBlockedPostponed,
}

pub struct ReinforcedResult {
    pub content: String,
    pub llm_content: Option<String>,
    pub title: String,
    pub summary: String,
    pub is_error: bool,
    pub error_type: Option<String>,
    pub display_type: String,            // "default", "diff", "text"
    pub approval_status: Option<String>, // "pending", "approved", "rejected", None
    pub observation_kind: Option<ObservationKind>,
}

const TOOL_OUTPUT_CHAR_LIMIT: usize = 20_000;

pub(crate) struct PersistedOverflowPreview {
    content: String,
    next_offset: usize,
}

fn value_to_text(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| serde_json::to_string(value).unwrap_or_default())
}

fn truncate_text_for_preview(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn line_based_preview(value: &str, max_chars: usize) -> PersistedOverflowPreview {
    let mut preview = String::new();
    let mut used_chars = 0;
    let mut next_offset = 0;

    for (index, line) in value.lines().enumerate() {
        let line_chars = line.chars().count();
        let separator_chars = usize::from(!preview.is_empty());
        if used_chars + separator_chars + line_chars > max_chars {
            break;
        }

        if !preview.is_empty() {
            preview.push('\n');
            used_chars += 1;
        }
        preview.push_str(line);
        used_chars += line_chars;
        next_offset = index + 1;
    }

    if preview.is_empty() {
        PersistedOverflowPreview {
            content: truncate_text_for_preview(value, max_chars),
            next_offset: 0,
        }
    } else {
        PersistedOverflowPreview {
            content: preview,
            next_offset,
        }
    }
}

fn persist_web_fetch_overflow(content: &str) -> Option<(String, u64)> {
    let tsid = TsidGenerator::new(1).ok()?.generate().ok()?;
    let path = std::env::temp_dir().join(format!("{}.txt", tsid));
    fs::write(&path, content).ok()?;
    let size = fs::metadata(&path).ok()?.len();
    Some((path.display().to_string(), size))
}

impl ObservationReinforcer {
    /// Reinforces with extra context (like full todo list)
    pub fn reinforce_with_context(
        tool_call: &Value,
        result: &Result<Value, ToolError>,
        extra_context: Option<Value>,
        primary_root: Option<&std::path::Path>,
    ) -> ReinforcedResult {
        // Extract tool name and arguments from the standard tool_call metadata structure
        let (tool_name, args) = if let Some(func) = tool_call.get("function") {
            let name = func
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let args_raw = func.get("arguments").cloned().unwrap_or(Value::Null);
            let args = if let Value::String(ref s) = args_raw {
                serde_json::from_str(s).unwrap_or(args_raw)
            } else {
                args_raw
            };
            (name, args)
        } else {
            (
                tool_call
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown"),
                tool_call
                    .get("arguments")
                    .or_else(|| tool_call.get("input"))
                    .cloned()
                    .unwrap_or(Value::Null),
            )
        };

        match result {
            Ok(val) => {
                let llm_content_override = val
                    .get("structured_content")
                    .and_then(|structured| structured.get("llm_content"))
                    .and_then(|value| value.as_str())
                    .map(str::to_string);
                let raw_res_for_summary =
                    if let Some(content) = val.get("content").and_then(|v| v.as_str()) {
                        content.to_string()
                    } else if let Some(structured) = val.get("structured_content") {
                        serde_json::to_string(structured).unwrap_or_default()
                    } else {
                        serde_json::to_string(val).unwrap_or_default()
                    };

                let title =
                    Self::generate_title(tool_name, &args, extra_context.as_ref(), primary_root);
                let summary = Self::generate_summary(tool_name, &raw_res_for_summary, &args);

                let mut raw_res = raw_res_for_summary;

                // --- Custom Logic for TODO tools (Formatting for AI) ---
                if tool_name == TOOL_TODO_CREATE {
                    if let Some(todos) = extra_context.and_then(|v| v.as_array().cloned()) {
                        let mut list_str = String::from("Task created. Current todo list:\n");
                        for (i, todo) in todos.iter().enumerate() {
                            let subject = todo["subject"].as_str().unwrap_or("Untitled");
                            let status = todo["status"].as_str().unwrap_or("pending");
                            list_str.push_str(&format!("{}. {} ({})\n", i + 1, subject, status));
                        }
                        raw_res = list_str;
                    }
                } else if tool_name == TOOL_TODO_UPDATE {
                    if let Some(todos) = extra_context.and_then(|v| v.as_array().cloned()) {
                        let mut list_str = String::from("Task updated.\n");
                        let next_pending = todos.iter().find(|t| {
                            let s = t["status"].as_str().unwrap_or("");
                            s == "pending" || s == "todo" || s == "in_progress"
                        });

                        if let Some(next) = next_pending {
                            let subject = next["subject"].as_str().unwrap_or("Untitled");
                            list_str.push_str(&format!(
                                "<SYSTEM_REMINDER>Next todo: {}\n</SYSTEM_REMINDER>",
                                subject
                            ));
                        } else {
                            // Treat completed, done, data_missing, and failed as terminal states
                            let all_terminal = todos.iter().all(|t| {
                                matches!(
                                    t["status"].as_str(),
                                    Some("completed" | "done" | "data_missing" | "failed")
                                )
                            });
                            if all_terminal && !todos.is_empty() {
                                list_str.push_str("<SYSTEM_REMINDER>Todos are terminal. Call 'complete_workflow_with_summary' and mention any data gaps.</SYSTEM_REMINDER>\n");
                            }
                        }
                        raw_res = list_str;
                    }
                } else if tool_name == TOOL_SUBMIT_RESULT {
                    let structured = val
                        .get("structured_content")
                        .cloned()
                        .unwrap_or(Value::Null);
                    let explicit_result = structured
                        .get("result")
                        .and_then(|value| value.as_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| value_to_text(val));
                    raw_res = explicit_result;
                }

                // --- Custom Logic for File Tools (Formatting for UI Diff) ---
                if matches!(
                    tool_name,
                    TOOL_EDIT_FILE | TOOL_WRITE_FILE | TOOL_PLAN_EDIT_NOTE | TOOL_PLAN_WRITE_NOTE
                ) {
                    let mut preview_args = args.clone();
                    merge_tool_result_into_preview_args(
                        &mut preview_args,
                        val.get("structured_content"),
                        val.get("content").and_then(|value| value.as_str()),
                    );
                    attach_display_context(&mut preview_args, true, primary_root);
                    raw_res = serde_json::to_string(&preview_args).unwrap_or(raw_res);
                }

                let display_type = if matches!(
                    tool_name,
                    TOOL_EDIT_FILE | TOOL_WRITE_FILE | TOOL_PLAN_EDIT_NOTE | TOOL_PLAN_WRITE_NOTE
                ) {
                    "diff"
                } else {
                    "text"
                };

                if raw_res == "[]" || raw_res == "{}" || raw_res.is_empty() {
                    let empty_hint = match tool_name {
                        TOOL_WEB_SEARCH => "No results. Try narrower keywords; use Chinese for China-centric topics.",
                        TOOL_WEB_FETCH => "No page content. Try another source or treat this URL as unavailable.",
                        _ => "No data returned. Narrow the query or verify the target exists.",
                    };
                    ReinforcedResult {
                        content: format!("Tool '{}' executed successfully but returned no data. <SYSTEM_REMINDER>{}</SYSTEM_REMINDER>", tool_name, empty_hint),
                        llm_content: llm_content_override.clone(),
                        title,
                        summary: "No data returned".to_string(),
                        is_error: false,
                        error_type: None,
                        display_type: display_type.to_string(),
                        approval_status: None,
                        observation_kind: None,
                    }
                } else if tool_name == TOOL_WEB_FETCH
                    && raw_res.chars().count() > TOOL_OUTPUT_CHAR_LIMIT
                {
                    if let Some((file_path, file_size)) = persist_web_fetch_overflow(&raw_res) {
                        let preview = line_based_preview(&raw_res, TOOL_OUTPUT_CHAR_LIMIT);
                        ReinforcedResult {
                            content: format!(
                                "<truncated_content path=\"{}\" next_offset=\"{}\" file_size_bytes=\"{}\">\n{}\n</truncated_content>\n<SYSTEM_REMINDER>web_fetch content exceeded {} chars, so the full response was saved to '{}'. File size: {} bytes. Continue with read_file using offset={} to read the remaining content. If you need to find specific facts quickly, use grep on this file path before reading more. Treat the saved file as the source of truth for the remainder instead of retrying the same URL.</SYSTEM_REMINDER>",
                                file_path,
                                preview.next_offset,
                                file_size,
                                preview.content,
                                TOOL_OUTPUT_CHAR_LIMIT,
                                file_path,
                                file_size,
                                preview.next_offset,
                            ),
                            llm_content: llm_content_override.clone(),
                            title,
                            summary: format!("{} (Persisted overflow)", summary),
                            is_error: false,
                            error_type: None,
                            display_type: display_type.to_string(),
                            approval_status: None,
                            observation_kind: None,
                        }
                    } else {
                        let truncated = truncate_text_for_preview(&raw_res, TOOL_OUTPUT_CHAR_LIMIT);
                        ReinforcedResult {
                            content: format!(
                                "<truncated_content>\n{}\n</truncated_content>\n<SYSTEM_REMINDER>web_fetch content exceeded {} chars. Failed to persist the full response to a temp file, so only the leading preview is available in this observation. Narrow the target URL or fetch another source if more content is required.</SYSTEM_REMINDER>",
                                truncated,
                                TOOL_OUTPUT_CHAR_LIMIT,
                            ),
                            llm_content: llm_content_override.clone(),
                            title,
                            summary: format!("{} (Truncated)", summary),
                            is_error: false,
                            error_type: None,
                            display_type: display_type.to_string(),
                            approval_status: None,
                            observation_kind: None,
                        }
                    }
                } else if !matches!(
                    tool_name,
                    TOOL_READ_FILE
                        | TOOL_PLAN_READ_NOTE
                        | TOOL_EDIT_FILE
                        | TOOL_WRITE_FILE
                        | TOOL_PLAN_EDIT_NOTE
                        | TOOL_PLAN_WRITE_NOTE
                ) && raw_res.len() > 20000
                {
                    let truncated = match raw_res.char_indices().nth(20000) {
                        Some((idx, _)) => &raw_res[..idx],
                        None => &raw_res,
                    };
                    ReinforcedResult {
                        content: format!(
                            "<truncated_content>\n{}\n</truncated_content>\n<SYSTEM_REMINDER>Output truncated at 20000 chars. Narrow with grep/glob or smaller reads.</SYSTEM_REMINDER>",
                            truncated
                        ),
                        llm_content: llm_content_override.clone(),
                        title,
                        summary: format!("{} (Truncated)", summary),
                        is_error: false,
                        error_type: None,
                        display_type: display_type.to_string(),
                        approval_status: None,
                        observation_kind: None,
                    }
                } else {
                    ReinforcedResult {
                        content: raw_res,
                        llm_content: llm_content_override,
                        title,
                        summary,
                        is_error: false,
                        error_type: None,
                        display_type: display_type.to_string(),
                        approval_status: None,
                        observation_kind: None,
                    }
                }
            }
            Err(err) => {
                let err_msg = err.to_string();
                let title =
                    Self::generate_title(tool_name, &args, extra_context.as_ref(), primary_root);
                let error_type = match err {
                    ToolError::Security(_) => "Security",
                    ToolError::IoError(_) => "Io",
                    ToolError::InvalidParams(_) => "InvalidParams",
                    ToolError::NetworkError(_) => "NetworkError",
                    ToolError::AuthError(_) => "AuthError",
                    _ => "Other",
                };

                // Content only contains the raw error.
                // Recovery hints will be injected dynamically by LlmProcessor to avoid duplication.
                let content = format!("Error: {}", err_msg);

                ReinforcedResult {
                    content,
                    llm_content: None,
                    title,
                    summary: format!("Error: {}", err_msg),
                    is_error: true,
                    error_type: Some(error_type.to_string()),
                    display_type: "text".to_string(),
                    approval_status: None,
                    observation_kind: None,
                }
            }
        }
    }

    pub(crate) fn generate_title(
        tool_name: &str,
        args: &Value,
        extra_context: Option<&Value>,
        primary_root: Option<&std::path::Path>,
    ) -> String {
        // let truncate = |s: &str, len: usize| -> String {
        //     let chars: Vec<char> = s.chars().collect();
        //     if chars.len() <= len {
        //         s.to_string()
        //     } else {
        //         let truncated: String = chars.iter().take(len - 3).collect();
        //         format!("{}...", truncated)
        //     }
        // };

        let get_domain = |url: &str| -> String {
            if let Some(host) = url.split("://").nth(1).and_then(|s| s.split('/').next()) {
                host.to_string()
            } else {
                url.to_string()
            }
        };

        let get_relative_path = |path_str: &str| -> String {
            if path_str.is_empty() {
                return path_str.to_string();
            }

            let path = std::path::Path::new(path_str);

            // Try to make path relative to primary_root
            if let Some(root) = primary_root {
                if let Ok(relative) = path.strip_prefix(root) {
                    // Return relative path, use "." if it's the root itself
                    let rel_str = relative.to_string_lossy();
                    return if rel_str.is_empty() {
                        ".".to_string()
                    } else {
                        rel_str.to_string()
                    };
                }
            }

            // Return original path if we can't make it relative
            path_str.to_string()
        };

        match tool_name {
            TOOL_READ_FILE => {
                let path = args["file_path"]
                    .as_str()
                    .or(args["path"].as_str())
                    .unwrap_or("");
                let display_path = get_relative_path(path);
                let limit = args["limit"].as_i64();
                let offset = args["offset"].as_i64();
                let mut suffix = String::new();
                if let (Some(l), Some(o)) = (limit, offset) {
                    suffix = format!(" L{}-{}", o.saturating_add(1), o.saturating_add(l));
                } else if let Some(l) = limit {
                    suffix = format!(" L1-{}", l);
                } else if let Some(o) = offset {
                    suffix = format!(" from L{}", o.saturating_add(1));
                }
                format!("Read {}{}", display_path, suffix)
            }
            TOOL_WRITE_FILE => {
                let path = args["file_path"]
                    .as_str()
                    .or(args["path"].as_str())
                    .unwrap_or("");
                let display_path = get_relative_path(path);
                format!("Write {}", display_path)
            }
            TOOL_EDIT_FILE => {
                let path = args["file_path"]
                    .as_str()
                    .or(args["path"].as_str())
                    .unwrap_or("");
                let display_path = get_relative_path(path);
                format!("Edit {}", display_path)
            }
            TOOL_PLAN_READ_NOTE => {
                let note_name = args["note_name"].as_str().unwrap_or("");
                format!("Read plan note {}", note_name)
            }
            TOOL_PLAN_WRITE_NOTE => {
                let note_name = args["note_name"].as_str().unwrap_or("");
                format!("Write plan note {}", note_name)
            }
            TOOL_PLAN_EDIT_NOTE => {
                let note_name = args["note_name"].as_str().unwrap_or("");
                format!("Edit plan note {}", note_name)
            }
            TOOL_LIST_DIR => {
                let path = args["path"]
                    .as_str()
                    .or(args["dir"].as_str())
                    .unwrap_or(".");
                let display_path = get_relative_path(path);
                format!("List {}", display_path)
            }
            TOOL_GLOB => {
                let pattern = args["pattern"]
                    .as_str()
                    .or(args["glob"].as_str())
                    .unwrap_or("");
                format!("Glob {}", pattern)
            }
            TOOL_GREP => {
                let pattern = args["pattern"]
                    .as_str()
                    .or(args["query"].as_str())
                    .unwrap_or("");
                let path = args["path"].as_str().unwrap_or("");
                if !path.is_empty() {
                    let display_path = get_relative_path(path);
                    format!("Grep \"{}\" in {}", pattern, display_path)
                } else {
                    format!("Grep \"{}\"", pattern)
                }
            }
            TOOL_WEB_FETCH => {
                let url = args["url"].as_str().unwrap_or("");
                format!("Fetch {}", get_domain(url))
            }
            TOOL_WEB_SEARCH => {
                let query = args["query"].as_str().unwrap_or("");
                let num_results = args["num_results"].as_i64();
                if let Some(n) = num_results {
                    format!("Search \"{}\" Number:{}", query, n)
                } else {
                    format!("Search \"{}\"", query)
                }
            }
            TOOL_BASH => {
                let cmd = args["command"].as_str().unwrap_or("");
                format!("Run {}", cmd)
            }
            TOOL_TODO_CREATE => {
                if let Some(tasks) = args["tasks"].as_array() {
                    return format!(
                        "{} ({} items)",
                        t!("workflow.summary.todo_create"),
                        tasks.len()
                    );
                }
                t!("workflow.summary.todo_create").to_string()
            }
            TOOL_TODO_UPDATE => {
                let id_val = args["todo_id"]
                    .as_str()
                    .or(args["task_id"].as_str())
                    .or(args["id"].as_str());

                let mut subject = args["subject"]
                    .as_str()
                    .or(args["title"].as_str())
                    .map(|s| s.to_string());

                // Lookup title from current todo list if not provided in arguments
                if subject.is_none() {
                    if let (Some(id), Some(todos)) =
                        (id_val, extra_context.and_then(|v| v.as_array()))
                    {
                        subject = todos
                            .iter()
                            .find(|t| {
                                t["id"].as_str() == Some(id)
                                    || t["task_id"].as_str() == Some(id)
                                    || t["todo_id"].as_str() == Some(id)
                            })
                            .and_then(|t| t["subject"].as_str().or(t["title"].as_str()))
                            .map(|s| s.to_string());
                    }
                }

                let display_subject =
                    subject.unwrap_or_else(|| id_val.unwrap_or("Task").to_string());
                let status_raw = args["status"].as_str().unwrap_or("updated");
                let status = match status_raw {
                    "completed" | "done" => t!("workflow.summary.todo_status_done"),
                    "in_progress" => t!("workflow.summary.todo_status_in_progress"),
                    "pending" => t!("workflow.summary.todo_status_pending"),
                    "failed" => t!("workflow.summary.todo_status_failed"),
                    "data_missing" => t!("workflow.summary.todo_status_data_missing"),
                    _ => status_raw.into(),
                };
                t!(
                    "workflow.summary.todo_update",
                    subject = &display_subject,
                    status = status
                )
                .to_string()
            }
            TOOL_TODO_LIST => t!("workflow.summary.todo_list").to_string(),
            TOOL_TODO_GET => t!("workflow.summary.todo_get").to_string(),
            TOOL_SUBMIT_PLAN => "Submit Plan".to_string(),
            TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY => "Complete Workflow with Summary".to_string(),
            TOOL_SUBMIT_RESULT => "Submit Result".to_string(),
            crate::tools::TOOL_SUB_AGENT_OUTPUT => {
                let task_id = args["task_id"].as_str().unwrap_or("").trim();
                if task_id.is_empty() {
                    "Sub-agent Output".to_string()
                } else {
                    format!("Sub-agent Output {}", task_id)
                }
            }
            crate::tools::TOOL_SKILL => {
                let skill = args["skill"].as_str().unwrap_or("").trim();
                if skill.is_empty() {
                    "Skill".to_string()
                } else {
                    format!("Skill {}", skill)
                }
            }
            _ => {
                // Special handling for MCP tools (format: server__MCP__tool)
                if tool_name.contains(crate::tools::MCP_TOOL_NAME_SPLIT) {
                    let parts: Vec<&str> =
                        tool_name.split(crate::tools::MCP_TOOL_NAME_SPLIT).collect();
                    if parts.len() == 2 {
                        let server = parts[0];
                        let tool = parts[1].replace(['_', '-'], " ");
                        let tool_capitalized = tool
                            .split_whitespace()
                            .map(|w| {
                                let mut c = w.chars();
                                match c.next() {
                                    None => String::new(),
                                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                                }
                            })
                            .collect::<Vec<_>>()
                            .join(" ");
                        return format!("{} ({})", tool_capitalized, server);
                    }
                }

                let name = tool_name.replace('_', " ");
                name.split_whitespace()
                    .map(|w| {
                        let mut c = w.chars();
                        match c.next() {
                            None => String::new(),
                            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            }
        }
    }

    fn generate_summary(tool_name: &str, content: &str, _args: &Value) -> String {
        match tool_name {
            TOOL_SUBMIT_PLAN => t!("workflow.summary.submit_plan").to_string(),
            TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY => t!("workflow.task_finished").to_string(),
            TOOL_SUBMIT_RESULT => {
                if let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(content) {
                    obj.get("summary")
                        .and_then(|value| value.as_str())
                        .unwrap_or("Result submitted")
                        .to_string()
                } else {
                    "Result submitted".to_string()
                }
            }
            TOOL_READ_FILE => {
                let lines = content.lines().count();
                format!("Read {} lines", lines)
            }
            TOOL_GREP if content.trim() == "[No matches found]" => "No matches found".to_string(),
            TOOL_LIST_DIR | TOOL_GLOB => {
                if let Ok(Value::Array(arr)) = serde_json::from_str::<Value>(content) {
                    format!("Found {} entries", arr.len())
                } else {
                    "Success".to_string()
                }
            }
            TOOL_GREP => {
                let lines = content.lines().count();
                format!("Found {} matches", lines)
            }
            TOOL_WEB_SEARCH => {
                if let Ok(Value::Array(arr)) = serde_json::from_str::<Value>(content) {
                    format!("Found {} results", arr.len())
                } else {
                    "Search completed".to_string()
                }
            }
            TOOL_WEB_FETCH => {
                // Return success immediately, handled by reinforcement usually
                "Fetched content".to_string()
            }
            TOOL_EDIT_FILE => t!("workflow.summary.edit_file").to_string(),
            TOOL_PLAN_EDIT_NOTE => t!("workflow.summary.edit_file").to_string(),
            TOOL_PLAN_WRITE_NOTE => t!("workflow.summary.write_file").to_string(),
            TOOL_PLAN_READ_NOTE => {
                let lines = content.lines().count();
                format!("Read {} lines", lines)
            }
            TOOL_WRITE_FILE => t!("workflow.summary.write_file").to_string(),
            TOOL_BASH => {
                let last_line = content.lines().last().unwrap_or("Done");
                match last_line.char_indices().nth(30) {
                    Some(_) => {
                        let truncated: String = last_line.chars().take(27).collect();
                        format!("{}...", truncated)
                    }
                    None => last_line.to_string(),
                }
            }
            _ => {
                if tool_name.contains(crate::tools::MCP_TOOL_NAME_SPLIT) {
                    "Operation completed".to_string()
                } else {
                    "Executed successfully".to_string()
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reinforce_read_file_does_not_apply_generic_second_truncation() {
        let long_content = format!(
            "<truncated_content>\n{}\n</truncated_content>\n<SYSTEM_REMINDER>File size: 12345 bytes; total lines: 999.</SYSTEM_REMINDER>",
            "x".repeat(25_000)
        );
        let tool_call = json!({
            "function": {
                "name": TOOL_READ_FILE,
                "arguments": "{\"file_path\":\"/tmp/demo.txt\"}"
            }
        });

        let reinforced = ObservationReinforcer::reinforce_with_context(
            &tool_call,
            &Ok(json!({ "content": long_content })),
            None,
            None,
        );

        assert!(reinforced.content.contains("<truncated_content>"));
        assert!(reinforced.content.contains("total lines: 999"));
        assert!(!reinforced
            .content
            .contains("Output truncated to 20000 chars"));
    }

    #[test]
    fn reinforce_web_fetch_persists_overflow_and_guides_follow_up_reads() {
        let line = "abcdefghij".repeat(100);
        let raw_content = (0..25)
            .map(|_| line.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let tool_call = json!({
            "function": {
                "name": TOOL_WEB_FETCH,
                "arguments": "{\"url\":\"https://raw.githubusercontent.com/waditu-tushare/skills/refs/heads/master/tushare/references/%E6%95%B0%E6%8D%AE%E6%8E%A5%E5%8F%A3.md\"}"
            }
        });

        let reinforced = ObservationReinforcer::reinforce_with_context(
            &tool_call,
            &Ok(json!({ "content": raw_content.clone() })),
            None,
            None,
        );

        assert!(reinforced.content.contains("<truncated_content path=\""));
        assert!(reinforced.content.contains("file_size_bytes=\""));
        assert!(reinforced.content.contains("next_offset=\"19\""));
        assert!(reinforced
            .content
            .contains("Continue with read_file using offset=19"));
        assert!(reinforced.content.contains("File size:"));
        assert!(reinforced.summary.contains("Persisted overflow"));
    }

    #[test]
    fn reinforce_web_fetch_uses_zero_offset_for_single_oversized_line() {
        let raw_content = "x".repeat(25_000);
        let tool_call = json!({
            "function": {
                "name": TOOL_WEB_FETCH,
                "arguments": "{\"url\":\"https://raw.githubusercontent.com/waditu-tushare/skills/refs/heads/master/tushare/references/%E6%95%B0%E6%8D%AE%E6%8E%A5%E5%8F%A3.md\"}"
            }
        });

        let reinforced = ObservationReinforcer::reinforce_with_context(
            &tool_call,
            &Ok(json!({ "content": raw_content })),
            None,
            None,
        );

        assert!(reinforced.content.contains("next_offset=\"0\""));
        assert!(reinforced
            .content
            .contains("Continue with read_file using offset=0"));
    }

    #[test]
    fn reinforce_write_file_overwrite_preserves_old_content_for_diff() {
        let tool_call = json!({
            "function": {
                "name": TOOL_WRITE_FILE,
                "arguments": {
                    "file_path": "/tmp/demo.txt",
                    "content": "new content",
                    "overwrite": true
                }
            }
        });

        let reinforced = ObservationReinforcer::reinforce_with_context(
            &tool_call,
            &Ok(json!({
                "content": "File written successfully; existing file was backed up first.",
                "structured_content": {
                    "file_path": "/tmp/demo.txt",
                    "display_path": "demo.txt",
                    "bytes_written": 11,
                    "overwritten": true,
                    "old_string": "original content",
                    "content": "new content",
                    "backup_path": "/tmp/demo.txt.bak"
                }
            })),
            None,
            None,
        );

        let details: Value = serde_json::from_str(&reinforced.content).unwrap();
        assert_eq!(details["old_string"].as_str(), Some("original content"));
        assert_eq!(details["content"].as_str(), Some("new content"));
        assert_eq!(details["overwritten"].as_bool(), Some(true));
    }
}
