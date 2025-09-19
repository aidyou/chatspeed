use serde_json::json;
use std::sync::RwLockWriteGuard;

use crate::ccproxy::{
    adapter::unified::{SseStatus, UnifiedStreamChunk},
    get_tool_id,
    helper::tool_use_xml::ToolUse,
    types::{
        TODO_TAG_END, TODO_TAG_START, TOOL_COMPAT_MODE_PROMPT, TOOL_PARSE_ERROR_REMINDER,
        TOOL_TAG_END, TOOL_TAG_START,
    },
};

pub fn update_message_block(status: &mut RwLockWriteGuard<'_, SseStatus>, block: String) {
    if !status.current_content_block.is_empty() && status.current_content_block != block {
        status.message_index += 1;
    }
    status.current_content_block = block;
}

pub fn generate_tool_prompt(tools: &Vec<crate::ccproxy::adapter::unified::UnifiedTool>) -> String {
    let tools_xml = crate::ccproxy::helper::tool_use_xml::generate_tools_xml(tools);

    TOOL_COMPAT_MODE_PROMPT.replace("{TOOLS_LIST}", &tools_xml)
}

/// Process tool calls found in the buffer
pub fn process_tool_calls_in_buffer(
    status: &mut std::sync::RwLockWriteGuard<SseStatus>,
    unified_chunks: &mut Vec<UnifiedStreamChunk>,
) {
    loop {
        if status.in_tool_call_block {
            // We are inside a tool call, looking for the end tag
            if let Some(end_pos) = status.tool_compat_buffer.find(TOOL_TAG_END) {
                let end_of_block = end_pos + TOOL_TAG_END.len();
                let tool_xml = &status.tool_compat_buffer[..end_of_block].to_string();

                parse_and_emit_tool_call(status, tool_xml, unified_chunks);

                status.tool_compat_buffer = status.tool_compat_buffer[end_of_block..].to_string();
                status.in_tool_call_block = false; // STATE CHANGE
            } else {
                // Incomplete block, wait for more data
                break;
            }
        } else {
            // We are not in a tool call, looking for the next start tag
            let next_todo_pos = status.tool_compat_buffer.find(TODO_TAG_START);
            let next_tool_pos = status.tool_compat_buffer.find(TOOL_TAG_START);

            let first_tag_info = match (next_todo_pos, next_tool_pos) {
                (Some(p1), Some(p2)) => {
                    if p1 < p2 {
                        Some((p1, "todo"))
                    } else {
                        Some((p2, "tool_use"))
                    }
                }
                (Some(p1), None) => Some((p1, "todo")),
                (None, Some(p2)) => Some((p2, "tool_use")),
                (None, None) => None,
            };

            if let Some((start_pos, tag_type)) = first_tag_info {
                if start_pos > 0 {
                    let text_before = status.tool_compat_buffer[..start_pos].to_string();
                    unified_chunks.push(UnifiedStreamChunk::Text { delta: text_before });
                }
                status.tool_compat_buffer = status.tool_compat_buffer[start_pos..].to_string();

                if tag_type == "todo" {
                    if let Some(end_pos) = status.tool_compat_buffer.find(TODO_TAG_END) {
                        let end_of_block = end_pos + TODO_TAG_END.len();
                        status.tool_compat_buffer =
                            status.tool_compat_buffer[end_of_block..].to_string();
                    } else {
                        break; // Incomplete block
                    }
                } else {
                    // "tool_use"
                    status.in_tool_call_block = true; // STATE CHANGE
                                                      // The loop will now re-evaluate in the `in_tool_call_block = true` state
                }
            } else {
                // No more tags found
                break;
            }
        }
    }
}

