use crate::tools::{
    ToolError, TOOL_BASH, TOOL_EDIT_FILE, TOOL_FINISH_TASK, TOOL_GLOB, TOOL_GREP, TOOL_LIST_DIR,
    TOOL_READ_FILE, TOOL_SUBMIT_PLAN, TOOL_TODO_CREATE, TOOL_TODO_GET, TOOL_TODO_LIST,
    TOOL_TODO_UPDATE, TOOL_WEB_FETCH, TOOL_WEB_SEARCH, TOOL_WRITE_FILE,
};

use rust_i18n::t;
use serde_json::Value;

pub struct ObservationReinforcer;

pub struct ReinforcedResult {
    pub content: String,
    pub title: String,
    pub summary: String,
    pub is_error: bool,
    pub error_type: Option<String>,
    pub display_type: String,            // "default", "diff", "text"
    pub approval_status: Option<String>, // "pending", "approved", "rejected", None
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
                                "<SYSTEM_REMINDER>Next pending task: {}\n</SYSTEM_REMINDER>",
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
                                list_str.push_str("<SYSTEM_REMINDER>All tasks have reached a terminal state (completed/data_missing/failed). You should now call 'finish_task' with a comprehensive summary, noting any data gaps.</SYSTEM_REMINDER>\n");
                            }
                        }
                        raw_res = list_str;
                    }
                }

                // --- Custom Logic for File Tools (Formatting for UI Diff) ---
                if tool_name == TOOL_EDIT_FILE || tool_name == TOOL_WRITE_FILE {
                    let mut preview_args = args.clone();
                    let preview_limit = 100_000;

                    // Truncate large fields for UI history to prevent DB bloat and rendering lag
                    if let Some(content) = preview_args.get_mut("content") {
                        if let Some(s) = content.as_str() {
                            if s.chars().count() > preview_limit {
                                let truncated: String = s.chars().take(preview_limit).collect();
                                *content = serde_json::json!(format!(
                                    "{}\n... (truncated for preview)",
                                    truncated
                                ));
                            }
                        }
                    }
                    if let Some(old_s) = preview_args.get_mut("old_string") {
                        if let Some(s) = old_s.as_str() {
                            if s.chars().count() > preview_limit {
                                let truncated: String = s.chars().take(preview_limit).collect();
                                *old_s = serde_json::json!(format!(
                                    "{}\n... (truncated for preview)",
                                    truncated
                                ));
                            }
                        }
                    }
                    if let Some(new_s) = preview_args.get_mut("new_string") {
                        if let Some(s) = new_s.as_str() {
                            if s.chars().count() > preview_limit {
                                let truncated: String = s.chars().take(preview_limit).collect();
                                *new_s = serde_json::json!(format!(
                                    "{}\n... (truncated for preview)",
                                    truncated
                                ));
                            }
                        }
                    }

                    raw_res = serde_json::to_string(&preview_args).unwrap_or(raw_res);
                }

                let display_type = if tool_name == TOOL_EDIT_FILE || tool_name == TOOL_WRITE_FILE {
                    "diff"
                } else {
                    "text"
                };

                if raw_res == "[]" || raw_res == "{}" || raw_res.is_empty() {
                    let empty_hint = match tool_name {
                        "web_search" => "No search results found. Try different keywords, use a more specific query, or search in Chinese if the topic is China-related.",
                        "web_fetch" => "Web page returned no content. The URL may be inaccessible, blocked, or require authentication. Try a different source.",
                        _ => "If you expected data, try adjusting your search terms or checking if the target exists.",
                    };
                    ReinforcedResult {
                        content: format!("Tool '{}' executed successfully but returned no data. <SYSTEM_REMINDER>{}</SYSTEM_REMINDER>", tool_name, empty_hint),
                        title,
                        summary: "No data returned".to_string(),
                        is_error: false,
                        error_type: None,
                        display_type: display_type.to_string(),
                        approval_status: None,
                    }
                } else if raw_res.len() > 20000 {
                    let truncated = match raw_res.char_indices().nth(20000) {
                        Some((idx, _)) => &raw_res[..idx],
                        None => &raw_res,
                    };
                    ReinforcedResult {
                        content: format!(
                            "<truncated_content>\n{}\n</truncated_content>\n<SYSTEM_REMINDER>Output truncated to 20000 chars. Use specific search patterns or read smaller chunks.</SYSTEM_REMINDER>",
                            truncated
                        ),
                        title,
                        summary: format!("{} (Truncated)", summary),
                        is_error: false,
                        error_type: None,
                        display_type: display_type.to_string(),
                        approval_status: None,
                    }
                } else {
                    ReinforcedResult {
                        content: raw_res,
                        title,
                        summary,
                        is_error: false,
                        error_type: None,
                        display_type: display_type.to_string(),
                        approval_status: None,
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
                    title,
                    summary: format!("Error: {}", err_msg),
                    is_error: true,
                    error_type: Some(error_type.to_string()),
                    display_type: "text".to_string(),
                    approval_status: None,
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
                    suffix = format!(" L{}-{}", l, o);
                } else if let Some(l) = limit {
                    suffix = format!(" L{}", l);
                } else if let Some(o) = offset {
                    suffix = format!(" @{}", o);
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
                format!("Bash {}", cmd)
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
            TOOL_FINISH_TASK => "Finish Task".to_string(),
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
            TOOL_FINISH_TASK => t!("workflow.task_finished").to_string(),
            TOOL_READ_FILE => {
                let lines = content.lines().count();
                format!("Read {} lines", lines)
            }
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
            TOOL_EDIT_FILE => "Applied changes".to_string(),
            TOOL_WRITE_FILE => "File written".to_string(),
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
