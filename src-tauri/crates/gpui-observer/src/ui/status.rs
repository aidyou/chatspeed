//! Live status above the prompt box.
//!
//! Mirrors `src/components/workflow/StatusNotifier.vue`: the line reports what
//! the runtime is doing *now* — the tool it is waiting on or running, the latest
//! streamed sentence, or the plain thinking state — instead of a static list of
//! states. The order of the checks is the same as Vue's `displayState`, because
//! a pending approval must win over streamed text and vice versa.

use std::time::Duration;

use gpui_kit::component::{h_flex, ActiveTheme};
use gpui_kit::*;
use rust_i18n::t;
use serde_json::Value;

use crate::presentation::{format_display_path, single_line, tool_arguments, tool_name};
use crate::ui::Icon;
use crate::{warning, Observer};

/// How long a finished tool stays in the status line, as in Vue.
const COMPLETED_VISIBLE: Duration = Duration::from_secs(5);

/// The longest text the line shows before it ellipsizes.
const PREVIEW_CHARS: usize = 72;

/// Tools that always wait for an approval before they run.
const APPROVAL_TOOLS: [&str; 3] = ["edit_file", "write_file", "bash"];

struct Live {
    text: String,
    icon: &'static str,
    warning: bool,
}

/// The runtime state line between the transcript and the box.
pub fn status(observer: &Observer, cx: &mut Context<Observer>) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let Some(live) = live_state(observer) else {
        return div().into_any_element();
    };
    let color = if live.warning {
        warning().into()
    } else {
        muted
    };
    let icon = Icon::new(live.icon).size(14.).color(color);
    h_flex()
        .w_full()
        .max_w(px(780.))
        .min_h(px(24.))
        .items_center()
        .gap_2()
        .px_1()
        .text_size(px(12.))
        .text_color(color)
        .child(icon)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(live.text),
        )
        .into_any_element()
}

fn live_state(observer: &Observer) -> Option<Live> {
    let model = &observer.model;
    let state = model.state.to_lowercase();
    let wait = model.wait_reason.to_lowercase();
    let running = observer.workflow_running();
    let waiting_user = wait == "user_input" || state == "awaiting_user";
    let waiting_approval =
        wait == "approval" || state == "awaiting_approval" || state == "awaiting_auto_approval";
    let terminal = matches!(
        state.as_str(),
        "completed" | "failed" | "cancelled" | "error" | "stopped"
    );
    // Vue shows nothing for a finished task, and nothing before the first run
    // either; the connecting note is the only non-runtime message it keeps.
    if terminal || (!running && !waiting_user && !waiting_approval) {
        return None;
    }

    if pending_plan(observer) {
        return Some(Live {
            text: t!("live_awaiting_plan_approval").to_string(),
            icon: "skill-plan",
            warning: true,
        });
    }

    if waiting_user {
        return Some(Live {
            text: t!("live_awaiting_user").to_string(),
            icon: "ask_user",
            warning: true,
        });
    }

    let latest = model
        .tool_order
        .last()
        .and_then(|id| model.tools.get(id).map(|tool| (id, tool)));

    if let Some((id, tool)) = latest {
        let phase = tool_phase(tool);
        if matches!(phase, "pending" | "running") {
            return Some(tool_status(observer, id, tool, phase));
        }
    }

    if waiting_approval {
        return Some(Live {
            text: t!("live_awaiting_approval").to_string(),
            icon: "warning",
            warning: true,
        });
    }

    if let Some(text) = latest_text(observer) {
        return Some(Live {
            text,
            icon: "reasoning",
            warning: false,
        });
    }

    // A tool that just finished stays on screen briefly, like Vue's 5s window.
    if let Some((id, tool)) = latest {
        let fresh = model
            .tool_finished
            .get(id)
            .is_some_and(|at| at.elapsed() < COMPLETED_VISIBLE);
        if fresh && tool_phase(tool) == "done" {
            return Some(tool_status(observer, id, tool, "done"));
        }
    }

    if running {
        return Some(Live {
            text: t!("live_thinking").to_string(),
            icon: "reasoning",
            warning: false,
        });
    }

    None
}

/// The runtime waits on a plan confirmation while `submit_plan` is pending.
fn pending_plan(observer: &Observer) -> bool {
    observer.model.pending_tool_ids.iter().any(|id| {
        observer
            .model
            .tools
            .get(id)
            .is_some_and(|tool| tool_name(tool) == "submit_plan")
    })
}

