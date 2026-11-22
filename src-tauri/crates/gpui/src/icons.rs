//! Vue iconfont names and glyphs shared by the native observer UI.
use crate::types::Message;

/// Native copy of the web icon font.
///
/// GPUI's cosmic-text backend drops every face whose charmap has no Latin `m`
/// from the primary-font candidates, because the text editor measures layout
/// with that glyph. A pure private-use-area icon font would therefore be
/// silently replaced by a fallback face and every icon would render as an
/// unrelated glyph. `assets/chatspeed-icons.ttf` is the iconfont.cn export plus
/// one empty `m` glyph and one mirrored `double-arrow-up`; rebuild it with
/// `scripts/build_gpui_icon_font.py` when the export changes.
pub const FONT: &[u8] = include_bytes!("../assets/chatspeed-icons.ttf");
pub const FAMILY: &str = "chatspeed";

/// U+F101 in the native font: the iconfont export has no upward double chevron.
const DOUBLE_ARROW_UP: &str = "\u{f101}";

/// `cs-mute` lives outside the BMP and needs the font's format 12 cmap.
const MUTE: &str = "\u{13f7c}";

pub fn glyph(name: &str) -> &'static str {
    match name {
        "fullscreen-off" => "\u{e686}",
        "fullscreen" => "\u{e685}",
        "minimize" => "\u{e640}",
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
        "skill-mindmap-circle" => "\u{e664}",
        "task" => "\u{e732}",
        "caret-right" => "\u{e6b9}",
        "caret-down" => "\u{e6b8}",
        "check-circle" => "\u{e64f}",
        "check-x" => "\u{ed1a}",
        "check" => "\u{e620}",
        "error" => "\u{e7a5}",
        "uncheck" => "\u{e7c9}",
        "tool" => "\u{e64b}",
        "skill-terminal" => "\u{e616}",
        "skill-piechart" => "\u{e668}",
        "loading" => "\u{e891}",
        "compress" => "\u{e752}",
        "double-arrow-down" => "\u{e642}",
        "double-arrow-up" => DOUBLE_ARROW_UP,
        "copy" => "\u{e610}",
        "close" => "\u{e6a5}",
        "stop" => "\u{e6a2}",
        "clock" => "\u{e819}",
        "list" => "\u{e661}",
        "add" => "\u{e673}",
        "sidebar" => "\u{ef19}",
        "send" => "\u{e705}",
        "pin" => "\u{e755}",
        "new-chat" => "\u{e6d1}",
        "approval" => "\u{e696}",
        "sandbox" => "\u{e6a8}",
        "hammer" => "\u{e692}",
        "clear-context" => "\u{e70c}",
        "attachment" => "\u{e65b}",
        "skill-plan" => "\u{e68c}",
        "warning" => "\u{e6a1}",
        "at" => "\u{e81e}",
        "menu" => "\u{e678}",
        "skill-chat-square" => "\u{e68a}",
        "note" => "\u{e63c}",
        "switch" => "\u{12615}",
        "setting" => "\u{e621}",
        "brain" => "\u{e6b3}",
        "yolo" => "\u{e7ec}",
        "model" => "\u{e66e}",
        "agent" => "\u{e69b}",
        "proxy" => "\u{e654}",
        "extract" => "\u{e712}",
        "about" => "\u{e635}",
        "quit" => "\u{e7e5}",
        "enter-square" => "\u{e65a}",
        "shift-enter-square" => "\u{e655}",
        "undo" => "\u{e7f2}",
        "skill-plan3" => "\u{e711}",
        "plugin" => "\u{e92d}",
        "connected" => "\u{e64a}",
        "ext-folder" => "\u{e7d1}",
        "money" => "\u{e6d3}",
        "restart" => "\u{e652}",
        "delete" => "\u{e651}",
        "unmute" => "\u{e862}",
        "mute" => MUTE,
        "zip" => "\u{e683}",
        "refresh" => "\u{e71e}",
        "trash" => "\u{e61a}",
        "ext-folder-open" => "\u{e85e}",
        "ext-pic" => "\u{e663}",
        "ext-file" => "\u{e662}",
        "ext-docx" => "\u{e666}",
        "ext-pptx" => "\u{e790}",
        "ext-xlsx" => "\u{ea45}",
        "ext-pdf" => "\u{e66b}",
        "ext-csv" => "\u{e848}",
        "ext-html" => "\u{e679}",
        "ext-css" => "\u{e676}",
        "ext-js" => "\u{e688}",
        "ext-ts" => "\u{e66f}",
        "ext-jsx" => "\u{e66d}",
        "ext-tsx" => "\u{e670}",
        "ext-vue" => "\u{e69d}",
        "ext-c" => "\u{e69a}",
        "ext-cpp" => "\u{e66a}",
        "ext-java" => "\u{e66c}",
        "ext-rs" => "\u{e687}",
        "ext-go" => "\u{e62d}",
        "ext-swift" => "\u{e69e}",
        "ext-rb" => "\u{e691}",
        "ext-php" => "\u{e67e}",
        "ext-py" => "\u{e67a}",
        "ext-shell" => "\u{e6e5}",
        "ext-exe" => "\u{e681}",
        "ext-yaml" => "\u{e690}",
        "ext-json" => "\u{e67c}",
        "ext-xml" => "\u{e945}",
        "ext-toml" => "\u{e75d}",
        "ext-setting" => "\u{e87e}",
        "ext-git" => "\u{f1b4}",
        "ext-md" => "\u{e671}",
        "ext-txt" => "\u{e680}",
        "ext-log" => "\u{e67f}",
        _ => "\u{e64b}",
    }
}

