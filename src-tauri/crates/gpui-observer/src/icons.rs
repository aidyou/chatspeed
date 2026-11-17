//! Vue iconfont names and glyphs shared by the native observer UI.
use crate::{presentation, types::Message};

pub const FONT: &[u8] = include_bytes!("../../../../src/components/icon/iconfont.ttf");
pub const FAMILY: &str = "chatspeed";

pub fn glyph(name: &str) -> &'static str {
    match name {
        "talk" => "\u{e649}",
        "assistant" => "\u{e6d0}",
        "reasoning" => "\u{e64c}",
        "edit" => "\u{e622}",
        "bash" => "\u{e69f}",
        "todo" => "\u{e7d4}",
        "search" => "\u{e633}",
        "skill-relation-chart" => "\u{e78d}",
        "mcp" => "\u{e650}",
        "read_file" => "\u{e6a0}",
        "write_file" => "\u{e694}",
        "edit_file" => "\u{e6a3}",
        "list_dir" => "\u{e67b}",
        "grep" => "\u{e6a6}",
        "glob" => "\u{e695}",
        "submit_plan" => "\u{e684}",
        "ask_user" => "\u{e689}",
        "skill" => "\u{e72a}",
        "task" => "\u{e732}",
        "caret-right" => "\u{e6b9}",
        "caret-down" => "\u{e6b8}",
        _ => "\u{e64b}",
    }
}

/// Mirror resolveWorkflowToolIcon, including MCP precedence over direct names.
pub fn tool_icon(name: &str) -> &'static str {
    let normalized = name.trim().to_lowercase();
    match normalized.as_str() {
        "web_fetch" | "web_search" | "mcp_tool_expand" | "mcp_tool_execute" | "mcp_tool_load" => {
            "mcp"
        }
        name if name.contains("__mcp__") => "mcp",
        "bash" => "bash",
        "read_file" => "read_file",
        "write_file" => "write_file",
        "edit_file" => "edit_file",
        "list_dir" => "list_dir",
        "glob" => "glob",
        "grep" => "grep",
        "submit_plan" => "submit_plan",
        "ask_user" => "ask_user",
        "skill" => "skill",
        "todo_create" | "todo_list" | "todo_update" => "todo",
        "sub_agent_run" | "sub_agent_output" => "task",
        _ => "tool",
    }
}

pub fn activity_icon(items: &[Message]) -> &'static str {
    if items.iter().any(|item| {
        matches!(
            presentation::tool_name(&item.metadata),
            "edit_file" | "write_file"
        )
    }) {
        return "edit";
    }
    match items
        .iter()
        .find(|item| !presentation::think_only(item))
        .map(|item| tool_icon(presentation::tool_name(&item.metadata)))
    {
        Some("bash") => "bash",
        Some("todo") => "todo",
        Some("read_file" | "list_dir" | "grep" | "glob") => "search",
        Some("task") => "skill-relation-chart",
        Some("mcp") => "mcp",
        None => "reasoning",
        _ => "tool",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyphs_match_vue_css() {
        let css = include_str!("../../../../src/components/icon/chatspeed.css");
        for name in [
            "talk",
            "assistant",
            "reasoning",
            "edit",
            "bash",
            "todo",
            "search",
            "skill-relation-chart",
            "mcp",
            "read_file",
            "write_file",
            "edit_file",
            "list_dir",
            "grep",
            "glob",
            "submit_plan",
            "ask_user",
            "skill",
            "task",
            "caret-right",
            "caret-down",
            "tool",
        ] {
            let rule = css
                .split(&format!(".cs-{name}:before {{"))
                .nth(1)
                .unwrap()
                .split('}')
                .next()
                .unwrap();
            let codepoint = glyph(name).chars().next().unwrap() as u32;
            assert!(
                rule.contains(&format!("\\{codepoint:x}\"")),
                "Vue glyph changed for {name}"
            );
        }
        assert_eq!(&FONT[..4], &[0, 1, 0, 0]);
    }

    #[test]
    fn tool_families_follow_vue_precedence_and_fallback() {
        for name in [
            "web_fetch",
            "web_search",
            "mcp_tool_execute",
            "server__mcp__tool",
        ] {
            assert_eq!(tool_icon(name), "mcp");
        }
        assert_eq!(tool_icon(" SUB_AGENT_RUN "), "task");
        assert_eq!(tool_icon("todo_update"), "todo");
        assert_eq!(tool_icon("read_file"), "read_file");
        assert_eq!(tool_icon("unknown"), "tool");
        let item = |name| Message {
            metadata: serde_json::json!({"tool_name": name}),
            ..Message::default()
        };
        assert_eq!(activity_icon(&[item("bash"), item("edit_file")]), "edit");
        assert_eq!(
            activity_icon(&[item("sub_agent_output")]),
            "skill-relation-chart"
        );
        assert_eq!(activity_icon(&[item("read_file")]), "search");
    }
}
