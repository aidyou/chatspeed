//! Read-only presentation of structured tool metadata; execution state remains untouched.
use crate::types::Message;
use serde_json::Value;

/// Hide internal reminders, including a partially streamed closing boundary.
pub fn clean_text(text: &str) -> String {
    let mut result = text.to_owned();
    for tag in ["SYSTEM_REMINDER", "system_reminder"] {
        let open = format!("<{tag}>");
        let close = format!("</{tag}>");
        while let Some(start) = result.find(&open) {
            let end = result[start + open.len()..]
                .find(&close)
                .map(|offset| start + open.len() + offset + close.len())
                .unwrap_or(result.len());
            result.replace_range(start..end, "");
        }
    }
    result.trim().to_owned()
}

fn fence(text: &str, language: &str) -> String {
    let mut delimiter = "```".to_owned();
    while text.contains(&delimiter) {
        delimiter.push('`');
    }
    format!("{delimiter}{language}\n{text}\n{delimiter}")
}

fn decoded(value: &Value) -> Value {
    if let Some(text) = value.as_str() {
        serde_json::from_str(text).unwrap_or_else(|_| value.clone())
    } else {
        value.clone()
    }
}

/// Match the web consumer's structured name/arguments/details precedence.
pub fn tool_text(tool: &Value) -> String {
    let call = tool.get("tool_call").unwrap_or(tool);
    let function = call.get("function").unwrap_or(call);
    let name = tool_name(tool);
    let structured_details = decoded(&tool["details"]);
    let args = if structured_details.is_object()
        && ["old_string", "new_string", "content"]
            .iter()
            .any(|key| structured_details.get(key).is_some())
    {
        structured_details
    } else {
        decoded(
            tool.get("arguments")
                .or_else(|| function.get("arguments"))
                .unwrap_or(&Value::Null),
        )
    };
    let target = ["file_path", "path", "query", "url", "description"]
        .iter()
        .find_map(|key| args[*key].as_str())
        .unwrap_or("");
    let mut text = format!("### `{name}`");
    if !target.is_empty() {
        text.push_str(&format!("\n\n{}", fence(target, "text")));
    }
    match name {
        "bash" => {
            if let Some(command) = args["command"].as_str() {
                text.push_str(&format!("\n\n{}", fence(command, "bash")));
            }
        }
        "edit_file" | "write_file" => {
            if let Some(old) = args["old_string"].as_str() {
                let diff = old
                    .lines()
                    .map(|line| format!("-{line}\n"))
                    .collect::<String>()
                    + &args["new_string"]
                        .as_str()
                        .unwrap_or("")
                        .lines()
                        .map(|line| format!("+{line}\n"))
                        .collect::<String>();
                text.push_str(&format!("\n\n{}", fence(&diff, "diff")));
            } else if let Some(content) = args["content"].as_str() {
                text.push_str(&format!("\n\n{}", fence(content, "text")));
            }
        }
        "read_file" | "list_dir" => {}
        _ => {
            if !args.is_null() && args != serde_json::json!({}) {
                text.push_str(&format!(
                    "\n\n{}",
                    fence(
                        &serde_json::to_string_pretty(&args).unwrap_or_default(),
                        "json"
                    )
                ));
            }
        }
    }
    if let Some(result) = tool.get("result").or_else(|| tool.get("error")) {
        let body = result
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| serde_json::to_string_pretty(result).unwrap_or_default());
        text.push_str(&format!("\n\n{}", fence(&clean_text(&body), "text")));
    }
    text
}

/// Empty tool_calls arrays are ordinary messages, not tool groups.
pub fn is_tool_message(message: &Message) -> bool {
    message.role == "tool"
        || message.metadata["tool_calls"]
            .as_array()
            .is_some_and(|calls| !calls.is_empty())
        || message.metadata["tool_name"]
            .as_str()
            .is_some_and(|name| !name.is_empty())
}