/// At the end of the stream, attempts to auto-complete an incomplete tool tag.
pub fn auto_complete_and_process_tool_tag(
    status: &mut std::sync::RwLockWriteGuard<SseStatus>,
    unified_chunks: &mut Vec<UnifiedStreamChunk>,
) {
    // This function is called at the end of the stream.
    // First, ensure all fragments are in the main buffer.
    if !status.tool_compat_fragment_buffer.is_empty() {
        let fragment = status.tool_compat_fragment_buffer.clone();
        status.tool_compat_buffer.push_str(&fragment);
        status.tool_compat_fragment_buffer.clear();
        status.tool_compat_fragment_count = 0;
    }

    // If we are not in a tool call block, or the buffer is empty, there's nothing to complete.
    if !status.in_tool_call_block || status.tool_compat_buffer.is_empty() {
        return;
    }

    let end_tag = TOOL_TAG_END; // e.g. "</cs:tool_use>"
    let mut partial_match_len = 0;

    // Iterate from the full tag length down to the minimum required ("</").
    for len in (2..=end_tag.len()).rev() {
        let partial_tag = &end_tag[..len];
        if status.tool_compat_buffer.ends_with(partial_tag) {
            partial_match_len = len;
            break;
        }
    }

    if partial_match_len > 0 {
        log::warn!(
            "Incomplete tool tag detected at end of stream. Buffer ends with '{}'. Attempting auto-completion.",
            &end_tag[..partial_match_len]
        );

        // The buffer already ends with a part of the tag. We just need to append the rest.
        let missing_part = &end_tag[partial_match_len..];
        status.tool_compat_buffer.push_str(missing_part);

        log::debug!(
            "Auto-completed tool tag. New buffer: {}",
            status.tool_compat_buffer
        );

        // Now that the tag is hopefully complete, process the buffer again.
        process_tool_calls_in_buffer(status, unified_chunks);
    } else {
        // If we are in a tool call block and there's no partial match of the end tag,
        // it means the entire end tag is missing.
        log::warn!(
            "Entire tool closing tag is missing at end of stream. Attempting auto-completion."
        );
        status.tool_compat_buffer.push_str(end_tag);

        log::debug!(
            "Auto-completed tool tag. New buffer: {}",
            status.tool_compat_buffer
        );

        // Now that the tag is hopefully complete, process the buffer again.
        process_tool_calls_in_buffer(status, unified_chunks);
    }
}

