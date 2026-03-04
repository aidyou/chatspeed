use crate::tools::{
    ToolError, TOOL_BASH, TOOL_EDIT_FILE, TOOL_FINISH_TASK, TOOL_GLOB, TOOL_GREP, TOOL_LIST_DIR,
    TOOL_READ_FILE, TOOL_TODO_CREATE, TOOL_TODO_GET, TOOL_TODO_LIST, TOOL_TODO_UPDATE,
    TOOL_WEB_FETCH, TOOL_WEB_SEARCH, TOOL_WRITE_FILE,
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
    pub display_type: String, // "default", "diff", "text"
}

impl ObservationReinforcer {
    /// Reinforces the tool result with heuristic hints to better guide the AI
    pub fn reinforce(tool_call: &Value, result: &Result<Value, ToolError>) -> ReinforcedResult {
        Self::reinforce_with_context(tool_call, result, None)
    }

    /// Reinforces with extra context (like full todo list)
    pub fn reinforce_with_context(
        tool_call: &Value,
        result: &Result<Value, ToolError>,
        extra_context: Option<Value>,
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

                let title = Self::generate_title(tool_name, &args);
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
                                "<system-reminder>Next pending task: {}\n</system-reminder>",
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
                                list_str.push_str("<system-reminder>All tasks have reached a terminal state (completed/data_missing/failed). You should now call 'finish_task' with a comprehensive summary, noting any data gaps.</system-reminder>\n");
                            }
                        }
                        raw_res = list_str;
                    }
                }

                // --- Structured formatting for web_search results (Formatting for AI) ---
                if tool_name == TOOL_WEB_SEARCH {
                    if let Ok(Value::Array(arr)) = serde_json::from_str::<Value>(&raw_res) {
                        if !arr.is_empty() {
                            let mut formatted = String::from("Search Results:\n");
                            for (i, item) in arr.iter().enumerate() {
                                let title = item["title"].as_str().unwrap_or("No title");
                                let snippet = item["snippet"].as_str().unwrap_or("");
                                let url = item["url"].as_str().unwrap_or("");
                                formatted.push_str(&format!(
                                    "{}. **{}**\n   {}\n   URL: {}\n\n",
                                    i + 1,
                                    title,
                                    snippet,
                                    url
                                ));
                            }
                            formatted.push_str("<system-reminder>Analyze these results carefully. Select the 1-3 most relevant and authoritative URLs, then use web_fetch to extract detailed data. Do NOT search again with similar keywords.</system-reminder>");
                            raw_res = formatted;
                        }
                    }
                }

                let display_type = if tool_name == TOOL_EDIT_FILE {
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
                        content: format!("Tool '{}' executed successfully but returned no data. <system-reminder>{}</system-reminder>", tool_name, empty_hint),
                        title,
                        summary: "No data returned".to_string(),
                        is_error: false,
                        error_type: None,
                        display_type: display_type.to_string(),
                    }
                } else if raw_res.len() > 20000 {
                    let truncated = match raw_res.char_indices().nth(20000) {
                        Some((idx, _)) => &raw_res[..idx],
                        None => &raw_res,
                    };
                    ReinforcedResult {
                        content: format!("[Result truncated to 20000 chars] {}\n<system-reminder>The output was truncated. Use more specific search patterns or read smaller chunks if needed.</system-reminder>", truncated),
                        title,
                        summary: format!("{} (Truncated)", summary),
                        is_error: false,
                        error_type: None,
                        display_type: display_type.to_string(),
                    }
                } else {
                    ReinforcedResult {
                        content: raw_res,
                        title,
                        summary,
                        is_error: false,
                        error_type: None,
                        display_type: display_type.to_string(),
                    }
                }
            }
            Err(err) => {
                let err_msg = err.to_string();
                let title = Self::generate_title(tool_name, &args);
                let error_type = match err {
                    ToolError::Security(_) => "Security",
                    ToolError::IoError(_) => "Io",
                    ToolError::InvalidParams(_) => "InvalidParams",
                    ToolError::NetworkError(_) => "Network",
                    ToolError::AuthError(_) => "Auth",
                    _ => "Other",
                };
                let recovery = Self::generate_recovery_hint(tool_name, error_type);
                let content = format!("Error: {}\n{}", err_msg, recovery);

                ReinforcedResult {
                    content,
                    title,
                    summary: format!("Error: {}", err_msg),
                    is_error: true,
                    error_type: Some(error_type.to_string()),
                    display_type: "text".to_string(),
                }
            }
        }
    }

    fn generate_title(tool_name: &str, args: &Value) -> String {
        let truncate = |s: &str, len: usize| -> String {
            let chars: Vec<char> = s.chars().collect();
            if chars.len() <= len {
                s.to_string()
            } else {
                let truncated: String = chars.iter().take(len - 3).collect();
                format!("{}...", truncated)
            }
        };

        let get_domain = |url: &str| -> String {
            if let Some(host) = url.split("://").nth(1).and_then(|s| s.split('/').next()) {
                host.to_string()
            } else {
                url.to_string()
            }
        };

        match tool_name {
            TOOL_READ_FILE => {
                let path = args["file_path"]
                    .as_str()
                    .or(args["path"].as_str())
                    .unwrap_or("");
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
                format!("Read {}{}", path, suffix)
            }
            TOOL_WRITE_FILE => {
                let path = args["file_path"]
                    .as_str()
                    .or(args["path"].as_str())
                    .unwrap_or("");
                format!("Write {}", path)
            }
            TOOL_EDIT_FILE => {
                let path = args["file_path"]
                    .as_str()
                    .or(args["path"].as_str())
                    .unwrap_or("");
                format!("Edit {}", path)
            }
            TOOL_LIST_DIR => {
                let path = args["path"]
                    .as_str()
                    .or(args["dir"].as_str())
                    .unwrap_or(".");
                format!("List {}", path)
            }
            TOOL_GLOB => {
                let pattern = args["pattern"]
                    .as_str()
                    .or(args["glob"].as_str())
                    .unwrap_or("");
                format!("Glob {}", truncate(pattern, 30))
            }
            TOOL_GREP => {
                let pattern = args["pattern"]
                    .as_str()
                    .or(args["query"].as_str())
                    .unwrap_or("");
                let path = args["path"].as_str().unwrap_or("");
                if !path.is_empty() {
                    format!(
                        "Grep \"{}\" in {}",
                        truncate(pattern, 15),
                        truncate(path, 15)
                    )
                } else {
                    format!("Grep \"{}\"", truncate(pattern, 25))
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
                    format!("Search \"{}\" Number:{}", truncate(query, 30), n)
                } else {
                    format!("Search \"{}\"", truncate(query, 30))
                }
            }
            TOOL_BASH => {
                let cmd = args["command"].as_str().unwrap_or("");
                format!("Bash {}", truncate(cmd, 30))
            }
            TOOL_TODO_CREATE => {
                if let Some(tasks) = args["tasks"].as_array() {
                    return format!(
                        "{} ({} items)",
                        t!("workflow.todo.createBatch"),
                        tasks.len()
                    );
                }
                let subject = args["subject"]
                    .as_str()
                    .or(args["title"].as_str())
                    .unwrap_or("Untitled");
                format!("{}: {}", t!("workflow.todo.create"), truncate(subject, 25))
            }
            TOOL_TODO_UPDATE => {
                let subject = args["subject"]
                    .as_str()
                    .or(args["title"].as_str())
                    .unwrap_or("Untitled");
                let status_raw = args["status"].as_str().unwrap_or("updated");
                let status = match status_raw {
                    "completed" | "done" => t!("workflow.todo.statusCompleted"),
                    "in_progress" => t!("workflow.todo.statusInProgress"),
                    "pending" => t!("workflow.todo.statusPending"),
                    _ => status_raw.into(),
                };
                format!("Update {} to {}", truncate(subject, 20), status)
            }
            TOOL_TODO_LIST => t!("workflow.todo.list").to_string(),
            TOOL_TODO_GET => t!("workflow.todo.view").to_string(),
            TOOL_FINISH_TASK => t!("workflow.finishTask").to_string(),
            _ => {
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

    fn generate_summary(tool_name: &str, content: &str, args: &Value) -> String {
        match tool_name {
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
                let url = args["url"].as_str().unwrap_or("");
                let domain =
                    if let Some(host) = url.split("://").nth(1).and_then(|s| s.split('/').next()) {
                        host
                    } else {
                        url
                    };
                format!("Fetched {} chars from {}", content.len(), domain)
            }
            TOOL_EDIT_FILE => "Applied changes".to_string(),
            TOOL_WRITE_FILE => "File written".to_string(),
            TOOL_BASH => {
                let last_line = content.lines().last().unwrap_or("Done");
                match last_line.char_indices().nth(30) {
                    Some(_) => {
                        let truncated = match last_line.char_indices().nth(27) {
                            Some((idx, _)) => &last_line[..idx],
                            None => last_line,
                        };
                        format!("{}...", truncated)
                    }
                    None => last_line.to_string(),
                }
            }
            TOOL_TODO_CREATE => {
                if let Ok(val) = serde_json::from_str::<Value>(content) {
                    let subject = val["subject"].as_str().unwrap_or("");
                    t!("workflow.summary.todo_create", subject = subject).to_string()
                } else {
                    t!("workflow.summary.todo_create", subject = "").to_string()
                }
            }
            TOOL_TODO_UPDATE => "Todo updated".to_string(),
            TOOL_TODO_LIST | TOOL_TODO_GET => {
                if let Ok(val) = serde_json::from_str::<Value>(content) {
                    let items = if val.is_array() {
                        val.as_array().unwrap().clone()
                    } else {
                        vec![val]
                    };

                    let mut summary = String::new();
                    for item in items {
                        let subject = item["subject"].as_str().unwrap_or("Unknown");
                        let status = item["status"].as_str().unwrap_or("todo");
                        let box_char = if status == "done" || status == "completed" {
                            "✓"
                        } else {
                            "☐"
                        };
                        if !summary.is_empty() {
                            summary.push('\n');
                        }
                        summary.push_str(&format!("{} {}", box_char, subject));
                    }
                    if summary.is_empty() {
                        t!("workflow.summary.todo_list").to_string()
                    } else {
                        summary
                    }
                } else {
                    t!("workflow.summary.todo_list").to_string()
                }
            }
            _ => "Executed successfully".to_string(),
        }
    }

    /// Generates actionable recovery hints based on the tool and error type.
    fn generate_recovery_hint(tool_name: &str, error_type: &str) -> String {
        match (tool_name, error_type) {
            ("web_search", "Network") => {
                "<system-reminder>Search failed due to network error. Try once more with the same query. If it fails again, try a completely different search engine keyword or mark the sub-task as data_missing.</system-reminder>".to_string()
            }
            ("web_fetch", "Network") | ("web_fetch", _) => {
                "<system-reminder>Failed to fetch this URL. Do NOT retry the same URL. Instead: (1) try an alternative URL from your search results, or (2) if no alternatives exist, mark the data as unavailable and move to the next task.</system-reminder>".to_string()
            }
            ("web_search", _) => {
                "<system-reminder>Search failed. Try rephrasing your query with different keywords. If the topic is China-related, try searching in Chinese.</system-reminder>".to_string()
            }
            (_, "Security") => {
                "<system-reminder>Path is outside your authorized workspace. Use list_dir to find valid paths or ask the user to grant access.</system-reminder>".to_string()
            }
            (_, "InvalidParams") => {
                "<system-reminder>Check the tool's input schema. Ensure all required fields are provided with correct types.</system-reminder>".to_string()
            }
            (_, "Io") => {
                "<system-reminder>File I/O error. Verify the path exists using list_dir before retrying.</system-reminder>".to_string()
            }
            _ => String::new(),
        }
    }
}