/// Compact identity for native headers without formatting full arguments or results.
pub fn tool_label(tool: &Value) -> String {
    let tool = primary_tool(tool);
    let call = tool.get("tool_call").unwrap_or(tool);
    let function = call.get("function").unwrap_or(call);
    let name = tool_name(tool);
    let args = decoded(
        tool.get("arguments")
            .or_else(|| function.get("arguments"))
            .unwrap_or(&Value::Null),
    );
    let target = [
        "file_path",
        "path",
        "command",
        "query",
        "url",
        "description",
    ]
    .iter()
    .find_map(|field| args[*field].as_str())
    .unwrap_or("");
    let target: String = target
        .lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(100)
        .collect();
    if target.is_empty() {
        name.to_owned()
    } else {
        format!("{name} · {target}")
    }
}

/// Display only structured tool lifecycle fields, never infer state from output text.
pub fn tool_status(tool: &Value) -> &'static str {
    match tool["type"]
        .as_str()
        .or_else(|| tool["execution_status"].as_str())
    {
        Some("tool_started" | "running" | "approved_running") => "running",
        Some("tool_failed" | "failed" | "final_error") => "failed",
        Some("tool_completed" | "completed" | "final_success") => "completed",
        _ => "",
    }
}

/// Canonical call arrays occur on assistant rows, results on later tool rows.
pub fn primary_tool(tool: &Value) -> &Value {
    tool["tool_calls"]
        .as_array()
        .and_then(|calls| calls.first())
        .unwrap_or(tool)
}

/// Extract canonical edit details for the native diff renderer.
pub fn edit_lines(tool: &Value) -> Option<Vec<(bool, String)>> {
    let tool = primary_tool(tool);
    let call = tool.get("tool_call").unwrap_or(tool);
    let function = call.get("function").unwrap_or(call);
    let name = tool["tool_name"]
        .as_str()
        .or_else(|| function["name"].as_str())?;
    if !matches!(name, "edit_file" | "write_file") {
        return None;
    }
    let details = decoded(&tool["details"]);
    let args = if details.is_object() {
        details
    } else {
        decoded(
            tool.get("arguments")
                .or_else(|| function.get("arguments"))
                .unwrap_or(&Value::Null),
        )
    };
    let mut lines = Vec::new();
    if let Some(old) = args["old_string"].as_str() {
        lines.extend(old.lines().map(|line| (false, line.to_owned())));
    }
    let new = args["new_string"]
        .as_str()
        .or_else(|| args["content"].as_str())?;
    lines.extend(new.lines().map(|line| (true, line.to_owned())));
    Some(lines)
}

/// Metadata is not a second transcript; only recognized tool fields are rendered.
pub fn message_text(message: &Message) -> String {
    if message.metadata["ui_visibility"] == "hide" {
        return String::new();
    }
    let mut text = clean_text(&message.message);
    if let Some(reasoning) = &message.reasoning {
        let reasoning = clean_text(reasoning);
        if !reasoning.is_empty() {
            text.push_str(&format!(
                "\n\n{}\n\n{reasoning}",
                rust_i18n::t!("reasoning")
            ));
        }
    }
    if let Some(calls) = message.metadata["tool_calls"].as_array() {
        for call in calls {
            text.push_str(&format!("\n\n{}", tool_text(call)));
        }
    } else if message.metadata.get("tool_call").is_some()
        || message.metadata.get("tool_name").is_some()
    {
        let header = tool_text(&message.metadata);
        text = format!(
            "{header}\n\n{}",
            if message.role == "tool" && message.metadata["display_type"] != "markdown" {
                fence(&text, "text")
            } else {
                text
            }
        );
    } else if message.role == "tool" && !text.is_empty() {
        text = fence(&text, "text");
    }
    text
}

