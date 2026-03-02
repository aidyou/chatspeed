use crate::tools::{
    ToolError, TOOL_BASH, TOOL_EDIT_FILE, TOOL_GLOB, TOOL_GREP, TOOL_LIST_DIR, TOOL_READ_FILE,
    TOOL_TODO_CREATE, TOOL_TODO_GET, TOOL_TODO_LIST, TOOL_TODO_UPDATE, TOOL_WEB_FETCH,
    TOOL_WEB_SEARCH, TOOL_WRITE_FILE,
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
                let mut raw_res = if let Some(content) = val.get("content").and_then(|v| v.as_str())
                {
                    content.to_string()
                } else if let Some(structured) = val.get("structured_content") {
                    if structured.is_null() {
                        "".to_string()
                    } else {
                        serde_json::to_string_pretty(structured).unwrap_or_default()
                    }
                } else {
                    serde_json::to_string(val).unwrap_or_default()
                };

                // --- Custom Logic for TODO tools ---
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
                            t["status"].as_str() == Some("pending")
                                || t["status"].as_str() == Some("todo")
                        });

                        if let Some(next) = next_pending {
                            let subject = next["subject"].as_str().unwrap_or("Untitled");
                            list_str.push_str(&format!("Next pending task: {}\n", subject));
                        } else {
                            let all_done = todos.iter().all(|t| {
                                t["status"].as_str() == Some("completed")
                                    || t["status"].as_str() == Some("done")
                            });
                            if all_done && !todos.is_empty() {
                                list_str.push_str("All tasks are COMPLETED. You can now proceed to provide the final answer to the user.\n");
                            }
                        }
                        raw_res = list_str;
                    }
                }

                let title = Self::generate_title(tool_name, &args);
                let summary = Self::generate_summary(tool_name, &raw_res);
                let display_type = if tool_name == TOOL_EDIT_FILE {
                    "diff"
                } else {
                    "text"
                };

                if raw_res == "[]" || raw_res == "{}" || raw_res.is_empty() {
                    ReinforcedResult {
                        content: format!("Tool '{}' executed successfully but returned no data. <system-reminder>If you expected data, try adjusting your search terms or checking if the target exists.</system-reminder>", tool_name),
                        title,
                        summary: "No data returned".to_string(),
                        is_error: false,
                        error_type: None,
                        display_type: display_type.to_string(),
                    }
                } else if raw_res.len() > 50000 {
                    let truncated = match raw_res.char_indices().nth(50000) {
                        Some((idx, _)) => &raw_res[..idx],
                        None => &raw_res,
                    };
                    ReinforcedResult {
                        content: format!("[Result too long, truncated] {}\n<system-reminder>The output was truncated. Use more specific search patterns or read smaller chunks if needed.</system-reminder>", truncated),
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
                let content = format!("Error: {}", err_msg);
                let error_type = match err {
                    ToolError::Security(_) => "Security",
                    ToolError::IoError(_) => "Io",
                    ToolError::InvalidParams(_) => "InvalidParams",
                    ToolError::NetworkError(_) => "Network",
                    ToolError::AuthError(_) => "Auth",
                    _ => "Other",
                };

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
        let name = match tool_name {
            TOOL_READ_FILE => "Read",
            TOOL_WRITE_FILE => "Write",
            TOOL_EDIT_FILE => "Edit",
            TOOL_LIST_DIR => "List",
            TOOL_GREP => "Grep",
            TOOL_GLOB => "Glob",
            TOOL_WEB_SEARCH => "Search",
            TOOL_WEB_FETCH => "Fetch",
            TOOL_BASH => "Bash",
            TOOL_TODO_CREATE => "TodoCreate",
            TOOL_TODO_UPDATE => "TodoUpdate",
            TOOL_TODO_LIST => "TodoList",
            TOOL_TODO_GET => "TodoGet",
            _ => tool_name,
        };

        let mut parts = Vec::new();
        if let Some(obj) = args.as_object() {
            for (k, v) in obj {
                // Skip internal or too technical keys if necessary, or show all
                let val_str = match v {
                    Value::String(s) => match s.char_indices().nth(40) {
                        Some(_) => {
                            let truncated = match s.char_indices().nth(37) {
                                Some((idx, _)) => &s[..idx],
                                None => s,
                            };
                            format!("\"{}...\"", truncated)
                        }
                        None => format!("\"{}\"", s),
                    },
                    _ => v.to_string(),
                };
                parts.push(format!("{}: {}", k, val_str));
            }
        }

        if parts.is_empty() {
            name.to_string()
        } else {
            format!("{}({})", name, parts.join(", "))
        }
    }

    fn generate_summary(tool_name: &str, content: &str) -> String {
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
                format!("Fetched {} chars", content.len())
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
            TOOL_TODO_UPDATE => {
                if let Ok(val) = serde_json::from_str::<Value>(content) {
                    let subject = val["subject"].as_str().unwrap_or("");
                    let status_raw = val["status"].as_str().unwrap_or("todo");
                    let status = if status_raw == "done" {
                        t!("workflow.summary.todo_status_done")
                    } else {
                        t!("workflow.summary.todo_status_todo")
                    };
                    t!(
                        "workflow.summary.todo_update",
                        subject = subject,
                        status = status
                    )
                    .to_string()
                } else {
                    "Todo updated".to_string()
                }
            }
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
                        let box_char = if status == "done" { "[x]" } else { "[ ]" };
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
}