/// File-type icon for a tree row, mirroring `getFileIcon` in `TreeNode.vue`.
///
/// `ext-zip` has no glyph in the web font, so archives use the standalone `zip`
/// icon instead of rendering an empty box.
pub fn file_icon(name: &str) -> &'static str {
    let lower = name.to_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or_default();
    match ext {
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "tgz" => "zip",
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "bmp" | "ico" | "tiff" | "raw" => {
            "ext-pic"
        }
        "mp3" | "wav" | "ogg" | "m4a" | "aac" | "flac" | "opus" | "mp4" | "webm" | "mov"
        | "m4v" | "ogv" | "mkv" => "ext-file",
        "doc" | "docx" => "ext-docx",
        "ppt" | "pptx" => "ext-pptx",
        "xls" | "xlsx" => "ext-xlsx",
        "pdf" => "ext-pdf",
        "csv" => "ext-csv",
        "html" | "htm" | "xhtml" => "ext-html",
        "css" | "scss" | "sass" | "less" | "styl" => "ext-css",
        "js" | "mjs" | "cjs" => "ext-js",
        "ts" | "mts" | "cts" => "ext-ts",
        "jsx" => "ext-jsx",
        "tsx" => "ext-tsx",
        "vue" => "ext-vue",
        "c" | "h" | "hpp" => "ext-c",
        "cpp" | "cc" | "cxx" | "hxx" => "ext-cpp",
        "java" | "kt" | "kts" | "jsp" | "scala" => "ext-java",
        "rs" => "ext-rs",
        "go" => "ext-go",
        "swift" => "ext-swift",
        "rb" => "ext-rb",
        "php" | "php4" | "php5" => "ext-php",
        "py" | "pyw" | "pyc" | "pyd" | "pyi" => "ext-py",
        "sh" | "bash" | "zsh" | "fish" | "ps1" | "psm1" | "bat" | "cmd" => "ext-shell",
        "exe" | "msi" | "dmg" | "app" | "bin" | "pkg" | "deb" | "rpm" | "apk" => "ext-exe",
        "yaml" | "yml" => "ext-yaml",
        "json" => "ext-json",
        "xml" => "ext-xml",
        "toml" => "ext-toml",
        "ini" | "conf" | "cfg" | "properties" | "env" => "ext-setting",
        "md" | "markdown" => "ext-md",
        "txt" => "ext-txt",
        "log" => "ext-log",
        _ if matches!(
            name,
            ".git" | ".gitignore" | ".gitattributes" | ".gitmodules"
        ) =>
        {
            "ext-git"
        }
        _ => "ext-file",
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
        for name in [
            "talk",
            "assistant",
            "bash",
            "caret-right",
            "fullscreen-off",
            "double-arrow-up",
        ] {
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
            "skill-mindmap-circle",
            "task",
            "caret-right",
            "caret-down",
            "check-circle",
            "check-x",
            "check",
            "error",
            "uncheck",
            "tool",
            "skill-terminal",
            "skill-piechart",
            "minimize",
            "fullscreen",
            "fullscreen-off",
            "loading",
            "compress",
            "double-arrow-down",
            "copy",
            "close",
            "stop",
            "clock",
            "list",
            "add",
            "sidebar",
            "send",
            "pin",
            "new-chat",
            "approval",
            "sandbox",
            "hammer",
            "clear-context",
            "attachment",
            "skill-plan",
            "warning",
            "at",
            "menu",
            "enter-square",
            "shift-enter-square",
            "undo",
            "skill-plan3",
            "plugin",
            "connected",
            "ext-folder",
            "money",
            "restart",
            "delete",
            "unmute",
            "mute",
            "zip",
            "refresh",
            "trash",
            "ext-folder-open",
            "ext-pic",
            "ext-file",
            "ext-docx",
            "ext-pptx",
            "ext-xlsx",
            "ext-pdf",
            "ext-csv",
            "ext-html",
            "ext-css",
            "ext-js",
            "ext-ts",
            "ext-jsx",
            "ext-tsx",
            "ext-vue",
            "ext-c",
            "ext-cpp",
            "ext-java",
            "ext-rs",
            "ext-go",
            "ext-swift",
            "ext-rb",
            "ext-php",
            "ext-py",
            "ext-shell",
            "ext-exe",
            "ext-yaml",
            "ext-json",
            "ext-xml",
            "ext-toml",
            "ext-setting",
            "ext-git",
            "ext-md",
            "ext-txt",
            "ext-log",
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

    #[test]
    fn file_icons_follow_tree_node_extensions() {
        assert_eq!(file_icon("main.rs"), "ext-rs");
        assert_eq!(file_icon("App.VUE"), "ext-vue");
        assert_eq!(file_icon("Cargo.toml"), "ext-toml");
        assert_eq!(file_icon("README.md"), "ext-md");
        assert_eq!(file_icon("bundle.tar.gz"), "zip");
        assert_eq!(file_icon(".gitignore"), "ext-git");
        assert_eq!(file_icon("LICENSE"), "ext-file");
    }
}