/// Parse tool XML and emit tool call chunks
fn parse_and_emit_tool_call(
    status: &mut std::sync::RwLockWriteGuard<SseStatus>,
    tool_xml: &str,
    unified_chunks: &mut Vec<UnifiedStreamChunk>,
) {
    if let Ok(parsed_tool) = ToolUse::try_from(tool_xml) {
        let tool_id = get_tool_id();
        if status.tool_id != "" {
            // send tool stop
            unified_chunks.push(UnifiedStreamChunk::ContentBlockStop {
                index: status.message_index,
            })
        }
        status.tool_id = tool_id.clone();
        update_message_block(status, tool_id.clone());

        let mut arguments = serde_json::Map::new();
        for param in parsed_tool.args {
            arguments.insert(param.name.clone(), param.get_value());
        }

        // Send tool call start for claude only
        unified_chunks.push(UnifiedStreamChunk::ContentBlockStart {
            index: status.message_index,
            block: json!({
                "type": "tool_use",
                "id": tool_id.clone(),
                "name": parsed_tool.name.clone(),
                "input": {}
            }),
        });
        // Send tool call start for gemini and openai
        unified_chunks.push(UnifiedStreamChunk::ToolUseStart {
            tool_type: "function".to_string(),
            id: tool_id.clone(),
            name: parsed_tool.name.clone(),
        });

        // Send tool call parameters
        let args_json = serde_json::to_string(&arguments).unwrap_or_default();
        unified_chunks.push(UnifiedStreamChunk::ToolUseDelta {
            id: tool_id,
            delta: args_json.clone(),
        });

        log::info!("tool parse success, name: {}", parsed_tool.name.clone());
    } else {
        let malformed_xml = tool_xml.to_string();
        log::warn!("tool use xml parse failed, xml: {}", malformed_xml);

        // 1. Send the malformed XML back as plain text so the AI can see what it did.
        unified_chunks.push(UnifiedStreamChunk::Text {
            delta: malformed_xml,
        });

        // 2. Send the corrective reminder.
        unified_chunks.push(UnifiedStreamChunk::Text {
            delta: TOOL_PARSE_ERROR_REMINDER.to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ccproxy::adapter::unified::SseStatus;
    use std::sync::{Arc, RwLock};

    struct TestResult {
        chunks: Vec<UnifiedStreamChunk>,
        final_buffer: String,
    }

    fn run_processor(input: &str) -> TestResult {
        let mut status = SseStatus::default();
        status.tool_compat_buffer = input.to_string();
        let mut chunks = Vec::new();

        // The function takes a RwLockWriteGuard, so we need to simulate that
        let status_arc = Arc::new(RwLock::new(status));
        let mut status_lock = status_arc.write().unwrap();

        process_tool_calls_in_buffer(&mut status_lock, &mut chunks);

        TestResult {
            chunks,
            final_buffer: status_lock.tool_compat_buffer.clone(),
        }
    }

    #[test]
    fn test_strips_todo_block() {
        let input = "start <cs:todo>\"thought\"</cs:todo> end";
        let result = run_processor(input);
        assert!(result
            .chunks
            .iter()
            .any(|c| matches!(c, UnifiedStreamChunk::Text{delta} if delta == "start ")));
        // The " end" part will be left in the buffer as there's no subsequent tag
        assert_eq!(result.final_buffer, " end");
    }

    #[test]
    fn test_parses_tool_use_after_todo() {
        let input = "<cs:todo>I should call a tool.</cs:todo><cs:tool_use><name>test_tool</name><args></args></cs:tool_use>";
        let result = run_processor(input);
        assert!(result.chunks.iter().any(
            |c| matches!(c, UnifiedStreamChunk::ToolUseStart{name, ..} if name == "test_tool")
        ));
        assert!(result.final_buffer.is_empty());
    }

    #[test]
    fn test_ignores_tool_use_inside_todo() {
        let input = "<cs:todo>do not run <cs:tool_use><name>fake_tool</name></cs:tool_use></cs:todo><cs:tool_use><name>real_tool</name><args></args></cs:tool_use>";
        let result = run_processor(input);
        assert!(!result.chunks.iter().any(
            |c| matches!(c, UnifiedStreamChunk::ToolUseStart{name, ..} if name == "fake_tool")
        ));
        assert!(result.chunks.iter().any(
            |c| matches!(c, UnifiedStreamChunk::ToolUseStart{name, ..} if name == "real_tool")
        ));
        assert!(result.final_buffer.is_empty());
    }

    #[test]
    fn test_handles_incomplete_todo_block() {
        let input = "text before <cs:todo>incomplete thought";
        let result = run_processor(input);
        assert!(result
            .chunks
            .iter()
            .any(|c| matches!(c, UnifiedStreamChunk::Text{delta} if delta == "text before ")));
        assert_eq!(result.final_buffer, "<cs:todo>incomplete thought");
    }

    #[test]
    fn test_handles_multiple_mixed_blocks() {
        let input = "text1<cs:todo>t1</cs:todo>text2<cs:tool_use><name>tool1</name><args></args></cs:tool_use>text3";
        let result = run_processor(input);

        let texts: Vec<String> = result
            .chunks
            .iter()
            .filter_map(|c| match c {
                UnifiedStreamChunk::Text { delta } => Some(delta.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["text1", "text2"]);

        let tools: Vec<String> = result
            .chunks
            .iter()
            .filter_map(|c| match c {
                UnifiedStreamChunk::ToolUseStart { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(tools, vec!["tool1"]);

        assert_eq!(result.final_buffer, "text3");
    }
}