/// Stable expansion/cache identity survives live-to-durable reconciliation.
pub fn message_key(message: &Message) -> String {
    message.metadata["tool_call_id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .map(|id| format!("call-{id}"))
        .unwrap_or_else(|| message.id.to_string())
}

/// A compact result line is separate from the tool action/target header.
pub fn tool_result_summary(message: &Message) -> String {
    let value = &message.metadata;
    let text = value["summary"]
        .as_str()
        .or_else(|| value["result"].as_str())
        .or_else(|| value["error"].as_str())
        .unwrap_or(&message.message);
    clean_text(text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(160)
        .collect()
}

/// Canonical structured tool identity; never infer it from transcript text.
pub fn tool_name(tool: &Value) -> &str {
    let tool = primary_tool(tool);
    tool["tool_name"]
        .as_str()
        .or_else(|| tool["name"].as_str())
        .or_else(|| tool["tool_call"]["name"].as_str())
        .or_else(|| tool["tool_call"]["function"]["name"].as_str())
        .or_else(|| tool["function"]["name"].as_str())
        .unwrap_or("")
}

pub fn completion_summary(message: &Message) -> String {
    let tool = primary_tool(&message.metadata);
    let args = decoded(
        tool.get("arguments")
            .or_else(|| tool.pointer("/function/arguments"))
            .or_else(|| tool.pointer("/tool_call/arguments"))
            .or_else(|| tool.pointer("/tool_call/function/arguments"))
            .unwrap_or(&Value::Null),
    );
    clean_text(
        message.metadata["summary"]
            .as_str()
            .filter(|summary| !summary.trim().is_empty())
            .or_else(|| {
                args["summary"]
                    .as_str()
                    .filter(|summary| !summary.trim().is_empty())
            })
            .unwrap_or(&message.message),
    )
}

pub fn think_only(message: &Message) -> bool {
    message.role == "assistant"
        && clean_text(&message.message).is_empty()
        && message
            .reasoning
            .as_deref()
            .is_some_and(|text| !clean_text(text).is_empty())
}

pub fn collapsible(message: &Message) -> bool {
    message.role == "tool"
        && !tool_name(&message.metadata).is_empty()
        && !matches!(
            tool_name(&message.metadata),
            "ask_user" | "submit_plan" | "complete_workflow" | "sub_agent_run"
        )
        && !matches!(
            message.metadata["approval_status"].as_str(),
            Some("pending" | "pending_approval")
        )
}

/// Port the Vue activity boundaries while preserving every thought in chronological order.
pub fn project_messages(
    messages: impl IntoIterator<Item = Message>,
) -> Vec<crate::types::DisplayRow> {
    use crate::types::DisplayRow;
    let mut result = Vec::new();
    let mut activity: Vec<Message> = Vec::new();
    let flush = |activity: &mut Vec<Message>, result: &mut Vec<DisplayRow>| {
        if let Some(first) = activity.first() {
            let key = first.metadata["tool_call_id"]
                .as_str()
                .map(|id| format!("activity-{id}"))
                .unwrap_or_else(|| format!("activity-{}", first.id));
            result.push(DisplayRow::Activity {
                key,
                items: std::mem::take(activity),
            });
        }
    };
    let source: Vec<Message> = messages.into_iter().collect();
    let mut normalized = Vec::new();
    for mut message in source.iter().cloned() {
        let calls = message.metadata["tool_calls"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if message.role == "assistant" && !calls.is_empty() {
            if let Some(metadata) = message.metadata.as_object_mut() {
                metadata.remove("tool_calls");
            }
            if !clean_text(&message.message).is_empty() || message.reasoning.is_some() {
                normalized.push(message.clone());
            }
            for (index, call) in calls.into_iter().enumerate() {
                let id = call["id"]
                    .as_str()
                    .or_else(|| call["tool_call_id"].as_str())
                    .unwrap_or("");
                // Durable result rows own completed calls; pending calls are projected in place.
                if !id.is_empty()
                    && source
                        .iter()
                        .any(|row| row.role == "tool" && row.metadata["tool_call_id"] == id)
                {
                    continue;
                }
                let name = tool_name(&call).to_owned();
                normalized.push(Message { id: -message.id.saturating_mul(1000) - index as i64 - 100,
                    role: "tool".into(), metadata: serde_json::json!({"tool_name":name,"tool_call":call,"tool_call_id":id,"execution_status":"running"}), ..Message::default() });
            }
        } else {
            if message.role == "tool" {
                let id = message.metadata["tool_call_id"].as_str().unwrap_or("");
                if let Some(call) = source
                    .iter()
                    .flat_map(|row| row.metadata["tool_calls"].as_array().into_iter().flatten())
                    .find(|call| call["id"].as_str() == Some(id) && !id.is_empty())
                {
                    let name = tool_name(call).to_owned();
                    if message.metadata.get("tool_call").is_none() {
                        message.metadata["tool_call"] = call.clone();
                    }
                    if tool_name(&message.metadata).is_empty() {
                        message.metadata["tool_name"] = Value::String(name);
                    }
                }
            }
            normalized.push(message);
        }
    }
    let normalized: Vec<Message> = normalized
        .iter()
        .enumerate()
        .filter(|(index, message)| {
            !(think_only(message)
                && normalized.get(index + 1).is_some_and(|next| {
                    next.role == "assistant"
                        && next.metadata["message_kind"] == "completion_report"
                        && message.metadata["step_index"] == next.metadata["step_index"]
                }))
        })
        .map(|(_, message)| message.clone())
        .collect();
    let mut deduped: Vec<Message> = Vec::new();
    for message in normalized {
        if let Some(previous) = deduped.last_mut() {
            if tool_name(&previous.metadata) == "complete_workflow"
                && tool_name(&message.metadata) == "complete_workflow"
                && tool_status(&previous.metadata) == "failed"
                && tool_status(&message.metadata) == "failed"
                && clean_text(&previous.message) == clean_text(&message.message)
                && previous.metadata["summary"] == message.metadata["summary"]
            {
                let count = previous.metadata["finish_task_error_count"]
                    .as_u64()
                    .unwrap_or(1)
                    + 1;
                previous.metadata["finish_task_error_count"] = Value::from(count);
                continue;
            }
        }
        deduped.push(message);
    }
    // A completed thought without adjacent tools remains a standalone reasoning row.
    let normalized = deduped;
    let last = normalized.len().saturating_sub(1);
    for (index, message) in normalized.into_iter().enumerate() {
        if message.metadata["ui_visibility"] == "hide"
            || message.metadata["error_type"] == "SubAgentInterrupted"
        {
            continue;
        }
        if message.role == "assistant"
            && clean_text(&message.message).is_empty()
            && !think_only(&message)
            && !is_tool_message(&message)
        {
            continue;
        }
        if collapsible(&message) || think_only(&message) {
            activity.push(message);
        } else {
            if !activity.is_empty() && activity.iter().all(think_only) {
                result.extend(
                    std::mem::take(&mut activity)
                        .into_iter()
                        .map(DisplayRow::Message),
                );
            } else {
                flush(&mut activity, &mut result);
            }
            result.push(DisplayRow::Message(message));
        }
        if index == last {
            flush(&mut activity, &mut result);
        }
    }
    flush(&mut activity, &mut result);
    result
}

/// Ordered category counts match the formal Vue group's compact summary.
pub fn activity_summary(items: &[Message]) -> String {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for item in items.iter().filter(|item| !think_only(item)) {
        let key = match tool_name(&item.metadata) {
            "edit_file" | "write_file" => "group_edit",
            "todo_create" | "todo_update" | "todo_list" => "group_tasks",
            "bash" => "group_command",
            "read_file" | "list_dir" => "group_read",
            "grep" | "glob" | "web_search" => "group_search",
            "web_fetch" => "group_web",
            "skill" => "group_skill",
            "sub_agent_output" => "group_agent",
            name if name.starts_with("mcp") || item.metadata["tool_category"] == "mcp" => {
                "group_mcp"
            }
            _ => "group_tool",
        };
        if let Some((_, count)) = counts.iter_mut().find(|(label, _)| *label == key) {
            *count += 1;
        } else {
            counts.push((key, 1));
        }
    }
    counts
        .iter()
        .map(|(key, count)| format!("{} x{count}", rust_i18n::t!(*key)))
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn activity_projection_preserves_thought_order_counts_and_completion_boundary() {
        use crate::types::DisplayRow;
        let thought = Message {
            id: 1,
            role: "assistant".into(),
            reasoning: Some("keep all reasoning".into()),
            ..Message::default()
        };
        let tool = |id, name| Message {
            id,
            role: "tool".into(),
            metadata: json!({"tool_name":name}),
            ..Message::default()
        };
        let rows = project_messages(vec![
            thought,
            tool(2, "edit_file"),
            tool(3, "todo_update"),
            tool(4, "edit_file"),
            tool(5, "complete_workflow"),
        ]);
        assert_eq!(rows.len(), 2);
        let DisplayRow::Activity { items, .. } = &rows[0] else {
            panic!("expected activity");
        };
        assert_eq!(
            items.iter().map(|item| item.id).collect::<Vec<_>>(),
            vec![1, 2, 3, 4]
        );
        assert_eq!(items[0].reasoning.as_deref(), Some("keep all reasoning"));
        assert_eq!(
            activity_summary(items),
            format!(
                "{} x2 · {} x1",
                rust_i18n::t!("group_edit"),
                rust_i18n::t!("group_tasks")
            )
        );
        assert!(
            matches!(&rows[1], DisplayRow::Message(message) if tool_name(&message.metadata) == "complete_workflow")
        );
        for name in ["ask_user", "submit_plan", "sub_agent_run"] {
            assert!(!collapsible(&tool(6, name)));
        }
    }

    #[test]
    fn completion_call_is_joined_to_result_without_losing_summary_or_thought() {
        use crate::types::DisplayRow;
        let call = Message {
            id: 10,
            role: "assistant".into(),
            reasoning: Some("thought".into()),
            metadata: json!({"tool_calls":[{"id":"done","function":{"name":"complete_workflow","arguments":"{\"summary\":\"**report**\"}"}}]}),
            ..Message::default()
        };
        let result = Message {
            id: 11,
            role: "tool".into(),
            message: "ok".into(),
            metadata: json!({"tool_call_id":"done"}),
            ..Message::default()
        };
        let rows = project_messages(vec![call, result]);
        assert_eq!(rows.len(), 2);
        let DisplayRow::Message(done) = &rows[1] else {
            panic!("completion must be standalone");
        };
        assert_eq!(completion_summary(done), "**report**");
        assert_eq!(tool_name(&done.metadata), "complete_workflow");
    }

    #[test]
    fn projection_keeps_noncollapsible_and_pending_tools_outside_activity_groups() {
        use crate::types::DisplayRow;
        let rows = project_messages(vec![
            Message {
                id: 1,
                role: "tool".into(),
                metadata: json!({"tool_name":"read_file","approval_status":"pending"}),
                ..Message::default()
            },
            Message {
                id: 2,
                role: "assistant".into(),
                message: "answer".into(),
                reasoning: Some("reason".into()),
                ..Message::default()
            },
            Message {
                id: 3,
                role: "assistant".into(),
                metadata: json!({"ui_visibility":"hide"}),
                ..Message::default()
            },
        ]);
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| matches!(row, DisplayRow::Message(_))));
        let live = Message {
            id: -7,
            metadata: json!({"tool_call_id":"stable"}),
            ..Message::default()
        };
        let durable = Message {
            id: 99,
            ..live.clone()
        };
        assert_eq!(message_key(&live), message_key(&durable));
    }

    #[test]
    fn completion_report_pair_and_repeated_errors_follow_vue_projection() {
        use crate::types::DisplayRow;
        let thought = Message {
            id: 1,
            role: "assistant".into(),
            reasoning: Some("private thought".into()),
            metadata: json!({"step_index":2}),
            ..Message::default()
        };
        let report = Message {
            id: 2,
            role: "assistant".into(),
            message: "report".into(),
            metadata: json!({"step_index":2,"message_kind":"completion_report"}),
            ..Message::default()
        };
        let error = Message {
            id: 3,
            role: "tool".into(),
            message: "rejected".into(),
            metadata: json!({"tool_name":"complete_workflow","execution_status":"failed"}),
            ..Message::default()
        };
        let rows = project_messages(vec![
            thought,
            report,
            error.clone(),
            Message { id: 4, ..error },
        ]);
        assert_eq!(rows.len(), 2);
        assert!(matches!(&rows[0], DisplayRow::Message(message) if message.message == "report"));
        assert!(
            matches!(&rows[1], DisplayRow::Message(message) if message.metadata["finish_task_error_count"] == 2)
        );
    }

    #[test]
    fn reminders_are_removed_without_touching_surrounding_body() {
        assert_eq!(
            clean_text("before<SYSTEM_REMINDER>secret</SYSTEM_REMINDER>after"),
            "beforeafter"
        );
        assert_eq!(clean_text("body<SYSTEM_REMINDER>partial"), "body");
    }
    #[test]
    fn shell_arguments_are_commands_not_metadata_dumps() {
        let text = tool_text(
            &json!({"tool_name":"bash","arguments":{"command":"echo hi","timeout":100},"duration_ms":42}),
        );
        assert!(text.contains("```bash\necho hi"));
        assert!(!text.contains("duration_ms"));
        assert!(!text.contains("timeout"));
    }
    #[test]
    fn structured_diff_precedes_arguments_and_function_wire_is_supported() {
        let text = tool_text(
            &json!({"tool_name":"edit_file", "arguments":{"old_string":"obsolete"}, "details":{"file_path":"a.rs","old_string":"before","new_string":"after"}}),
        );
        assert!(text.contains("-before\n+after"));
        assert!(!text.contains("obsolete"));
        let call =
            tool_text(&json!({"function":{"name":"bash","arguments":"{\"command\":\"pwd\"}"}}));
        assert!(call.contains("```bash\npwd"));
    }

    #[test]
    fn empty_tool_arrays_do_not_hide_assistant_and_labels_stay_compact() {
        let message = Message {
            metadata: json!({"tool_calls": []}),
            message: "answer".into(),
            ..Message::default()
        };
        assert!(!is_tool_message(&message));
        assert_eq!(
            tool_label(
                &json!({"tool_calls":[{"function":{"name":"bash","arguments":"{\"command\":\"pwd\"}"}}]})
            ),
            "bash · pwd"
        );
        assert_eq!(tool_status(&json!({"type":"tool_failed"})), "failed");
        assert_eq!(tool_status(&json!({"result":"failed in text"})), "");
    }

    #[test]
    fn native_edit_lines_keep_additions_deletions_and_details_priority() {
        let lines = edit_lines(&json!({"tool_name":"edit_file","arguments":{"old_string":"stale","new_string":"stale"},"details":{"old_string":"a\nb","new_string":"c"}})).unwrap();
        assert_eq!(
            lines,
            vec![(false, "a".into()), (false, "b".into()), (true, "c".into())]
        );
        assert!(edit_lines(&json!({"tool_name":"bash","arguments":{"command":"pwd"}})).is_none());
    }

    #[test]
    fn opaque_metadata_is_not_rendered_and_fences_are_safe() {
        let message = Message {
            message: "hello".into(),
            metadata: json!({"internal":"private"}),
            ..Message::default()
        };
        assert_eq!(message_text(&message), "hello");
        assert!(fence("```", "text").starts_with("````text"));
        let hidden = Message {
            metadata: json!({"ui_visibility":"hide"}),
            ..message
        };
        assert!(message_text(&hidden).is_empty());
    }
}
