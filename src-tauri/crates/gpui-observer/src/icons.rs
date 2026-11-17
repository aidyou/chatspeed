//! Vue iconfont names and glyphs shared by the native observer UI.
use crate::types::Message;

/// Native copy of the web icon font.
///
/// GPUI's cosmic-text backend drops every face whose charmap has no Latin `m`
/// from the primary-font candidates, because the text editor measures layout
/// with that glyph. A pure private-use-area icon font would therefore be
/// silently replaced by a fallback face and every icon would render as an
/// unrelated glyph. `assets/chatspeed-icons.ttf` is the iconfont.cn export plus
/// one empty `m` glyph; regenerate it with `fontTools` when the export changes.
pub const FONT: &[u8] = include_bytes!("../assets/chatspeed-icons.ttf");
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
        "check-circle" => "\u{e64f}",
        "check-x" => "\u{ed1a}",
        "check" => "\u{e620}",
        "uncheck" => "\u{e7c9}",
        "loading" => "\u{e891}",
        "compress" => "\u{e752}",
        "double-arrow-down" => "\u{e642}",
        "copy" => "\u{e610}",
        "close" => "\u{e6a5}",
        "stop" => "\u{e6a2}",
        "clock" => "\u{e819}",
        "list" => "\u{e661}",
        "add" => "\u{e673}",
        _ => "\u{e64b}",
    }
}

const DIRECT_ICON_NAMES: &[&str] = &[
    "list_dir",
    "edit_file",
    "read_file",
    "write_file",
    "submit_plan",
    "ask_user",
    "glob",
    "grep",
    "bash",
    "skill",
];

/// Mirror `resolveWorkflowToolIcon`, including MCP precedence over direct names.
pub fn tool_icon_for(name: &str, category: Option<&str>) -> &'static str {
    let normalized = name.trim().to_lowercase();
    if normalized.is_empty() {
        return "tool";
    }
    if is_mcp(&normalized, category) {
        return "mcp";
    }
    if let Some(direct) = DIRECT_ICON_NAMES
        .iter()
        .find(|candidate| **candidate == normalized)
    {
        return direct;
    }
    match normalized.as_str() {
        "todo_create" | "todo_list" | "todo_update" => "todo",
        "sub_agent_run" | "sub_agent_output" => "task",
        _ => "tool",
    }
}

fn is_mcp(name: &str, category: Option<&str>) -> bool {
    if category.is_some_and(|value| value.eq_ignore_ascii_case("mcp")) {
        return true;
    }
    name == "web_fetch"
        || name == "web_search"
        || name == "mcp_tool_expand"
        || name == "mcp_tool_execute"
        || name == "mcp_tool_load"
        || name.contains("__mcp__")
}

/// Group icon used by the compact activity summary.
pub fn activity_icon(items: &[Message]) -> &'static str {
    if items.iter().any(|item| {
        matches!(
            crate::presentation::tool_name(&item.metadata),
            "edit_file" | "write_file"
        )
    }) {
        return "edit";
    }
    match items
        .iter()
        .find(|item| item.role == "tool")
        .map(|item| tool_icon_for(crate::presentation::tool_name(&item.metadata), None))
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

    /// Resolves a BMP character through the font's format 4 Unicode cmap.
    fn glyph_id_for_char(font: &[u8], ch: char) -> Option<u16> {
        let u16_at = |offset: usize| u16::from_be_bytes([font[offset], font[offset + 1]]);
        let u32_at = |offset: usize| {
            u32::from_be_bytes([
                font[offset],
                font[offset + 1],
                font[offset + 2],
                font[offset + 3],
            ])
        };
        let tables = u16_at(4) as usize;
        let cmap = (0..tables)
            .map(|index| 12 + index * 16)
            .find(|offset| &font[*offset..*offset + 4] == b"cmap")
            .expect("cmap table");
        let cmap = u32_at(cmap + 8) as usize;
        for index in 0..u16_at(cmap + 2) as usize {
            let record = cmap + 4 + index * 8;
            let offset = cmap + u32_at(record + 4) as usize;
            let segments = u16_at(offset + 6) / 2;
            if u16_at(offset) != 4 || segments == 0 {
                continue;
            }
            let ends = offset + 14;
            let starts = ends + segments as usize * 2 + 2;
            let deltas = starts + segments as usize * 2;
            let ranges = deltas + segments as usize * 2;
            for segment in 0..segments as usize {
                let start = u16_at(starts + segment * 2);
                if !(start..=u16_at(ends + segment * 2)).contains(&(ch as u16)) {
                    continue;
                }
                let delta = u16_at(deltas + segment * 2);
                let range = u16_at(ranges + segment * 2);
                let glyph = if range == 0 {
                    (ch as u32).wrapping_add(delta as u32)
                } else {
                    let address =
                        ranges + segment * 2 + range as usize + (ch as u16 - start) as usize * 2;
                    u16_at(address) as u32 + delta as u32
                };
                let glyph = (glyph & 0xFFFF) as u16;
                return (glyph != 0).then_some(glyph);
            }
        }
        None
    }

    #[test]
    fn native_face_can_be_a_primary_font() {
        // GPUI drops faces without an `m` mapping from the primary-font
        // candidates, which silently renders every icon with a fallback glyph.
        assert!(
            glyph_id_for_char(FONT, 'm').is_some(),
            "native icon font lost its Latin 'm' glyph"
        );
        for name in ["talk", "assistant", "bash", "caret-right"] {
            let ch = glyph(name).chars().next().unwrap();
            assert!(glyph_id_for_char(FONT, ch).is_some(), "missing {name}");
        }
    }

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
            "check-circle",
            "check-x",
            "check",
            "uncheck",
            "loading",
            "compress",
            "double-arrow-down",
            "copy",
            "close",
            "stop",
            "clock",
            "list",
            "add",
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
            assert_eq!(tool_icon_for(name, None), "mcp");
        }
        assert_eq!(tool_icon_for(" SUB_AGENT_RUN ", None), "task");
        assert_eq!(tool_icon_for("todo_update", None), "todo");
        assert_eq!(tool_icon_for("read_file", None), "read_file");
        assert_eq!(tool_icon_for("unknown", None), "tool");
        assert_eq!(tool_icon_for("custom", Some("mcp")), "mcp");
        let item = |name| Message {
            metadata: serde_json::json!({"tool_name": name}),
            role: "tool".into(),
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
