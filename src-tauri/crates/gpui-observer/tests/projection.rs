//! End-to-end projection of a representative transcript through the public API.
//!
//! The fixture mirrors the camelCase wire shape the workflow command returns, so
//! this also guards the adapter that moves `messageKind`, `stepType` and the
//! completion observation out of `metadata`.

use chatspeed_gpui_observer::presentation::{project, Row, SubAgentStatus, ToolStatus};
use chatspeed_gpui_observer::types::Message;
use serde_json::json;

fn messages(value: serde_json::Value) -> Vec<Message> {
    serde_json::from_value(value).expect("fixture matches the command payload")
}

#[test]
fn session_transcript_projects_snapshot_group_and_sub_agent() {
    let rows = project(
        messages(json!([
            {
                "id": 1,
                "role": "system",
                "messageKind": "summary",
                "message": "## Previous Context Snapshot\n{\"overall_goal\":\"Ship the observer\"}"
            },
            { "id": 2, "role": "user", "message": "Run the suite" },
            { "id": 3, "role": "assistant", "message": "Checking the repo." },
            {
                "id": 4,
                "role": "tool",
                "metadata": {
                    "tool_name": "bash",
                    "tool_call_id": "call-1",
                    "tool_call": { "function": { "name": "bash", "arguments": "{\"command\":\"cargo test\"}" } },
                    "execution_status": "completed",
                    "duration_ms": 2000,
                    "summary": "12 passed"
                }
            },
            {
                "id": 5,
                "role": "tool",
                "metadata": {
                    "tool_name": "sub_agent_run",
                    "tool_call_id": "call-2",
                    "sub_agent_id": "subagent_7",
                    "sub_agent_name": "Code Explorer",
                    "sub_agent_task": "Scan the crate",
                    "execution_mode": "call",
                    "execution_status": "running"
                }
            },
            {
                "id": 6,
                "role": "user",
                "stepType": "observe",
                "message": "<tool_result tool=\"sub_agent_run\" id=\"subagent_7\" status=\"completed\">",
                "metadata": {
                    "data": {
                        "execution_status": "completed",
                        "result": {
                            "result": "Six files inspected",
                            "tool_calls_count": 3,
                            "current_context_tokens": 640,
                            "max_context_tokens": 1280
                        }
                    }
                }
            },
            {
                "id": 7,
                "role": "tool",
                "metadata": {
                    "tool_name": "complete_workflow",
                    "tool_call_id": "call-3",
                    "execution_status": "completed",
                    "summary": "Done"
                }
            }
        ])),
        &[],
    );

    assert_eq!(
        rows.len(),
        6,
        "snapshot, user, assistant, group, card, finish"
    );
    assert!(matches!(&rows[0], Row::Snapshot(view) if view.body.contains("Ship the observer")));
    assert!(matches!(&rows[1], Row::User(view) if view.text == "Run the suite"));
    assert!(
        matches!(&rows[2], Row::Assistant(view) if view.markdown.contains("Checking the repo."))
    );

    let Row::Activity(group) = &rows[3] else {
        panic!("expected the tool call to collapse into an activity group");
    };
    let tool = group.items[0].tool.as_ref().expect("tool item");
    assert_eq!(tool.action, "Run cargo test");
    assert_eq!(tool.status, ToolStatus::Completed);
    assert_eq!(tool.summary, "12 passed");
    assert_eq!(tool.badges.last().map(String::as_str), Some("2s"));

    let Row::SubAgent(card) = &rows[4] else {
        panic!("expected the completion observation to enrich the card");
    };
    assert_eq!(card.status, SubAgentStatus::Completed);
    assert_eq!(card.agent, "Code Explorer");
    assert_eq!(card.tools, "3 tools");
    assert_eq!(card.context, "50% ctx");
    assert!(card.result.contains("Six files inspected"));

    assert!(
        matches!(&rows[5], Row::FinishTask(task) if !task.failed && task.label == "Task finished"),
        "completion reports render the localized finish label, not the tool summary"
    );
}