/// The tool text for one call, matching Vue's `buildToolState` branches.
fn tool_status(observer: &Observer, id: &str, tool: &Value, phase: &str) -> Live {
    let name = tool_name(tool).to_lowercase();
    let args = tool_arguments(tool);
    let path = args["file_path"]
        .as_str()
        .or_else(|| args["path"].as_str())
        .map(|path| format_display_path(path, &observer.model.roots))
        .unwrap_or_default();
    let command = args["command"]
        .as_str()
        .map(|command| {
            let normalized = single_line(command);
            truncate(&format!("Run {normalized}"), PREVIEW_CHARS)
        })
        .unwrap_or_default();

    let (text, icon) = match (phase, name.as_str()) {
        ("pending", "edit_file") if !path.is_empty() => (
            t!("live_awaiting_edit_approval", path = path).to_string(),
            "edit",
        ),
        ("pending", "write_file") if !path.is_empty() => (
            t!("live_awaiting_write_approval", path = path).to_string(),
            "write_file",
        ),
        ("pending", "bash") => (
            t!(
                "live_awaiting_bash_approval",
                command = command_or_label(&command, tool)
            )
            .to_string(),
            "bash",
        ),
        ("pending", _) if !APPROVAL_TOOLS.contains(&name.as_str()) => {
            (t!("live_awaiting_approval").to_string(), "warning")
        }
        ("running", "edit_file") if !path.is_empty() => {
            (t!("live_running_edit", path = path).to_string(), "edit")
        }
        ("running", "write_file") if !path.is_empty() => (
            t!("live_running_write", path = path).to_string(),
            "write_file",
        ),
        ("running", "bash") => (
            t!(
                "live_running_command",
                command = command_or_label(&command, tool)
            )
            .to_string(),
            "bash",
        ),
        ("running", _) => (
            t!("live_running_tool", tool = tool_label(tool)).to_string(),
            "tool",
        ),
        ("done", "edit_file") if !path.is_empty() => (
            t!("live_file_edited", path = path).to_string(),
            "check-circle",
        ),
        ("done", "write_file") if !path.is_empty() => (
            t!("live_file_created", path = path).to_string(),
            "check-circle",
        ),
        ("done", "bash") => (
            t!(
                "live_tool_completed",
                tool = command_or_label(&command, tool)
            )
            .to_string(),
            "check-circle",
        ),
        _ => (
            t!("live_tool_completed", tool = tool_label(tool)).to_string(),
            "check-circle",
        ),
    };

    let text = if phase == "running" {
        observer
            .model
            .tool_started
            .get(id)
            .map(|started| {
                t!(
                    "live_elapsed",
                    text = text,
                    seconds = started.elapsed().as_secs()
                )
                .to_string()
            })
            .unwrap_or(text)
    } else {
        text
    };

    Live {
        text,
        icon,
        warning: phase == "pending",
    }
}

/// `execution_status` and the live event type both describe the same lifecycle.
fn tool_phase(tool: &Value) -> &'static str {
    let event = tool["type"].as_str().unwrap_or("");
    match event {
        "tool_started" => return "running",
        "tool_completed" => return "done",
        "tool_failed" => return "failed",
        _ => {}
    }
    match tool["execution_status"]
        .as_str()
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "pending_approval" => "pending",
        "running" | "approval_submitted" => "running",
        "completed" => "done",
        "failed" | "rejected" => "failed",
        _ => "running",
    }
}

fn command_or_label(command: &str, tool: &Value) -> String {
    if command.is_empty() {
        tool_label(tool)
    } else {
        command.to_owned()
    }
}

fn tool_label(tool: &Value) -> String {
    let name = tool_name(tool);
    let args = tool_arguments(tool);
    let detail = args["description"]
        .as_str()
        .or_else(|| args["query"].as_str())
        .or_else(|| args["pattern"].as_str())
        .or_else(|| args["path"].as_str())
        .unwrap_or("");
    if detail.is_empty() {
        name.to_owned()
    } else {
        truncate(&format!("{name} {detail}"), PREVIEW_CHARS)
    }
}

/// The newest streamed sentence, which Vue prefers over the thinking fallback.
fn latest_text(observer: &Observer) -> Option<String> {
    let live = &observer.model.live;
    let content = preview_segment(&live.message);
    let reasoning = preview_segment(live.reasoning.as_deref().unwrap_or(""));
    let text = if content.is_empty() {
        reasoning
    } else {
        content
    };
    (!text.is_empty()).then_some(text)
}

/// Vue's `getPreviewSegment`: the last finished sentence, else the last segment.
fn preview_segment(text: &str) -> String {
    let cleaned = strip_reminders(text);
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        return String::new();
    }
    let latest = cleaned
        .rsplit(|c| matches!(c, '。' | '！' | '？' | '.' | '!' | '?'))
        .find(|segment| !segment.trim().is_empty())
        .unwrap_or(cleaned)
        .trim();
    truncate(latest, PREVIEW_CHARS)
}

/// Removes the reminder and thinking wrappers the streamed text still carries.
fn strip_reminders(text: &str) -> String {
    let mut cleaned = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<SYSTEM_REMINDER>") {
        cleaned.push_str(&rest[..start]);
        match rest[start..].find("</SYSTEM_REMINDER>") {
            Some(end) => rest = &rest[start + end + "</SYSTEM_REMINDER>".len()..],
            None => {
                rest = "";
                break;
            }
        }
    }
    cleaned.push_str(rest);
    let mut collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    for tag in ["<think>", "</think>", "<thinking>", "</thinking>"] {
        collapsed = collapsed.replace(tag, "");
    }
    collapsed.trim().to_owned()
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let head: String = text.chars().take(max.saturating_sub(3)).collect();
    format!("{head}...")
}

#[cfg(test)]
mod tests {
    use super::{preview_segment, tool_phase, truncate};

    #[test]
    fn preview_keeps_the_latest_sentence_and_drops_reminders() {
        let text = "<SYSTEM_REMINDER>noise</SYSTEM_REMINDER> First done. Second part.";
        assert_eq!(preview_segment(text), "Second part");
    }

    #[test]
    fn truncation_marks_what_it_cut() {
        assert_eq!(truncate("abcdef", 6), "abcdef");
        assert_eq!(truncate("abcdefgh", 6), "abc...");
        assert_eq!(truncate("缓存命中率很高", 5), "缓存...");
    }

    #[test]
    fn tool_phases_follow_the_live_event_first() {
        assert_eq!(
            tool_phase(&serde_json::json!({"type": "tool_completed"})),
            "done"
        );
        assert_eq!(
            tool_phase(&serde_json::json!({"execution_status": "pending_approval"})),
            "pending"
        );
        assert_eq!(
            tool_phase(&serde_json::json!({"execution_status": "completed"})),
            "done"
        );
    }
}
